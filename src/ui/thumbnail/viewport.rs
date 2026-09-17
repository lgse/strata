// SPDX-License-Identifier: MIT

use std::rc::{Rc, Weak};

use super::*;
use crate::{app::Browser, model::Location};

thread_local! {
    static REFRESH_PENDING: Cell<bool> = const { Cell::new(false) };
    static HOOKS: RefCell<Vec<glib::WeakRef<gtk::ScrolledWindow>>> = const { RefCell::new(Vec::new()) };
    static METADATA: RefCell<HashMap<usize, MetadataTarget>> = RefCell::new(HashMap::new());
}

struct MetadataTarget {
    item: glib::WeakRef<gtk::Widget>,
    browser: Weak<Browser>,
    depth: usize,
    position: usize,
    location: Location,
    details: bool,
}

pub(in crate::ui) fn request_metadata(
    image: &ThumbnailSlot,
    item: &impl IsA<gtk::Widget>,
    browser: &Rc<Browser>,
    depth: usize,
    position: usize,
    location: Location,
    details: bool,
) {
    METADATA.with(|pending| {
        pending.borrow_mut().insert(
            image.as_ptr() as usize,
            MetadataTarget {
                item: item.as_ref().downgrade(),
                browser: Rc::downgrade(browser),
                depth,
                position,
                location,
                details,
            },
        );
    });
    schedule_refresh();
}

pub(super) fn cancel_metadata(image_id: usize) {
    METADATA.with(|pending| {
        pending.borrow_mut().remove(&image_id);
    });
}

pub(super) fn schedule_refresh() {
    if REFRESH_PENDING.with(|pending| pending.replace(true)) {
        return;
    }
    glib::idle_add_local_once(|| {
        REFRESH_PENDING.with(|pending| pending.set(false));
        let ready = METADATA.with(|pending| {
            let mut pending = pending.borrow_mut();
            let mut ready = Vec::new();
            pending.retain(|_, target| {
                let (Some(item), Some(browser)) = (target.item.upgrade(), target.browser.upgrade())
                else {
                    return false;
                };
                hook_ancestors(&item);
                if visibility(&item).0 >= 2 {
                    return true;
                }
                if browser
                    .entry_at(target.depth, target.position)
                    .is_some_and(|entry| entry.location == target.location)
                {
                    ready.push((
                        browser,
                        target.depth,
                        target.position,
                        target.location.clone(),
                        target.details,
                    ));
                }
                false
            });
            ready
        });
        for (browser, depth, position, location, details) in ready {
            browser.request_metadata_fill(depth, position, location, details);
        }
        demote_offscreen();
        retry_deferred_thumbnails();
        start_thumbnail_jobs();
        start_render_jobs();
    });
}

pub(super) fn hook_ancestors(widget: &impl IsA<gtk::Widget>) {
    let mut ancestor = Some(widget.clone().upcast::<gtk::Widget>());
    while let Some(widget) = ancestor {
        ancestor = widget.parent();
        let Ok(viewport) = widget.downcast::<gtk::ScrolledWindow>() else {
            continue;
        };
        let hooked = HOOKS.with(|hooks| {
            let mut hooks = hooks.borrow_mut();
            hooks.retain(|hook| hook.upgrade().is_some());
            if hooks
                .iter()
                .any(|hook| hook.upgrade().as_ref() == Some(&viewport))
            {
                return true;
            }
            hooks.push(viewport.downgrade());
            false
        });
        if !hooked {
            for adjustment in [viewport.vadjustment(), viewport.hadjustment()] {
                adjustment.connect_value_changed(|_| schedule_refresh());
                adjustment.connect_changed(|_| schedule_refresh());
            }
            viewport.connect_map(|_| schedule_refresh());
        }
    }
}

type Priority = (u8, i32, i32, u64);

pub(super) fn priority(target: &PendingTarget) -> Priority {
    let (rank, y, x) = target
        .image
        .upgrade()
        .map_or((3, 0, 0), |image| visibility(&image));
    (rank, y, x, target.request)
}

fn visibility(image: &impl IsA<gtk::Widget>) -> (u8, i32, i32) {
    let mut ancestor = image.parent();
    let mut result = (0, 0, 0);
    let mut first = true;
    while let Some(widget) = ancestor {
        ancestor = widget.parent();
        let Ok(viewport) = widget.downcast::<gtk::ScrolledWindow>() else {
            continue;
        };
        let Some(bounds) = image.compute_bounds(&viewport) else {
            return (3, 0, 0);
        };
        if !image.is_mapped()
            || bounds.width() <= 0.0
            || bounds.height() <= 0.0
            || viewport.width() <= 0
            || viewport.height() <= 0
        {
            return (3, 0, 0);
        }
        if first {
            result.1 = bounds.y() as i32;
            result.2 = bounds.x() as i32;
            first = false;
        }
        let intersects = |margin: f32| {
            bounds.x() < viewport.width() as f32 * (1.0 + margin)
                && bounds.x() + bounds.width() > -(viewport.width() as f32) * margin
                && bounds.y() < viewport.height() as f32 * (1.0 + margin)
                && bounds.y() + bounds.height() > -(viewport.height() as f32) * margin
        };
        result.0 = result.0.max(if intersects(0.0) {
            0
        } else if intersects(0.25) {
            1
        } else {
            2
        });
    }
    result
}

pub(super) fn key_priority(key: &ThumbnailKey) -> Priority {
    PENDING_THUMBNAILS.with(|pending| {
        pending
            .borrow()
            .get(key)
            .and_then(|job| {
                job.targets
                    .iter()
                    .filter(|target| request_is_live(target))
                    .map(priority)
                    .min()
            })
            .unwrap_or((3, 0, 0, 0))
    })
}

fn demote_offscreen() {
    let mut queued =
        THUMBNAIL_QUEUE.with(|queue| queue.borrow().queued.iter().cloned().collect::<Vec<_>>());
    RENDER_QUEUE.with(|queue| queued.extend(queue.borrow().iter().map(|job| job.key.clone())));
    for key in queued {
        if key_priority(&key).0 < 2 {
            continue;
        }
        let pending = PENDING_THUMBNAILS.with(|pending| pending.borrow_mut().remove(&key));
        if let Some(pending) = pending {
            pending.cancellation.cancel();
            for target in pending.targets {
                mark_deferred(key.clone(), pending.kind, target.image_id, target.request);
            }
        }
        THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().cancel(&key));
    }
    RENDER_QUEUE.with(|queue| {
        queue
            .borrow_mut()
            .retain(|job| !job.cancellation.is_cancelled())
    });
}

pub(super) fn prioritize_queue(queue: &mut VecDeque<ThumbnailKey>) {
    queue.make_contiguous().sort_by_cached_key(key_priority);
}

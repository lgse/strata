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

pub(super) fn publish_thumbnail_metadata(
    image_id: usize,
    path: &Path,
    metadata: &crate::sandbox::metadata::MediaMetadata,
) {
    let target = METADATA.with(|pending| {
        let pending = pending.borrow();
        let target = pending.get(&image_id)?;
        if target.location.native_path() != Some(path) {
            return None;
        }
        Some((
            target.browser.upgrade()?,
            target.depth,
            target.position,
            target.location.clone(),
        ))
    });
    let Some((browser, depth, position, location)) = target else {
        return;
    };
    browser.apply_thumbnail_metadata(
        depth,
        position,
        crate::services::MetadataUpdate {
            location,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            image_dimensions: metadata
                .dimensions
                .map_or(MetadataValue::Unknown, MetadataValue::Known),
            child_count: MetadataValue::Unknown,
            duration_seconds: metadata.duration.map_or(MetadataValue::Unknown, |seconds| {
                MetadataValue::Known(seconds.round() as u64)
            }),
        },
    );
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
                let Some(entry) = browser
                    .entry_at(target.depth, target.position)
                    .filter(|entry| entry.location == target.location)
                else {
                    return false;
                };
                if !crate::ui::browser::metadata_needs_fill(&entry)
                    && entry.mode != MetadataValue::Unknown
                    && !(target.details && crate::ui::browser_modes::icon_details_need_fill(&entry))
                {
                    return false;
                }
                hook_ancestors(&item);
                ready.push((
                    visibility(&item),
                    browser,
                    target.depth,
                    target.position,
                    target.location.clone(),
                    target.details,
                ));
                true
            });
            ready
        });
        let mut ready = ready;
        ready.sort_by_key(|(priority, ..)| *priority);
        let mut viewports = HashMap::new();
        for (priority, browser, depth, position, location, details) in ready {
            let (_, visible) = viewports
                .entry((Rc::as_ptr(&browser) as usize, depth))
                .or_insert_with(|| (browser.clone(), Vec::new()));
            if priority.0 < 2 {
                visible.push(location.clone());
                browser.request_metadata_fill(depth, position, location, details);
            }
        }
        for ((_, depth), (browser, visible)) in viewports {
            browser.prioritize_metadata_fills(depth, &visible);
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
            let pending = Rc::new(RefCell::new(None::<crate::ui::frame::FrameTask>));
            let weak_viewport = viewport.downgrade();
            let refresh = Rc::new(move || {
                schedule_refresh();
                if pending.borrow().is_some() {
                    return;
                }
                let Some(viewport) = weak_viewport.upgrade().filter(|view| view.is_mapped()) else {
                    return;
                };
                let pending_for_frame = pending.clone();
                // Adjustment signals can precede allocation. Retry with the final bounds,
                // even if all work was deferred and no worker or input can wake it again.
                pending.replace(Some(crate::ui::frame::FrameTask::new(
                    Some(viewport.upcast_ref()),
                    move || {
                        pending_for_frame.borrow_mut().take();
                        schedule_refresh();
                    },
                )));
            });
            for adjustment in [viewport.vadjustment(), viewport.hadjustment()] {
                let refresh_value = refresh.clone();
                adjustment.connect_value_changed(move |_| refresh_value());
                let refresh_changed = refresh.clone();
                adjustment.connect_changed(move |_| refresh_changed());
            }
            let refresh_map = refresh.clone();
            viewport.connect_map(move |_| refresh_map());
            refresh();
        }
    }
}

pub(in crate::ui) fn near_viewport(widget: &impl IsA<gtk::Widget>) -> bool {
    visibility(widget).0 < 2
}

#[cfg(test)]
mod scroll_tests;
#[cfg(test)]
mod tests;

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

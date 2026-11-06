// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    model::{FileEntry, MetadataValue},
    sandbox::Cancellation,
};

mod background;
mod cache;
mod camera;
mod icons;
mod jobs;
mod kind;
mod persist;
mod slot;
#[cfg(test)]
mod tests;
mod viewport;
#[cfg(test)]
pub(super) use cache::remember_thumbnail_for_test;
use cache::{CacheHit, THUMBNAIL_CACHE};
pub(super) use cache::{cached_thumbnail, preserve_renamed_thumbnail};
use icons::{TRACKED_CUSTOMIZED_ICONS, set_fallback_icon};
pub(super) use icons::{
    refresh_all_customized_icons, refresh_customized_icons, refresh_customized_icons_within,
    refresh_slot_icon, show_customized_folder_image, show_customized_icon,
    show_customized_icon_image, show_fallback_icon, show_locked_folder,
};
pub(super) use jobs::set_worker_limit;
#[cfg(test)]
use jobs::{MODEL_RUNNING, ThumbnailJob, finish_thumbnail_job, next_render_job};
use jobs::{RENDER_QUEUE, start_render_jobs, start_thumbnail_jobs};
use kind::{ThumbnailKind, heavy, render_thumbnail, thumbnail_kind};
use persist::enqueue_persist;
pub(crate) use slot::ThumbnailSlot;
#[cfg(test)]
use std::time::Instant;
pub(super) use viewport::{near_viewport, request_metadata, schedule_refresh as refresh_metadata};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

const MAX_CACHE_READERS: usize = 4;
const MAX_QUEUED_THUMBNAILS: usize = 64;

thread_local! {
    static ACTIVE_REQUESTS: RefCell<HashMap<usize, ActiveRequest>> =
        RefCell::new(HashMap::new());
    static PENDING_THUMBNAILS: RefCell<HashMap<ThumbnailKey, PendingThumbnail>> =
        RefCell::new(HashMap::new());
    static THUMBNAIL_QUEUE: RefCell<ThumbnailQueue> = RefCell::new(ThumbnailQueue::default());
    /// Per-viewport admission batches; one view's fling never postpones another's.
    static SETTLE_VIEWS: RefCell<HashMap<usize, ViewSettle>> = RefCell::new(HashMap::new());
    static TRACKED_THUMBNAILS: RefCell<HashMap<usize, TrackedThumbnail>> = RefCell::new(HashMap::new());
}

struct TrackedThumbnail {
    path: PathBuf,
}

struct ActiveRequest {
    id: u64,
    key: ThumbnailKey,
    image: glib::WeakRef<ThumbnailSlot>,
    deferred: Option<DeferredThumbnail>,
}

#[derive(Clone)]
struct DeferredThumbnail {
    key: ThumbnailKey,
    kind: ThumbnailKind,
}

#[derive(Clone)]
struct PendingTarget {
    image_id: usize,
    request: u64,
    image: glib::WeakRef<ThumbnailSlot>,
}
struct SettledPark {
    key: ThumbnailKey,
    kind: ThumbnailKind,
    target: PendingTarget,
}

struct ViewSettle {
    viewport: glib::WeakRef<gtk::ScrolledWindow>,
    pending: Vec<SettledPark>,
    timer: Option<super::frame::FrameTask>,
    hooked: bool,
}

struct PendingThumbnail {
    id: u64,
    kind: ThumbnailKind,
    cancellation: Cancellation,
    targets: Vec<PendingTarget>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ThumbnailKey {
    path: PathBuf,
    modified: Option<i64>,
    file_size: Option<u64>,
    thumbnail_size: i32,
}

#[derive(Default)]
struct ThumbnailQueue {
    running: usize,
    queued: VecDeque<ThumbnailKey>,
}

impl ThumbnailQueue {
    fn enqueue(&mut self, key: ThumbnailKey) -> bool {
        if self.queued.len() >= MAX_QUEUED_THUMBNAILS {
            return false;
        }
        self.queued.push_back(key);
        true
    }

    fn begin_next(&mut self) -> Option<ThumbnailKey> {
        if self.running >= MAX_CACHE_READERS {
            return None;
        }
        let key = self.queued.pop_front()?;
        self.running += 1;
        Some(key)
    }

    fn finish(&mut self) {
        self.running = self.running.saturating_sub(1);
    }

    fn cancel(&mut self, key: &ThumbnailKey) {
        self.queued.retain(|queued| queued != key);
    }
}

pub(super) fn set_thumbnail_or_icon(
    image: &ThumbnailSlot,
    entry: &FileEntry,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    super::file_providers::bind(image, entry.location.native_path());
    // A thumbnail of a root-only file would persist in the user's cache.
    if entry.location.is_administrator() {
        show_fallback_icon(image, fallback_icon, icon_size);
        return;
    }
    let request = |path: &Path, kind: Option<ThumbnailKind>| {
        set_thumbnail_for_path(ThumbnailRequest {
            image,
            path,
            kind,
            modified: known_metadata(&entry.modified_unix_seconds),
            file_size: known_metadata(&entry.size),
            fallback_icon,
            icon_size,
            thumbnail_size,
        });
    };
    if let Some(path) = entry.local_thumbnail_path() {
        let kind = if entry.is_directory() {
            None
        } else {
            thumbnail_kind(Path::new(&entry.display_name))
        };
        request(path, kind);
    } else if is_camera_file(entry) {
        request(
            Path::new(entry.location.uri_value().unwrap_or_default()),
            Some(ThumbnailKind::Camera),
        );
    } else if let Some((mirror_path, kind)) = remote_mirror_thumbnail(entry) {
        request(&mirror_path, Some(kind));
    } else {
        show_fallback_icon(image, fallback_icon, icon_size);
    }
}

/// A file on a gphoto2 camera that the camera preview can render.
fn is_camera_file(entry: &FileEntry) -> bool {
    let camera_file = entry.location.backend_name() == "gphoto2" && !entry.is_directory();
    camera_file
        && thumbnail_kind(Path::new(&entry.native_name))
            .is_some_and(|kind| kind != ThumbnailKind::RenderedModel)
}

// GVfs FUSE paths are render inputs only; navigation must retain the URI identity.
fn remote_mirror_thumbnail(entry: &FileEntry) -> Option<(PathBuf, ThumbnailKind)> {
    if entry.is_directory() {
        return None;
    }
    let kind = thumbnail_kind(Path::new(&entry.display_name))?;
    if kind == ThumbnailKind::RenderedModel {
        return None;
    }
    let uri = entry.location.uri_value()?;
    let path = gio::File::for_uri(uri).path()?;
    Some((path, kind))
}

pub(super) fn set_thumbnail_or_icon_for_path(
    image: &ThumbnailSlot,
    path: &Path,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    super::file_providers::bind(image, Some(path));
    set_thumbnail_for_path(ThumbnailRequest {
        image,
        path,
        kind: thumbnail_kind(path),
        modified: None,
        file_size: None,
        fallback_icon,
        icon_size,
        thumbnail_size,
    });
}

/// Bundled to stay under the argument-count lint.
struct ThumbnailRequest<'a> {
    image: &'a ThumbnailSlot,
    path: &'a Path,
    kind: Option<ThumbnailKind>,
    modified: Option<i64>,
    file_size: Option<u64>,
    fallback_icon: &'a str,
    icon_size: i32,
    thumbnail_size: i32,
}

#[derive(Clone)]
pub(super) struct ThumbnailSource {
    path: PathBuf,
    kind: Option<ThumbnailKind>,
    modified: Option<i64>,
    file_size: Option<u64>,
    fallback_icon: String,
    icon_size: i32,
    thumbnail_size: i32,
}

fn set_thumbnail_for_path(request: ThumbnailRequest<'_>) {
    viewport::cancel_metadata(request.image.as_ptr() as usize);
    request
        .image
        .set_thumbnail_source(request.kind.map(|_| ThumbnailSource {
            path: request.path.to_path_buf(),
            kind: request.kind,
            modified: request.modified,
            file_size: request.file_size,
            fallback_icon: request.fallback_icon.to_owned(),
            icon_size: request.icon_size,
            thumbnail_size: request.thumbnail_size,
        }));
    display_thumbnail_for_path(request);
}

fn refresh_slot_thumbnail(image: &ThumbnailSlot) {
    let Some(source) = image.thumbnail_source() else {
        return;
    };
    display_thumbnail_for_path(ThumbnailRequest {
        image,
        path: &source.path,
        kind: source.kind,
        modified: source.modified,
        file_size: source.file_size,
        fallback_icon: &source.fallback_icon,
        icon_size: source.icon_size,
        thumbnail_size: source.thumbnail_size,
    });
}

fn display_thumbnail_for_path(request: ThumbnailRequest<'_>) {
    let customization_path = (request.kind != Some(ThumbnailKind::Camera)).then_some(request.path);
    let show_icon = || {
        set_fallback_icon(
            request.image,
            customization_path,
            request.fallback_icon,
            request.icon_size,
        )
    };
    if shows_icon_instead(request.kind, customization_path) {
        show_icon();
        return;
    }
    let path = request.path.to_path_buf();
    let thumbnail_size = request.thumbnail_size.clamp(16, 256);
    let Some(kind) = request.kind else {
        show_icon();
        return;
    };
    if displayed_thumbnail_matches(request.image, request.path) {
        return;
    }
    let key = ThumbnailKey {
        path: path.clone(),
        modified: request.modified,
        file_size: request.file_size,
        thumbnail_size: super::thumbnail_cache::CANONICAL_MAX_EDGE,
    };
    match THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&key)) {
        Some(CacheHit::Ready(texture)) => {
            let (image_id, _) = prepare_thumbnail_target(request.image, thumbnail_size);
            apply_thumbnail(request.image, &texture, &path);
            ACTIVE_REQUESTS.with(|requests| {
                requests.borrow_mut().remove(&image_id);
            });
            return;
        }
        Some(CacheHit::Failed) => {
            show_icon();
            return;
        }
        None => {}
    }
    if already_requested(request.image, &key) {
        request.image.set_slot(thumbnail_size);
        return;
    }
    let (image_id, request_id) = show_icon();
    request.image.set_slot(thumbnail_size);
    let target = register_active_request(request.image, image_id, request_id, key.clone());
    // Walking ancestors or hooking the viewport during bind can corrupt layout.
    glib::idle_add_local_once(move || {
        if request_is_live(&target) {
            park_thumbnail(key, kind, target);
        }
    });
}

/// A custom icon, thumbnails turned off, or a model thumbnail turned off keeps the icon.
fn shows_icon_instead(kind: Option<ThumbnailKind>, customization_path: Option<&Path>) -> bool {
    let has_custom_icon = customization_path.is_some_and(|path| {
        super::preferences::PreferenceManager::shared()
            .custom_icon(path)
            .is_some()
    });
    let preferences = super::preferences::PreferenceManager::shared();
    let model_hidden = kind == Some(ThumbnailKind::RenderedModel) && !preferences.stl_thumbnails();
    has_custom_icon || !preferences.show_thumbnails() || model_hidden
}

/// Whether `image` already waits for `key`.
fn already_requested(image: &ThumbnailSlot, key: &ThumbnailKey) -> bool {
    ACTIVE_REQUESTS.with(|requests| {
        requests
            .borrow()
            .get(&(image.as_ptr() as usize))
            .is_some_and(|active| {
                active.key == *key && active.image.upgrade().as_ref() == Some(image)
            })
    })
}

fn viewport_of(image: &impl IsA<gtk::Widget>) -> Option<gtk::ScrolledWindow> {
    let mut ancestor = image.parent();
    while let Some(widget) = ancestor {
        ancestor = widget.parent();
        if let Ok(viewport) = widget.downcast::<gtk::ScrolledWindow>() {
            return Some(viewport);
        }
    }
    None
}

fn group_address(viewport: Option<&gtk::ScrolledWindow>) -> usize {
    viewport.map_or(0, |viewport| viewport.as_ptr() as usize)
}

fn park_thumbnail(key: ThumbnailKey, kind: ThumbnailKind, target: PendingTarget) {
    crate::metrics::mark_thumbnail_requested();
    let viewport = target.image.upgrade().and_then(|image| viewport_of(&image));
    let group = group_address(viewport.as_ref());
    let viewport_ref = glib::WeakRef::new();
    if let Some(viewport) = &viewport {
        viewport_ref.set(Some(viewport));
    }
    SETTLE_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let settle = views.entry(group).or_insert_with(|| ViewSettle {
            viewport: viewport_ref.clone(),
            pending: Vec::new(),
            timer: None,
            hooked: false,
        });
        // A dead viewport's address may be recycled: reset the group instead of joining its stale hooks and pending requests.
        if group != 0 && settle.viewport.upgrade().is_none() {
            settle.timer.take();
            *settle = ViewSettle {
                viewport: viewport_ref.clone(),
                pending: Vec::new(),
                timer: None,
                hooked: false,
            };
        }
        settle.pending.push(SettledPark { key, kind, target });
    });
    if let Some(viewport) = viewport {
        hook_viewport(group, &viewport);
    }
    if kind == ThumbnailKind::Camera {
        request_group_fire(group);
    } else {
        fire_view_group(group);
    }
}

fn mark_deferred(key: ThumbnailKey, kind: ThumbnailKind, image_id: usize, request: u64) {
    ACTIVE_REQUESTS.with(|requests| {
        if let Some(active) = requests
            .borrow_mut()
            .get_mut(&image_id)
            .filter(|active| active.id == request)
        {
            active.deferred = Some(DeferredThumbnail { key, kind });
        }
    });
}

/// Fires happen only on the main loop: firing inside binds or adjustment callbacks can walk the
/// widget tree while GTK is mutating it.
fn request_group_fire(group: usize) {
    SETTLE_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let Some(settle) = views.get_mut(&group) else {
            return;
        };
        // No pending rows, no timer: re-arming into an empty queue turns every thumbnail application (which relayouts) into another fire.
        if settle.pending.is_empty() {
            return;
        }
        // Collect one frame's camera binds without postponing work on a fling.
        if settle.timer.is_some() {
            return;
        }
        let viewport = settle.viewport.upgrade();
        settle.timer = Some(super::frame::FrameTask::new(
            viewport.as_ref().map(|view| view.upcast_ref()),
            move || {
                fire_view_group(group);
            },
        ));
    });
}
fn hook_viewport(group: usize, viewport: &gtk::ScrolledWindow) {
    let hooked = SETTLE_VIEWS.with(|views| {
        views
            .borrow_mut()
            .get_mut(&group)
            .map(|settle| std::mem::replace(&mut settle.hooked, true))
            .unwrap_or(true)
    });
    if hooked {
        return;
    }
    for adjustment in [viewport.vadjustment(), viewport.hadjustment()] {
        adjustment.connect_value_changed(move |_| {
            request_group_fire(group);
            self::viewport::schedule_refresh();
        });
        adjustment.connect_changed(move |_| {
            request_group_fire(group);
            self::viewport::schedule_refresh();
        });
    }
    self::viewport::hook_ancestors(viewport);
}

fn fire_view_group(group: usize) {
    let drained = SETTLE_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let Some(settle) = views.get_mut(&group) else {
            return Vec::new();
        };
        settle.timer.take();
        if group != 0 && settle.viewport.upgrade().is_none() {
            views.remove(&group);
            return Vec::new();
        }
        std::mem::take(&mut settle.pending)
    });
    fire_parks(drained);
}

fn request_is_live(target: &PendingTarget) -> bool {
    ACTIVE_REQUESTS.with(|requests| {
        requests
            .borrow()
            .get(&target.image_id)
            .is_some_and(|active| active.id == target.request)
    })
}

fn register_active_request(
    image: &ThumbnailSlot,
    image_id: usize,
    request_id: u64,
    key: ThumbnailKey,
) -> PendingTarget {
    let weak_image = glib::WeakRef::new();
    weak_image.set(Some(image));
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(
            image_id,
            ActiveRequest {
                id: request_id,
                key,
                image: weak_image.clone(),
                deferred: None,
            },
        );
    });
    PendingTarget {
        image_id,
        request: request_id,
        image: weak_image,
    }
}

fn apply_live_thumbnail(target: PendingTarget, texture: gdk::Texture, path: PathBuf) {
    if !request_is_live(&target) {
        crate::metrics::mark_thumbnail_stale();
        return;
    }
    let Some(image) = target.image.upgrade() else {
        crate::metrics::mark_thumbnail_stale();
        ACTIVE_REQUESTS.with(|requests| {
            requests.borrow_mut().remove(&target.image_id);
        });
        return;
    };
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().remove(&target.image_id);
    });
    apply_thumbnail(&image, &texture, &path);
    crate::metrics::mark_thumbnail_applied();
}

fn fire_parks(mut drained: Vec<SettledPark>) {
    drained.sort_by_cached_key(|park| viewport::priority(&park.target));
    let mut eligible = 0;
    let mut started = false;
    for park in drained {
        if !request_is_live(&park.target) {
            continue;
        }
        if viewport::priority(&park.target).0 >= 2 {
            mark_deferred(
                park.key,
                park.kind,
                park.target.image_id,
                park.target.request,
            );
            continue;
        }
        eligible += 1;
        started |= fire_park(park);
    }
    if started {
        start_thumbnail_jobs();
    }
    crate::metrics::mark_thumbnail_eligible(eligible);
}

/// Applies a cached result or schedules a job, reporting whether a job was scheduled.
fn fire_park(park: SettledPark) -> bool {
    if let Some(hit) = THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&park.key)) {
        if let CacheHit::Ready(texture) = hit {
            apply_live_thumbnail(park.target, texture, park.key.path);
        }
        return false;
    }
    let image_id = park.target.image_id;
    let request = park.target.request;
    if schedule_thumbnail(park.key.clone(), park.kind, park.target) {
        return true;
    }
    mark_deferred(park.key, park.kind, image_id, request);
    false
}

fn schedule_thumbnail(key: ThumbnailKey, kind: ThumbnailKind, target: PendingTarget) -> bool {
    PENDING_THUMBNAILS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if let Some(pending) = pending.get_mut(&key) {
            pending.targets.push(target);
            true
        } else {
            if pending.len() >= MAX_QUEUED_THUMBNAILS + crate::sandbox::browser::worker_limit() {
                return false;
            }
            let queued = THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().enqueue(key.clone()));
            if queued {
                pending.insert(
                    key.clone(),
                    PendingThumbnail {
                        id: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
                        kind,
                        cancellation: Cancellation::default(),
                        targets: vec![target],
                    },
                );
            }
            queued
        }
    })
}

fn retry_deferred_thumbnails() {
    let mut promoted = false;
    loop {
        // ponytail: deferred work is bounded by live GTK image widgets; add an explicit cap if a
        // future non-virtualized producer can create an unbounded number of them.
        let deferred = ACTIVE_REQUESTS.with(|requests| {
            let mut requests = requests.borrow_mut();
            requests.retain(|_, active| active.image.upgrade().is_some());
            requests
                .iter()
                .filter_map(|(image_id, active)| {
                    active.deferred.as_ref().map(|deferred| {
                        (*image_id, active.id, active.image.clone(), deferred.clone())
                    })
                })
                .min_by_key(|(image_id, request, image, _)| {
                    viewport::priority(&PendingTarget {
                        image_id: *image_id,
                        request: *request,
                        image: image.clone(),
                    })
                })
        });
        let Some((image_id, request, image, deferred)) = deferred else {
            break;
        };
        if viewport::priority(&PendingTarget {
            image_id,
            request,
            image: image.clone(),
        })
        .0 >= 2
        {
            break;
        }
        if !retry_deferred_thumbnail(image_id, request, image, deferred) {
            break;
        }
        promoted = true;
    }
    if promoted {
        start_thumbnail_jobs();
    }
}

fn retry_deferred_thumbnail(
    image_id: usize,
    request: u64,
    image: glib::WeakRef<ThumbnailSlot>,
    deferred: DeferredThumbnail,
) -> bool {
    if !schedule_thumbnail(
        deferred.key,
        deferred.kind,
        PendingTarget {
            image_id,
            request,
            image,
        },
    ) {
        return false;
    }
    ACTIVE_REQUESTS.with(|requests| {
        if let Some(active) = requests
            .borrow_mut()
            .get_mut(&image_id)
            .filter(|active| active.id == request)
        {
            active.deferred = None;
        }
    });
    true
}

fn take_pending_targets(key: &ThumbnailKey, job_id: u64) -> Option<Vec<PendingTarget>> {
    PENDING_THUMBNAILS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.get(key).is_some_and(|pending| pending.id == job_id) {
            pending.remove(key).map(|pending| pending.targets)
        } else {
            None
        }
    })
}

fn finish_thumbnail_targets(
    targets: Vec<PendingTarget>,
    texture: Option<&gdk::Texture>,
    path: &Path,
) {
    for target in targets {
        if !request_is_live(&target) {
            crate::metrics::mark_thumbnail_stale();
            continue;
        }
        let Some(texture) = texture else {
            ACTIVE_REQUESTS.with(|requests| {
                requests.borrow_mut().remove(&target.image_id);
            });
            continue;
        };
        apply_live_thumbnail(target, texture.clone(), path.to_path_buf());
    }
}

fn known_metadata<T: Copy>(value: &MetadataValue<T>) -> Option<T> {
    match value {
        MetadataValue::Known(value) => Some(*value),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

fn apply_thumbnail(image: &ThumbnailSlot, texture: &gdk::Texture, path: &Path) {
    image.set_texture(texture);
    register_displayed_thumbnail(image, path);
}

fn displayed_thumbnail_matches(image: &ThumbnailSlot, path: &Path) -> bool {
    TRACKED_THUMBNAILS.with_borrow(|thumbnails| {
        thumbnails
            .get(&(image.as_ptr() as usize))
            .is_some_and(|tracked| tracked.path == path)
    })
}

fn register_displayed_thumbnail(image: &ThumbnailSlot, path: &Path) {
    TRACKED_THUMBNAILS.with_borrow_mut(|thumbnails| {
        thumbnails.insert(
            image.as_ptr() as usize,
            TrackedThumbnail {
                path: path.to_path_buf(),
            },
        );
    });
}

fn clear_displayed_thumbnail(image: &ThumbnailSlot) {
    TRACKED_THUMBNAILS.with_borrow_mut(|thumbnails| {
        thumbnails.remove(&(image.as_ptr() as usize));
    });
}

fn forget_slot(image_id: usize) {
    // Dispose removes pointer keys before GTK can reuse the slot's address.
    let _ = TRACKED_THUMBNAILS.try_with(|thumbnails| {
        thumbnails.borrow_mut().remove(&image_id);
    });
    let _ = TRACKED_CUSTOMIZED_ICONS.try_with(|icons| {
        icons.borrow_mut().remove(&image_id);
    });
}

pub(super) fn cancel_list_item_thumbnails(item: &glib::Object) {
    let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
        return;
    };
    if let Some(child) = item.child() {
        cancel_thumbnails_in(&child);
    }
}

pub(super) fn cancel_thumbnails_in(widget: &gtk::Widget) {
    if let Some(image) = widget.downcast_ref::<ThumbnailSlot>() {
        image.set_thumbnail_source(None);
        cancel_thumbnail(image.as_ptr() as usize);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        cancel_thumbnails_in(&current);
    }
}

fn prepare_thumbnail_target(image: &ThumbnailSlot, size: i32) -> (usize, u64) {
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let image_id = image.as_ptr() as usize;
    // Refreshing artwork must not discard the bound row's pending metadata.
    cancel_thumbnail_request(image_id);
    image.set_slot(size);
    (image_id, request)
}

fn cancel_thumbnail(image_id: usize) {
    viewport::cancel_metadata(image_id);
    cancel_thumbnail_request(image_id);
}

fn cancel_thumbnail_request(image_id: usize) {
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().remove(&image_id);
    });
    SETTLE_VIEWS.with(|views| {
        views.borrow_mut().retain(|_, settle| {
            settle
                .pending
                .retain(|park| park.target.image_id != image_id);
            !settle.pending.is_empty() || (settle.hooked && settle.viewport.upgrade().is_some())
        });
    });
    let cancelled = PENDING_THUMBNAILS.with(|pending| {
        let mut pending = pending.borrow_mut();
        let mut cancelled = Vec::new();
        pending.retain(|key, thumbnail| {
            thumbnail
                .targets
                .retain(|target| target.image_id != image_id);
            if thumbnail.targets.is_empty() {
                thumbnail.cancellation.cancel();
                cancelled.push(key.clone());
                false
            } else {
                true
            }
        });
        cancelled
    });
    THUMBNAIL_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        for key in cancelled {
            queue.cancel(&key);
        }
    });
    camera::retry_after_cancel();
}

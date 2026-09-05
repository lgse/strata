// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    model::{FileEntry, MetadataValue},
    sandbox::{Cancellation, ParseOperation},
};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
/// Viewport bounding keeps the admitted work fixed.
const MAX_THUMBNAIL_WORKERS: usize = 4;
const MAX_QUEUED_THUMBNAILS: usize = 64;
const FAILED_THUMBNAIL_TTL: Duration = Duration::from_secs(30);
/// Settles scrolling before decoding; cache hits bypass the gate.
const THUMBNAIL_SETTLE_DELAY: Duration = Duration::from_millis(120);
/// Bounds starvation: overlong parks fire anyway, still viewport-intersected.
const MAX_SETTLE_WAIT: Duration = Duration::from_millis(400);
const VIEWPORT_OVERSCAN: f32 = 0.25;

thread_local! {
    static ACTIVE_REQUESTS: RefCell<HashMap<usize, ActiveRequest>> =
        RefCell::new(HashMap::new());
    static PENDING_THUMBNAILS: RefCell<HashMap<ThumbnailKey, PendingThumbnail>> =
        RefCell::new(HashMap::new());
    static THUMBNAIL_QUEUE: RefCell<ThumbnailQueue> = RefCell::new(ThumbnailQueue::default());
    static THUMBNAIL_CACHE: RefCell<ThumbnailCache> = RefCell::new(ThumbnailCache::default());
    /// Per-viewport settle groups (key zero is the fallback); one view's fling never postpones another's.
    static SETTLE_VIEWS: RefCell<HashMap<usize, ViewSettle>> = RefCell::new(HashMap::new());
    /// Parked while metadata is unknown to avoid rendering twice.
    static METADATA_WAITERS: RefCell<HashMap<PathBuf, Vec<MetadataWaiter>>> =
        RefCell::new(HashMap::new());
    static TRACKED_CUSTOMIZED_ICONS: RefCell<Vec<TrackedCustomizedIcon>> =
        const { RefCell::new(Vec::new()) };
}

struct TrackedCustomizedIcon {
    image: glib::WeakRef<gtk::Image>,
    path: PathBuf,
    icon: String,
    customized: bool,
}

struct ActiveRequest {
    id: u64,
    image: glib::WeakRef<gtk::Image>,
    deferred: Option<DeferredThumbnail>,
}

#[derive(Clone)]
struct DeferredThumbnail {
    key: ThumbnailKey,
    kind: ThumbnailKind,
}

struct PendingTarget {
    image_id: usize,
    request: u64,
    image: glib::WeakRef<gtk::Image>,
}
struct SettledPark {
    key: ThumbnailKey,
    kind: ThumbnailKind,
    target: PendingTarget,
    wait_for_metadata: bool,
}

struct ViewSettle {
    viewport: glib::WeakRef<gtk::ScrolledWindow>,
    pending: Vec<SettledPark>,
    timer: Option<glib::SourceId>,
    first_park: Option<Instant>,
    hooked: bool,
}

struct MetadataWaiter {
    group: usize,
    kind: ThumbnailKind,
    target: PendingTarget,
    file_size: Option<u64>,
    thumbnail_size: i32,
}
struct PersistJob {
    path: PathBuf,
    mtime: i64,
    png: Vec<u8>,
}

/// Bounded: slow disk never delays display; over capacity the oldest entry drops.
const MAX_PERSIST_QUEUE: usize = 32;

struct PersistQueue {
    queue: VecDeque<PersistJob>,
}

impl PersistQueue {
    const fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    fn push(&mut self, job: PersistJob) {
        if self.queue.len() >= MAX_PERSIST_QUEUE {
            self.queue.pop_front();
        }
        self.queue.push_back(job);
    }

    fn pop_front(&mut self) -> Option<PersistJob> {
        self.queue.pop_front()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.queue.len()
    }
}

// Process-wide: the persistence pump runs on a worker thread, so the queue
// cannot be a main-thread local like the settle state.
static PERSIST_QUEUE: std::sync::Mutex<PersistQueue> = std::sync::Mutex::new(PersistQueue::new());
static PERSIST_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn enqueue_persist(path: PathBuf, mtime: i64, png: Vec<u8>) {
    PERSIST_QUEUE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .push(PersistJob { path, mtime, png });
    pump_persist_queue();
}

fn pump_persist_queue() {
    use std::sync::atomic::Ordering;
    if PERSIST_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    gio::spawn_blocking(|| {
        loop {
            let job = PERSIST_QUEUE
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .pop_front();
            let Some(job) = job else {
                break;
            };
            // Best effort: store failures are dropped; the in-memory result already applied.
            super::thumbnail_cache::store(&job.path, job.mtime, &job.png);
        }
        PERSIST_RUNNING.store(false, Ordering::SeqCst);
        // A job enqueued after the drain but before the flag cleared
        // restarts the pump instead of stranding work.
        if !PERSIST_QUEUE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .queue
            .is_empty()
        {
            pump_persist_queue();
        }
    });
}

struct PendingThumbnail {
    id: u64,
    kind: ThumbnailKind,
    cancellation: Cancellation,
    targets: Vec<PendingTarget>,
}

struct ThumbnailJob {
    id: u64,
    key: ThumbnailKey,
    kind: ThumbnailKind,
    cancellation: Cancellation,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ThumbnailKey {
    path: PathBuf,
    modified: Option<i64>,
    file_size: Option<u64>,
    thumbnail_size: i32,
}

#[derive(Default)]
struct ThumbnailCache {
    entries: HashMap<ThumbnailKey, CachedThumbnail>,
    recent: VecDeque<ThumbnailKey>,
    byte_count: usize,
}

#[derive(Clone)]
enum CachedThumbnail {
    Ready(glib::Bytes),
    Failed(Instant),
}

enum CacheHit {
    Ready(glib::Bytes),
    Failed,
}

impl ThumbnailCache {
    fn get(&mut self, key: &ThumbnailKey) -> Option<CacheHit> {
        let entry = self.entries.get(key)?.clone();
        if matches!(entry, CachedThumbnail::Failed(expires) if expires <= Instant::now()) {
            self.remove(key);
            return None;
        }
        self.recent.retain(|candidate| candidate != key);
        self.recent.push_back(key.clone());
        Some(match entry {
            CachedThumbnail::Ready(bytes) => CacheHit::Ready(bytes),
            CachedThumbnail::Failed(_) => CacheHit::Failed,
        })
    }

    fn insert(&mut self, key: ThumbnailKey, bytes: glib::Bytes) {
        self.insert_entry(key, CachedThumbnail::Ready(bytes));
    }

    fn insert_failure(&mut self, key: ThumbnailKey) {
        self.insert_entry(
            key,
            CachedThumbnail::Failed(Instant::now() + FAILED_THUMBNAIL_TTL),
        );
    }

    fn insert_entry(&mut self, key: ThumbnailKey, entry: CachedThumbnail) {
        self.remove(&key);
        self.byte_count = self.byte_count.saturating_add(entry.byte_len());
        self.recent.push_back(key.clone());
        self.entries.insert(key, entry);
        while self.entries.len() > MAX_CACHE_ENTRIES || self.byte_count > MAX_CACHE_BYTES {
            let Some(oldest) = self.recent.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.byte_count = self.byte_count.saturating_sub(removed.byte_len());
            }
        }
    }

    fn remove(&mut self, key: &ThumbnailKey) {
        if let Some(removed) = self.entries.remove(key) {
            self.byte_count = self.byte_count.saturating_sub(removed.byte_len());
        }
        self.recent.retain(|candidate| candidate != key);
    }
}

impl CachedThumbnail {
    fn byte_len(&self) -> usize {
        match self {
            Self::Ready(bytes) => bytes.len(),
            Self::Failed(_) => 0,
        }
    }
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
        if self.running >= MAX_THUMBNAIL_WORKERS {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThumbnailKind {
    Image,
    RawImage,
    Pdf,
    Video,
}

pub(super) fn set_thumbnail_or_icon(
    image: &gtk::Image,
    entry: &FileEntry,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    let Some(path) = entry.location.native_path() else {
        show_fallback_icon(image, fallback_icon, icon_size);
        return;
    };
    set_thumbnail_for_path(ThumbnailRequest {
        image,
        path,
        modified: known_metadata(&entry.modified_unix_seconds),
        file_size: known_metadata(&entry.size),
        fallback_icon,
        icon_size,
        thumbnail_size,
        wait_for_metadata: true,
    });
}

pub(super) fn set_thumbnail_or_icon_for_path(
    image: &gtk::Image,
    path: &Path,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    set_thumbnail_for_path(ThumbnailRequest {
        image,
        path,
        modified: None,
        file_size: None,
        fallback_icon,
        icon_size,
        thumbnail_size,
        wait_for_metadata: false,
    });
}

/// Bundled to stay under the argument-count lint.
struct ThumbnailRequest<'a> {
    image: &'a gtk::Image,
    path: &'a Path,
    modified: Option<i64>,
    file_size: Option<u64>,
    fallback_icon: &'a str,
    icon_size: i32,
    thumbnail_size: i32,
    wait_for_metadata: bool,
}

fn set_thumbnail_for_path(request: ThumbnailRequest<'_>) {
    let has_custom_icon = super::theme::ThemeManager::shared()
        .custom_icon(request.path)
        .is_some();
    if has_custom_icon {
        set_fallback_icon(
            request.image,
            Some(request.path),
            request.fallback_icon,
            request.icon_size,
        );
        return;
    }
    let path = request.path.to_path_buf();
    let thumbnail_size = request.thumbnail_size.clamp(16, 256);
    let Some(kind) = thumbnail_kind(&path) else {
        set_fallback_icon(
            request.image,
            Some(request.path),
            request.fallback_icon,
            request.icon_size,
        );
        return;
    };
    let key = ThumbnailKey {
        path: path.clone(),
        modified: request.modified,
        file_size: request.file_size,
        thumbnail_size,
    };
    // Disk validation moves to fire time so offscreen rows never touch the disk.
    match THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&key)) {
        Some(CacheHit::Ready(bytes)) => {
            cancel_thumbnail(request.image.as_ptr() as usize);
            apply_thumbnail(request.image, &bytes, thumbnail_size);
            return;
        }
        Some(CacheHit::Failed) => {
            set_fallback_icon(
                request.image,
                Some(request.path),
                request.fallback_icon,
                request.icon_size,
            );
            return;
        }
        None => {}
    }
    let (image_id, request_id) = if should_paint_fallback(request.image.paintable().is_some()) {
        set_fallback_icon(
            request.image,
            Some(request.path),
            request.fallback_icon,
            request.icon_size,
        )
    } else {
        prepare_thumbnail_target(request.image, thumbnail_size)
    };
    let weak_image = glib::WeakRef::new();
    weak_image.set(Some(request.image));
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(
            image_id,
            ActiveRequest {
                id: request_id,
                image: weak_image.clone(),
                deferred: None,
            },
        );
    });
    let target = PendingTarget {
        image_id,
        request: request_id,
        image: weak_image,
    };
    // Binds run while GTK mutates the widget tree; walking ancestors or hooking the viewport here can corrupt layout. Defer to idle.
    glib::idle_add_local_once(move || {
        if target_live_image(&target).is_some() {
            park_thumbnail(key, kind, target, request.wait_for_metadata);
        }
    });
}

fn viewport_of(image: &gtk::Image) -> Option<gtk::ScrolledWindow> {
    let mut ancestor = image.parent();
    while let Some(widget) = ancestor {
        ancestor = widget.parent();
        if let Ok(viewport) = widget.downcast::<gtk::ScrolledWindow>() {
            return Some(viewport);
        }
    }
    None
}

fn rect_eligible(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    viewport_width: f32,
    viewport_height: f32,
) -> bool {
    let overscan = VIEWPORT_OVERSCAN;
    width > 0.0
        && height > 0.0
        && viewport_width > 0.0
        && viewport_height > 0.0
        && x < viewport_width * (1.0 + overscan)
        && x + width > -viewport_width * overscan
        && y < viewport_height * (1.0 + overscan)
        && y + height > -viewport_height * overscan
}

fn image_viewport_rect(
    image: &gtk::Image,
    viewport: &gtk::ScrolledWindow,
) -> Option<(f32, f32, f32, f32)> {
    let bounds = image.compute_bounds(viewport)?;
    Some((bounds.x(), bounds.y(), bounds.width(), bounds.height()))
}

fn list_item_owner(widget: &gtk::Widget) -> Option<gtk::Widget> {
    let mut child = widget.clone();
    while let Some(parent) = child.parent() {
        if parent.is::<gtk::ListView>() || parent.is::<gtk::GridView>() {
            return Some(child);
        }
        child = parent;
    }
    None
}

/// GTK keeps stale allocations from fling-visited rows; hit-test the visible part to find the row actually displayed.
fn image_is_currently_picked(
    image: &gtk::Image,
    viewport: &gtk::ScrolledWindow,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> bool {
    let left = x.max(0.0);
    let right = (x + width).min(viewport.width() as f32);
    let top = y.max(0.0);
    let bottom = (y + height).min(viewport.height() as f32);
    if left >= right || top >= bottom {
        return true;
    }
    let Some(picked) = viewport.pick(
        f64::from((left + right) / 2.0),
        f64::from((top + bottom) / 2.0),
        gtk::PickFlags::DEFAULT,
    ) else {
        return false;
    };
    match (
        list_item_owner(image.upcast_ref()),
        list_item_owner(&picked),
    ) {
        (Some(image), Some(picked)) => image == picked,
        _ => false,
    }
}

fn image_eligible(image: &gtk::Image, viewport: &gtk::ScrolledWindow) -> bool {
    let Some((x, y, width, height)) = image_viewport_rect(image, viewport) else {
        return false;
    };
    rect_eligible(
        x,
        y,
        width,
        height,
        viewport.width() as f32,
        viewport.height() as f32,
    ) && image_is_currently_picked(image, viewport, x, y, width, height)
}

fn group_address(viewport: Option<&gtk::ScrolledWindow>) -> usize {
    viewport.map_or(0, |viewport| viewport.as_ptr() as usize)
}

fn park_thumbnail(
    key: ThumbnailKey,
    kind: ThumbnailKind,
    target: PendingTarget,
    wait_for_metadata: bool,
) {
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
            first_park: None,
            hooked: false,
        });
        // A dead viewport's address may be recycled: reset the group instead of joining its stale hooks and pending requests.
        if group != 0 && settle.viewport.upgrade().is_none() {
            if let Some(timer) = settle.timer.take() {
                timer.remove();
            }
            *settle = ViewSettle {
                viewport: viewport_ref.clone(),
                pending: Vec::new(),
                timer: None,
                first_park: None,
                hooked: false,
            };
        }
        settle.pending.push(SettledPark {
            key,
            kind,
            target,
            wait_for_metadata,
        });
        if settle.first_park.is_none() {
            settle.first_park = Some(Instant::now());
        }
    });
    if let Some(viewport) = viewport {
        hook_viewport(group, &viewport);
    }
    request_group_fire(group);
}

#[cfg(test)]
fn schedule_or_defer(key: ThumbnailKey, kind: ThumbnailKind, target: PendingTarget) {
    park_thumbnail(key, kind, target, false);
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

/// Fires happen only on the main loop: firing inside binds or adjustment callbacks would nest thumbnail application inside GTK layout (reentrant relayout, fatal `bounds.y` assertion). Overdue groups arm a zero-delay timer so the bound holds without ever firing nested.
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
        let overdue = settle
            .first_park
            .is_some_and(|first| first.elapsed() >= MAX_SETTLE_WAIT);
        if let Some(timer) = settle.timer.take() {
            timer.remove();
        }
        let delay = if overdue {
            Duration::ZERO
        } else {
            THUMBNAIL_SETTLE_DELAY
        };
        settle.timer = Some(glib::timeout_add_local_once(delay, move || {
            fire_view_group(group);
        }));
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
        adjustment.connect_value_changed(move |_| request_group_fire(group));
        adjustment.connect_changed(move |_| request_group_fire(group));
    }
}

fn fire_view_group(group: usize) {
    let (viewport, drained) = SETTLE_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let Some(settle) = views.get_mut(&group) else {
            return (None, Vec::new());
        };
        if let Some(timer) = settle.timer.take() {
            timer.remove();
        }
        settle.first_park = None;
        let viewport = settle.viewport.upgrade();
        if group != 0 && viewport.is_none() {
            views.remove(&group);
            return (None, Vec::new());
        }
        let drained = std::mem::take(&mut settle.pending);
        (viewport, drained)
    });
    fire_parks(drained, viewport.as_ref());
}

#[cfg(test)]
fn fire_settled_thumbnails() {
    fire_view_group(0);
}

fn target_live_image(target: &PendingTarget) -> Option<gtk::Image> {
    let live = ACTIVE_REQUESTS.with(|requests| {
        requests
            .borrow()
            .get(&target.image_id)
            .is_some_and(|active| active.id == target.request)
    });
    if !live {
        return None;
    }
    target.image.upgrade()
}

fn fire_parks(mut drained: Vec<SettledPark>, viewport: Option<&gtk::ScrolledWindow>) {
    if let Some(viewport) = viewport {
        // Bind order is not display order; queue top-to-bottom so the first visible item is not overtaken.
        drained.sort_by(|left, right| {
            let position = |park: &SettledPark| {
                park.target
                    .image
                    .upgrade()
                    .and_then(|image| image_viewport_rect(&image, viewport))
                    .map_or((f32::MAX, f32::MAX), |(x, y, _, _)| (y, x))
            };
            let left = position(left);
            let right = position(right);
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.total_cmp(&right.1))
        });
    }
    let mut offscreen = Vec::new();
    let mut eligible = 0;
    let mut started = false;
    for park in drained {
        let image_id = park.target.image_id;
        let request = park.target.request;
        let live = ACTIVE_REQUESTS.with(|requests| {
            requests
                .borrow()
                .get(&image_id)
                .is_some_and(|active| active.id == request)
        });
        if !live {
            continue;
        }
        let image = park.target.image.upgrade();
        if let (Some(image), Some(viewport)) = (image.as_ref(), viewport)
            && !image_eligible(image, viewport)
        {
            // Keep pre-bound offscreen requests parked; scrolling into them starts work without a rebind.
            offscreen.push(park);
            continue;
        }
        eligible += 1;
        if let Some(image) = image.as_ref()
            && let Some(hit) = THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&park.key))
        {
            match hit {
                CacheHit::Ready(bytes) => {
                    apply_thumbnail(image, &bytes, park.key.thumbnail_size);
                }
                CacheHit::Failed => {}
            }
            continue;
        }
        if park.wait_for_metadata && park.key.modified.is_none() {
            push_metadata_waiter(group_of_viewport(viewport), park);
            continue;
        }
        if schedule_thumbnail(park.key.clone(), park.kind, park.target) {
            started = true;
        } else {
            mark_deferred(park.key, park.kind, image_id, request);
        }
    }
    if started {
        start_thumbnail_jobs();
    }
    crate::metrics::mark_thumbnail_eligible(eligible);
    if let Some(viewport) = viewport
        && !offscreen.is_empty()
    {
        let group = group_address(Some(viewport));
        SETTLE_VIEWS.with(|views| {
            if let Some(settle) = views.borrow_mut().get_mut(&group) {
                settle.pending.extend(offscreen);
            }
        });
    }
}

fn group_of_viewport(viewport: Option<&gtk::ScrolledWindow>) -> usize {
    group_address(viewport)
}

fn push_metadata_waiter(group: usize, park: SettledPark) {
    METADATA_WAITERS.with(|waiters| {
        let mut waiters = waiters.borrow_mut();
        let queue = waiters.entry(park.key.path.clone()).or_default();
        // Capped per file; extras re-park on their next bind.
        if queue.len() < 8 {
            queue.push(MetadataWaiter {
                group,
                kind: park.kind,
                target: park.target,
                file_size: park.key.file_size,
                thumbnail_size: park.key.thumbnail_size,
            });
        }
    });
}

pub(super) fn note_metadata(path: &Path, modified: Option<i64>, file_size: Option<u64>) {
    // A completed metadata attempt releases thumbnail work even when mtime is unavailable.
    // Such renders remain memory-only because the shared cache cannot validate them.
    SETTLE_VIEWS.with(|views| {
        for settle in views.borrow_mut().values_mut() {
            for park in &mut settle.pending {
                if park.wait_for_metadata && park.key.path == path {
                    park.key.modified = modified;
                    park.key.file_size = file_size.or(park.key.file_size);
                    park.wait_for_metadata = false;
                }
            }
        }
    });
    ACTIVE_REQUESTS.with(|requests| {
        for deferred in requests
            .borrow_mut()
            .values_mut()
            .filter_map(|active| active.deferred.as_mut())
        {
            if deferred.key.path == path {
                deferred.key.modified = modified;
                deferred.key.file_size = file_size.or(deferred.key.file_size);
            }
        }
    });
    let Some(waiters) = METADATA_WAITERS.with(|waiters| waiters.borrow_mut().remove(path)) else {
        return;
    };
    for waiter in waiters {
        if target_live_image(&waiter.target).is_none() {
            continue;
        }
        let key = ThumbnailKey {
            path: path.to_path_buf(),
            modified,
            file_size: file_size.or(waiter.file_size),
            thumbnail_size: waiter.thumbnail_size,
        };
        park_into_group(waiter.group, key, waiter.kind, waiter.target, false);
    }
}
pub(super) fn note_metadata_entry(entry: &FileEntry) {
    let Some(path) = entry.location.native_path() else {
        return;
    };
    note_metadata(
        path,
        known_metadata(&entry.modified_unix_seconds),
        known_metadata(&entry.size),
    );
}

fn park_into_group(
    group: usize,
    key: ThumbnailKey,
    kind: ThumbnailKind,
    target: PendingTarget,
    wait_for_metadata: bool,
) {
    let known = SETTLE_VIEWS.with(|views| views.borrow().contains_key(&group));
    if !known && group != 0 {
        park_thumbnail(key, kind, target, wait_for_metadata);
        return;
    }
    SETTLE_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let Some(settle) = views.get_mut(&group) else {
            return;
        };
        settle.pending.push(SettledPark {
            key,
            kind,
            target,
            wait_for_metadata,
        });
        if settle.first_park.is_none() {
            settle.first_park = Some(Instant::now());
        }
    });
    request_group_fire(group);
}

fn schedule_thumbnail(key: ThumbnailKey, kind: ThumbnailKind, target: PendingTarget) -> bool {
    PENDING_THUMBNAILS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if let Some(pending) = pending.get_mut(&key) {
            pending.targets.push(target);
            true
        } else {
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

fn start_thumbnail_jobs() {
    while let Some(key) = THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().begin_next()) {
        let job = PENDING_THUMBNAILS.with(|pending| {
            pending.borrow().get(&key).map(|pending| ThumbnailJob {
                id: pending.id,
                key,
                kind: pending.kind,
                cancellation: pending.cancellation.clone(),
            })
        });
        let Some(job) = job else {
            THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().finish());
            continue;
        };
        crate::metrics::mark_thumbnail_started();
        glib::MainContext::default().spawn_local(run_thumbnail_job(job));
    }
}

async fn run_thumbnail_job(job: ThumbnailJob) {
    let job_id = job.id;
    let key = job.key.clone();
    let thumbnail_size = key.thumbnail_size;
    let result = gio::spawn_blocking(move || {
        if let Some(mtime) = job.key.modified
            && let Some(png) = super::thumbnail_cache::lookup(&job.key.path, mtime)
        {
            return Ok((png, false));
        }
        // Render the canonical size class: one entry per file, so a small view must never poison the bucket.
        render_thumbnail(
            &job.key.path,
            job.kind,
            super::thumbnail_cache::CANONICAL_MAX_EDGE,
            &job.cancellation,
        )
        .map(|png| (png, true))
    })
    .await;
    let targets = take_pending_targets(&key, job_id);
    THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().finish());
    if let Some(targets) = targets {
        match result {
            Ok(Ok((png, rendered))) => {
                crate::metrics::mark_thumbnail_completed();
                let bytes = glib::Bytes::from_owned(png.clone());
                THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert(key.clone(), bytes.clone()));
                finish_thumbnail_targets(targets, Some(&bytes), thumbnail_size);
                // Unverifiable keys (unknown mtime) skip persistence: nothing validates them later.
                if rendered && let Some(mtime) = key.modified {
                    enqueue_persist(key.path.clone(), mtime, png);
                }
            }
            Ok(Err(_)) | Err(_) => {
                crate::metrics::mark_thumbnail_cancelled();
                THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert_failure(key));
                finish_thumbnail_targets(targets, None, thumbnail_size);
            }
        }
    }
    start_thumbnail_jobs();
    retry_deferred_thumbnails();
    let counts = crate::metrics::thumbnail_counts();
    tracing::debug!(?counts, "thumbnail pipeline settled");
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
                .min_by_key(|(_, request, _, _)| *request)
        });
        let Some((image_id, request, image, deferred)) = deferred else {
            break;
        };
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
    image: glib::WeakRef<gtk::Image>,
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
    bytes: Option<&glib::Bytes>,
    thumbnail_size: i32,
) {
    for target in targets {
        let is_current = ACTIVE_REQUESTS.with(|requests| {
            let mut requests = requests.borrow_mut();
            if requests
                .get(&target.image_id)
                .is_some_and(|active| active.id == target.request)
            {
                requests.remove(&target.image_id);
                true
            } else {
                false
            }
        });
        if !is_current {
            crate::metrics::mark_thumbnail_stale();
            continue;
        }
        let Some(bytes) = bytes else {
            continue;
        };
        let Some(image) = target.image.upgrade() else {
            crate::metrics::mark_thumbnail_stale();
            continue;
        };
        apply_thumbnail(&image, bytes, thumbnail_size);
        crate::metrics::mark_thumbnail_applied();
    }
}

fn known_metadata<T: Copy>(value: &MetadataValue<T>) -> Option<T> {
    match value {
        MetadataValue::Known(value) => Some(*value),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

fn apply_thumbnail(image: &gtk::Image, bytes: &glib::Bytes, thumbnail_size: i32) {
    if let Ok(texture) = gdk::Texture::from_bytes(bytes) {
        crate::assets::remove_primary_icon(image);
        ensure_image_slot(image, thumbnail_size);
        image.set_paintable(Some(&texture));
        image.set_opacity(1.0);
    }
}

pub(super) fn show_fallback_icon(image: &gtk::Image, icon: &str, size: i32) {
    set_fallback_icon(image, None, icon, size);
}

pub(super) fn show_customized_icon(
    image: &gtk::Image,
    path: &Path,
    fallback_icon: &str,
    size: i32,
) {
    set_fallback_icon(image, Some(path), fallback_icon, size);
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
    if let Some(image) = widget.downcast_ref::<gtk::Image>() {
        cancel_thumbnail(image.as_ptr() as usize);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        cancel_thumbnails_in(&current);
    }
}

pub(super) fn ensure_image_slot(image: &gtk::Image, size: i32) {
    if image.pixel_size() != size {
        image.set_pixel_size(size);
    }
    if image.width_request() != size || image.height_request() != size {
        image.set_size_request(size, size);
    }
}

fn should_paint_fallback(has_content: bool) -> bool {
    !has_content
}

fn prepare_thumbnail_target(image: &gtk::Image, size: i32) -> (usize, u64) {
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let image_id = image.as_ptr() as usize;
    cancel_thumbnail(image_id);
    ensure_image_slot(image, size);
    (image_id, request)
}

fn set_fallback_icon(
    image: &gtk::Image,
    path: Option<&Path>,
    icon: &str,
    size: i32,
) -> (usize, u64) {
    let ids = prepare_thumbnail_target(image, size);
    if let Some(p) = path {
        let customized = apply_path_customization(image, p, icon);
        register_tracked_icon(image, p, icon, customized);
    } else {
        crate::assets::set_primary_icon(image, icon);
    }
    ids
}

fn apply_path_customization(image: &gtk::Image, path: &Path, fallback_icon: &str) -> bool {
    let theme_manager = super::theme::ThemeManager::shared();
    let custom_icon = theme_manager.custom_icon(path);
    let color = theme_manager.folder_color(path);
    let customized = custom_icon.is_some() || color.is_some();

    if fallback_icon == crate::assets::icons::FOLDER
        && let Some(decoration) = custom_icon.as_deref()
    {
        let color = color
            .as_ref()
            .map_or_else(crate::assets::primary_icon_color, |color| {
                color.hex().to_owned()
            });
        crate::assets::set_folder_decoration_icon(image, decoration, &color);
    } else if let Some(emoji) = custom_icon
        .as_deref()
        .and_then(crate::assets::icons::custom_emoji)
    {
        crate::assets::set_emoji_icon(image, emoji);
    } else {
        let rendered_icon = custom_icon.as_deref().unwrap_or(fallback_icon);
        if let Some(color) = color {
            crate::assets::set_custom_colored_icon(image, rendered_icon, color.hex());
        } else {
            crate::assets::set_primary_icon(image, rendered_icon);
        }
    }
    customized
}

fn register_tracked_icon(image: &gtk::Image, path: &Path, icon: &str, customized: bool) {
    let weak_ref = glib::WeakRef::new();
    weak_ref.set(Some(image));
    TRACKED_CUSTOMIZED_ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        icons.retain(|t| t.image.upgrade().is_some());
        if let Some(existing) = icons
            .iter_mut()
            .find(|t| t.image.upgrade().as_ref() == Some(image))
        {
            existing.path = path.to_path_buf();
            existing.icon = icon.to_owned();
            existing.customized = customized;
        } else {
            icons.push(TrackedCustomizedIcon {
                image: weak_ref,
                path: path.to_path_buf(),
                icon: icon.to_owned(),
                customized,
            });
        }
    });
}

pub(super) fn refresh_customized_icons(paths: &[PathBuf]) {
    refresh_tracked_icons(|tracked| paths.iter().any(|candidate| candidate == &tracked.path));
}

pub(super) fn refresh_all_customized_icons() {
    refresh_tracked_icons(|tracked| tracked.customized);
}

fn refresh_tracked_icons(matches: impl Fn(&TrackedCustomizedIcon) -> bool) {
    TRACKED_CUSTOMIZED_ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        icons.retain_mut(|tracked| {
            let Some(image) = tracked.image.upgrade() else {
                return false;
            };
            if matches(tracked) {
                cancel_thumbnail(image.as_ptr() as usize);
                tracked.customized = apply_path_customization(&image, &tracked.path, &tracked.icon);
            }
            true
        });
    });
}

fn cancel_thumbnail(image_id: usize) {
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().remove(&image_id);
    });
    METADATA_WAITERS.with(|waiters| {
        waiters.borrow_mut().retain(|_, targets| {
            targets.retain(|waiter| waiter.target.image_id != image_id);
            !targets.is_empty()
        });
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
    retry_deferred_thumbnails();
}

fn thumbnail_kind(path: &Path) -> Option<ThumbnailKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" => {
            Some(ThumbnailKind::Image)
        }
        "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf" | "kdc" | "mef" | "mos" | "mrw"
        | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2" | "rwl" | "sr2" | "srf" | "srw"
        | "x3f" => Some(ThumbnailKind::RawImage),
        "pdf" => Some(ThumbnailKind::Pdf),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "ogv" => {
            Some(ThumbnailKind::Video)
        }
        _ => None,
    }
}

fn render_thumbnail(
    path: &Path,
    kind: ThumbnailKind,
    size: i32,
    cancellation: &Cancellation,
) -> Result<Vec<u8>, String> {
    let operation = match kind {
        ThumbnailKind::Image => ParseOperation::ThumbnailImage,
        ThumbnailKind::RawImage => ParseOperation::ThumbnailRaw,
        ThumbnailKind::Pdf => ParseOperation::ThumbnailPdf,
        ThumbnailKind::Video => ParseOperation::ThumbnailVideo,
    };
    crate::sandbox::parse(
        path,
        operation,
        size.clamp(16, 256),
        crate::sandbox::MediaPreviewBackend::Software,
        cancellation,
    )
    .map(|output| output.data)
}

#[cfg(test)]
mod tests;

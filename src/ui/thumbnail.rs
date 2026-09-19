// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
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

mod background;
mod camera;
mod slot;
mod viewport;
pub(crate) use slot::ThumbnailSlot;
pub(super) use viewport::{near_viewport, request_metadata};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CACHE_READERS: usize = 4;
// Cache reads never wait for a decoder permit.
thread_local! {
    static RENDER_QUEUE: RefCell<VecDeque<ThumbnailJob>> = const { RefCell::new(VecDeque::new()) };
    static RENDER_RUNNING: Cell<usize> = const { Cell::new(0) };
    static HEAVY_RUNNING: Cell<usize> = const { Cell::new(0) };
}
const MAX_QUEUED_THUMBNAILS: usize = 64;
const FAILED_THUMBNAIL_TTL: Duration = Duration::from_secs(30);

thread_local! {
    static ACTIVE_REQUESTS: RefCell<HashMap<usize, ActiveRequest>> =
        RefCell::new(HashMap::new());
    static PENDING_THUMBNAILS: RefCell<HashMap<ThumbnailKey, PendingThumbnail>> =
        RefCell::new(HashMap::new());
    static THUMBNAIL_QUEUE: RefCell<ThumbnailQueue> = RefCell::new(ThumbnailQueue::default());
    static THUMBNAIL_CACHE: RefCell<ThumbnailCache> = RefCell::new(ThumbnailCache::default());
    /// Per-viewport admission batches; one view's fling never postpones another's.
    static SETTLE_VIEWS: RefCell<HashMap<usize, ViewSettle>> = RefCell::new(HashMap::new());
    static TRACKED_CUSTOMIZED_ICONS: RefCell<Vec<TrackedCustomizedIcon>> =
        const { RefCell::new(Vec::new()) };
    static TRACKED_THUMBNAILS: RefCell<Vec<TrackedThumbnail>> = const { RefCell::new(Vec::new()) };
    static REFRESHING_CUSTOMIZED_ICONS: Cell<bool> = const { Cell::new(false) };
}

struct TrackedThumbnail {
    image: glib::WeakRef<ThumbnailSlot>,
    path: PathBuf,
}

struct TrackedCustomizedIcon {
    image: glib::WeakRef<ThumbnailSlot>,
    path: PathBuf,
    icon: String,
    customized: bool,
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
    resolved: ThumbnailKey,
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
    Ready(gdk::Texture),
    Failed(Instant),
}

enum CacheHit {
    Ready(gdk::Texture),
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
            CachedThumbnail::Ready(texture) => CacheHit::Ready(texture),
            CachedThumbnail::Failed(_) => CacheHit::Failed,
        })
    }

    fn insert(&mut self, key: ThumbnailKey, texture: gdk::Texture) {
        self.insert_entry(key, CachedThumbnail::Ready(texture));
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
            Self::Ready(texture) => {
                (texture.width().max(0) as usize).saturating_mul(texture.height().max(0) as usize)
                    * 4
            }
            Self::Failed(_) => 0,
        }
    }
}

pub(super) fn preserve_renamed_thumbnail(from: &crate::model::Location, entry: &FileEntry) {
    let Some((from, to)) = from.native_path().zip(entry.location.native_path()) else {
        return;
    };
    if from == to || from.extension() != to.extension() {
        return;
    }
    let (Some(modified), Some(file_size)) = (
        known_metadata(&entry.modified_unix_seconds),
        known_metadata(&entry.size),
    ) else {
        return;
    };
    THUMBNAIL_CACHE.with_borrow_mut(|cache| {
        let key = ThumbnailKey {
            path: from.to_path_buf(),
            modified: Some(modified),
            file_size: Some(file_size),
            thumbnail_size: super::thumbnail_cache::CANONICAL_MAX_EDGE,
        };
        if let Some(CacheHit::Ready(texture)) = cache.get(&key) {
            cache.remove(&key);
            cache.insert(
                ThumbnailKey {
                    path: to.to_path_buf(),
                    ..key
                },
                texture,
            );
        }
    });
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThumbnailKind {
    Camera,
    Image,
    RawImage,
    Pdf,
    Video,
    AppImage,
}

pub(super) fn set_thumbnail_or_icon(
    image: &ThumbnailSlot,
    entry: &FileEntry,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    let Some(path) = entry.local_thumbnail_path() else {
        if entry.location.backend_name() == "gphoto2"
            && !entry.is_directory()
            && thumbnail_kind(Path::new(&entry.native_name)).is_some()
        {
            set_thumbnail_for_path(ThumbnailRequest {
                image,
                path: Path::new(entry.location.uri_value().unwrap_or_default()),
                kind: Some(ThumbnailKind::Camera),
                modified: known_metadata(&entry.modified_unix_seconds),
                file_size: known_metadata(&entry.size),
                fallback_icon,
                icon_size,
                thumbnail_size,
            });
        } else if let Some((mirror_path, kind)) = remote_mirror_thumbnail(entry) {
            set_thumbnail_for_path(ThumbnailRequest {
                image,
                path: &mirror_path,
                kind: Some(kind),
                modified: known_metadata(&entry.modified_unix_seconds),
                file_size: known_metadata(&entry.size),
                fallback_icon,
                icon_size,
                thumbnail_size,
            });
        } else {
            show_fallback_icon(image, fallback_icon, icon_size);
        }
        return;
    };
    set_thumbnail_for_path(ThumbnailRequest {
        image,
        path,
        kind: if entry.is_directory() {
            None
        } else {
            thumbnail_kind(Path::new(&entry.display_name))
        },
        modified: known_metadata(&entry.modified_unix_seconds),
        file_size: known_metadata(&entry.size),
        fallback_icon,
        icon_size,
        thumbnail_size,
    });
}

// GVfs FUSE paths are render inputs only; navigation must retain the URI identity.
fn remote_mirror_thumbnail(entry: &FileEntry) -> Option<(PathBuf, ThumbnailKind)> {
    if entry.is_directory() {
        return None;
    }
    let kind = thumbnail_kind(Path::new(&entry.display_name))?;
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

fn set_thumbnail_for_path(request: ThumbnailRequest<'_>) {
    let customization_path = (request.kind != Some(ThumbnailKind::Camera)).then_some(request.path);
    let has_custom_icon = customization_path.is_some_and(|path| {
        super::preferences::PreferenceManager::shared()
            .custom_icon(path)
            .is_some()
    });
    if has_custom_icon {
        set_fallback_icon(
            request.image,
            customization_path,
            request.fallback_icon,
            request.icon_size,
        );
        return;
    }
    let path = request.path.to_path_buf();
    let thumbnail_size = request.thumbnail_size.clamp(16, 256);
    let Some(kind) = request.kind else {
        set_fallback_icon(
            request.image,
            customization_path,
            request.fallback_icon,
            request.icon_size,
        );
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
            set_fallback_icon(
                request.image,
                customization_path,
                request.fallback_icon,
                request.icon_size,
            );
            return;
        }
        None => {}
    }
    let already_requested = ACTIVE_REQUESTS.with(|requests| {
        requests
            .borrow()
            .get(&(request.image.as_ptr() as usize))
            .is_some_and(|active| {
                active.key == key && active.image.upgrade().as_ref() == Some(request.image)
            })
    });
    if already_requested {
        request.image.set_slot(thumbnail_size);
        return;
    }
    let (image_id, request_id) = set_fallback_icon(
        request.image,
        customization_path,
        request.fallback_icon,
        request.icon_size,
    );
    request.image.set_slot(thumbnail_size);
    let target = register_active_request(request.image, image_id, request_id, key.clone());
    // Walking ancestors or hooking the viewport during bind can corrupt layout.
    glib::idle_add_local_once(move || {
        if request_is_live(&target) {
            park_thumbnail(key, kind, target);
        }
    });
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

#[cfg(test)]
fn schedule_or_defer(key: ThumbnailKey, kind: ThumbnailKind, target: PendingTarget) {
    park_thumbnail(key, kind, target);
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

#[cfg(test)]
fn fire_settled_thumbnails() {
    fire_view_group(0);
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
        if let Some(hit) = THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&park.key)) {
            match hit {
                CacheHit::Ready(texture) => {
                    apply_live_thumbnail(park.target, texture, park.key.path);
                }
                CacheHit::Failed => {}
            }
            continue;
        }
        let image_id = park.target.image_id;
        let request = park.target.request;
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

fn start_thumbnail_jobs() {
    while let Some(key) = THUMBNAIL_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        if queue.running < MAX_CACHE_READERS {
            viewport::prioritize_queue(&mut queue.queued);
        }
        queue.begin_next()
    }) {
        let job = PENDING_THUMBNAILS.with(|pending| {
            pending.borrow().get(&key).map(|pending| ThumbnailJob {
                id: pending.id,
                resolved: key.clone(),
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

async fn run_thumbnail_job(mut job: ThumbnailJob) {
    let mut key = job.key.clone();
    let cached = if job.kind == ThumbnailKind::Camera {
        None
    } else {
        let lookup = background::cache(move || {
            use std::os::unix::fs::MetadataExt;
            if let Ok(metadata) = std::fs::metadata(&key.path) {
                key.modified = Some(metadata.mtime());
                key.file_size = Some(metadata.len());
            }
            let png = key
                .modified
                .and_then(|mtime| super::thumbnail_cache::lookup(&key.path, mtime));
            (key, png)
        })
        .await;
        match lookup {
            Ok((key, png)) => {
                job.resolved = key;
                png
            }
            Err(_) => None,
        }
    };
    let memory = THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&job.resolved));
    if let Some(hit) = memory {
        if let Some(targets) = take_pending_targets(&job.key, job.id) {
            let texture = match hit {
                CacheHit::Ready(texture) => Some(texture),
                CacheHit::Failed => None,
            };
            finish_thumbnail_targets(targets, texture.as_ref(), &job.key.path);
        }
    } else if let Some(png) = cached {
        tracing::debug!("browser thumbnail disk hit");
        finish_thumbnail_job(
            job,
            Ok((
                crate::sandbox::browser::Thumbnail {
                    png,
                    metadata: None,
                },
                false,
            )),
        )
        .await;
    } else if !job.cancellation.is_cancelled() {
        RENDER_QUEUE.with(|queue| queue.borrow_mut().push_back(job));
    }
    THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().finish());
    start_render_jobs();
    start_thumbnail_jobs();
    retry_deferred_thumbnails();
}

fn heavy(kind: ThumbnailKind) -> bool {
    matches!(
        kind,
        ThumbnailKind::RawImage | ThumbnailKind::Pdf | ThumbnailKind::Video
    )
}

pub(in crate::ui) fn set_worker_limit(workers: usize) {
    crate::sandbox::browser::set_worker_limit(workers);
    start_render_jobs();
}

fn start_render_jobs() {
    let limit = crate::sandbox::browser::worker_limit();
    while RENDER_RUNNING.with(Cell::get) < limit {
        let job = RENDER_QUEUE.with(|queue| {
            let mut queue = queue.borrow_mut();
            queue.retain(|job| !job.cancellation.is_cancelled());
            queue
                .make_contiguous()
                .sort_by_cached_key(|job| viewport::key_priority(&job.key));
            let index = queue.iter().position(|job| {
                !heavy(job.kind) || HEAVY_RUNNING.with(Cell::get) < limit.saturating_sub(1).max(1)
            })?;
            queue.remove(index)
        });
        let Some(job) = job else {
            break;
        };
        if viewport::key_priority(&job.key).0 >= 2 {
            if let Some(targets) = take_pending_targets(&job.key, job.id) {
                for target in targets {
                    mark_deferred(job.key.clone(), job.kind, target.image_id, target.request);
                }
            }
            continue;
        }
        RENDER_RUNNING.with(|running| running.set(running.get() + 1));
        if heavy(job.kind) {
            HEAVY_RUNNING.with(|running| running.set(running.get() + 1));
        }
        glib::MainContext::default().spawn_local(run_render_job(job));
    }
}

async fn run_render_job(job: ThumbnailJob) {
    let preview_turn = (job.kind == ThumbnailKind::Camera)
        .then(|| crate::services::camera_preview::begin(&job.key.path.to_string_lossy()));
    let result = if job.kind == ThumbnailKind::Camera {
        camera::render(&job.key.path, &job.cancellation)
            .await
            .map(|png| {
                (
                    crate::sandbox::browser::Thumbnail {
                        png,
                        metadata: None,
                    },
                    false,
                )
            })
    } else {
        let path = job.key.path.clone();
        let kind = job.kind;
        let cancellation = job.cancellation.clone();
        background::render(move || render_thumbnail(&path, kind, &cancellation))
            .await
            .map_err(|_| "Thumbnail worker failed".to_owned())
            .and_then(|result| result)
            .map(|png| (png, true))
    };
    let was_heavy = heavy(job.kind);
    finish_thumbnail_job(job, result).await;
    RENDER_RUNNING.with(|running| running.set(running.get().saturating_sub(1)));
    if was_heavy {
        HEAVY_RUNNING.with(|running| running.set(running.get().saturating_sub(1)));
    }
    drop(preview_turn);
    start_render_jobs();
    start_thumbnail_jobs();
    retry_deferred_thumbnails();
}

async fn finish_thumbnail_job(
    job: ThumbnailJob,
    result: Result<(crate::sandbox::browser::Thumbnail, bool), String>,
) {
    let targets = take_pending_targets(&job.key, job.id);
    let key = job.resolved;
    let path = key.path.clone();
    if let Some(targets) = targets {
        match result {
            Ok((thumbnail, rendered)) => {
                if let Some(metadata) = thumbnail.metadata {
                    for target in &targets {
                        if request_is_live(target) {
                            viewport::publish_thumbnail_metadata(target.image_id, &path, &metadata);
                        }
                    }
                }
                let png = thumbnail.png;
                crate::metrics::mark_thumbnail_completed();
                let bytes = glib::Bytes::from_owned(png.clone());
                let texture = background::cache(move || gdk::Texture::from_bytes(&bytes).ok())
                    .await
                    .ok()
                    .flatten();
                if let Some(texture) = texture {
                    THUMBNAIL_CACHE
                        .with(|cache| cache.borrow_mut().insert(key.clone(), texture.clone()));
                    finish_thumbnail_targets(targets, Some(&texture), &path);
                } else {
                    finish_thumbnail_targets(targets, None, &path);
                }
                if rendered && let Some(mtime) = key.modified {
                    enqueue_persist(key.path.clone(), mtime, png);
                }
            }
            Err(_) => {
                crate::metrics::mark_thumbnail_cancelled();
                THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert_failure(key));
                finish_thumbnail_targets(targets, None, &path);
            }
        }
    }
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
    TRACKED_THUMBNAILS.with(|thumbnails| {
        let mut thumbnails = thumbnails.borrow_mut();
        thumbnails.retain(|tracked| tracked.image.upgrade().is_some());
        thumbnails
            .iter()
            .find(|tracked| tracked.image.upgrade().as_ref() == Some(image))
            .is_some_and(|tracked| tracked.path == path)
    })
}

fn register_displayed_thumbnail(image: &ThumbnailSlot, path: &Path) {
    let weak_ref = glib::WeakRef::new();
    weak_ref.set(Some(image));
    TRACKED_THUMBNAILS.with(|thumbnails| {
        let mut thumbnails = thumbnails.borrow_mut();
        thumbnails.retain(|tracked| tracked.image.upgrade().is_some());
        if let Some(existing) = thumbnails
            .iter_mut()
            .find(|tracked| tracked.image.upgrade().as_ref() == Some(image))
        {
            existing.path = path.to_path_buf();
        } else {
            thumbnails.push(TrackedThumbnail {
                image: weak_ref,
                path: path.to_path_buf(),
            });
        }
    });
}

fn clear_displayed_thumbnail(image: &ThumbnailSlot) {
    TRACKED_THUMBNAILS.with(|thumbnails| {
        thumbnails.borrow_mut().retain(|tracked| {
            tracked
                .image
                .upgrade()
                .is_some_and(|tracked_image| tracked_image != *image)
        });
    });
}

pub(super) fn show_fallback_icon(image: &ThumbnailSlot, icon: &str, size: i32) {
    set_fallback_icon(image, None, icon, size);
}

pub(super) fn show_customized_icon(
    image: &ThumbnailSlot,
    path: &Path,
    fallback_icon: &str,
    size: i32,
) {
    set_fallback_icon(image, Some(path), fallback_icon, size);
}

pub(super) fn show_customized_icon_image(
    image: &gtk::Image,
    path: &Path,
    fallback_icon: &str,
    size: i32,
) {
    if image.pixel_size() != size {
        image.set_pixel_size(size);
    }
    if image.width_request() != size || image.height_request() != size {
        image.set_size_request(size, size);
    }
    apply_path_customization_image(image, path, fallback_icon);
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
    cancel_thumbnail(image_id);
    image.set_slot(size);
    (image_id, request)
}

fn set_fallback_icon(
    image: &ThumbnailSlot,
    path: Option<&Path>,
    icon: &str,
    size: i32,
) -> (usize, u64) {
    let ids = prepare_thumbnail_target(image, size);
    clear_displayed_thumbnail(image);
    let (texture, customized) = path_icon_texture(path, icon);
    image.set_fallback(icon, texture.as_ref());
    if let Some(p) = path {
        register_tracked_icon(image, p, icon, customized);
    }
    ids
}

fn path_icon_texture(path: Option<&Path>, fallback_icon: &str) -> (Option<gdk::Texture>, bool) {
    let Some(path) = path else {
        return (crate::assets::primary_icon_paintable(fallback_icon), false);
    };
    let preference_manager = super::preferences::PreferenceManager::shared();
    let custom_icon = preference_manager.custom_icon(path);
    let color = preference_manager.folder_color(path);
    let customized = custom_icon.is_some() || color.is_some();
    let texture = if fallback_icon == crate::assets::icons::FOLDER
        && let Some(decoration) = custom_icon.as_deref()
    {
        let color = color
            .as_ref()
            .map_or_else(crate::assets::primary_icon_color, |color| {
                color.hex().to_owned()
            });
        crate::assets::folder_decoration_paintable(decoration, &color)
    } else if let Some(emoji) = custom_icon
        .as_deref()
        .and_then(crate::assets::icons::custom_emoji)
    {
        crate::assets::emoji_icon_paintable(emoji)
    } else {
        let rendered_icon = custom_icon.as_deref().unwrap_or(fallback_icon);
        if let Some(color) = color {
            crate::assets::custom_colored_icon_paintable(rendered_icon, color.hex())
        } else {
            crate::assets::primary_icon_paintable(rendered_icon)
        }
    };
    (texture, customized)
}

fn apply_path_customization(image: &ThumbnailSlot, path: &Path, fallback_icon: &str) -> bool {
    let (texture, customized) = path_icon_texture(Some(path), fallback_icon);
    image.set_fallback(fallback_icon, texture.as_ref());
    customized
}

fn apply_path_customization_image(image: &gtk::Image, path: &Path, fallback_icon: &str) -> bool {
    let preference_manager = super::preferences::PreferenceManager::shared();
    let custom_icon = preference_manager.custom_icon(path);
    let color = preference_manager.folder_color(path);
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

fn register_tracked_icon(image: &ThumbnailSlot, path: &Path, icon: &str, customized: bool) {
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
    refresh_tracked_icons(|_| true);
}

fn refresh_tracked_icons(matches: impl Fn(&TrackedCustomizedIcon) -> bool) {
    if REFRESHING_CUSTOMIZED_ICONS.with(|busy| busy.replace(true)) {
        return;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            REFRESHING_CUSTOMIZED_ICONS.with(|busy| busy.set(false));
        }
    }
    let _reset = Reset;
    let pending = TRACKED_CUSTOMIZED_ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        icons.retain(|tracked| tracked.image.upgrade().is_some());
        icons
            .iter()
            .filter(|tracked| matches(tracked))
            .filter_map(|tracked| {
                let image = tracked.image.upgrade()?;
                image
                    .texture()
                    .is_none()
                    .then(|| (image, tracked.path.clone(), tracked.icon.clone()))
            })
            .collect::<Vec<_>>()
    });
    for (image, path, icon) in pending {
        if super::preferences::PreferenceManager::shared()
            .custom_icon(&path)
            .is_some()
        {
            cancel_thumbnail(image.as_ptr() as usize);
        }
        let customized = apply_path_customization(&image, &path, &icon);
        TRACKED_CUSTOMIZED_ICONS.with(|icons| {
            let Ok(mut icons) = icons.try_borrow_mut() else {
                return;
            };
            if let Some(tracked) = icons
                .iter_mut()
                .find(|tracked| tracked.image.upgrade().as_ref() == Some(&image))
            {
                tracked.customized = customized;
            }
        });
    }
}

fn cancel_thumbnail(image_id: usize) {
    viewport::cancel_metadata(image_id);
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

fn thumbnail_kind(path: &Path) -> Option<ThumbnailKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" | "heic"
        | "heif" | "avif" | "jxl" => Some(ThumbnailKind::Image),
        "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf" | "kdc" | "mef" | "mos" | "mrw"
        | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2" | "rwl" | "sr2" | "srf" | "srw"
        | "x3f" => Some(ThumbnailKind::RawImage),
        "pdf" => Some(ThumbnailKind::Pdf),
        "appimage" => Some(ThumbnailKind::AppImage),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "ogv" => {
            Some(ThumbnailKind::Video)
        }
        _ => None,
    }
}

fn render_thumbnail(
    path: &Path,
    kind: ThumbnailKind,
    cancellation: &Cancellation,
) -> Result<crate::sandbox::browser::Thumbnail, String> {
    let operation = match kind {
        ThumbnailKind::Camera => return Err("Camera thumbnails require their preview icon".into()),
        ThumbnailKind::Image => ParseOperation::ThumbnailImage,
        ThumbnailKind::RawImage => ParseOperation::ThumbnailRaw,
        ThumbnailKind::Pdf => ParseOperation::ThumbnailPdf,
        ThumbnailKind::Video => ParseOperation::ThumbnailVideo,
        ThumbnailKind::AppImage => ParseOperation::ThumbnailAppImage,
    };
    crate::sandbox::browser::thumbnail(path, operation, cancellation)
}

#[cfg(test)]
pub(super) fn pending_thumbnail_id(path: &Path) -> Option<u64> {
    PENDING_THUMBNAILS.with(|pending| {
        pending
            .borrow()
            .iter()
            .find_map(|(key, pending)| (key.path == path).then_some(pending.id))
    })
}

#[cfg(test)]
pub(super) fn has_pending_thumbnail(path: &Path) -> bool {
    pending_thumbnail_id(path).is_some()
}

#[cfg(test)]
pub(super) fn hold_thumbnail_workers() {
    THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().running = MAX_CACHE_READERS);
}

#[cfg(test)]
pub(super) fn clear_thumbnail_runtime() {
    THUMBNAIL_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.running = 0;
        queue.queued.clear();
    });
    PENDING_THUMBNAILS.with(|pending| pending.borrow_mut().clear());
    ACTIVE_REQUESTS.with(|requests| requests.borrow_mut().clear());
    SETTLE_VIEWS.with(|views| views.borrow_mut().clear());
}

#[cfg(test)]
pub(super) mod tests;

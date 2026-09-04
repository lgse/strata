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
const MAX_THUMBNAIL_WORKERS: usize = 1;
const MAX_QUEUED_THUMBNAILS: usize = 64;
const MAX_LOOKAHEAD_ITEMS: usize = 16;
const FAILED_THUMBNAIL_TTL: Duration = Duration::from_secs(30);

thread_local! {
    static ACTIVE_REQUESTS: RefCell<HashMap<usize, ActiveRequest>> =
        RefCell::new(HashMap::new());
    static PENDING_THUMBNAILS: RefCell<HashMap<ThumbnailKey, PendingThumbnail>> =
        RefCell::new(HashMap::new());
    static THUMBNAIL_QUEUE: RefCell<ThumbnailQueue> = RefCell::new(ThumbnailQueue::default());
    static THUMBNAIL_CACHE: RefCell<ThumbnailCache> = RefCell::new(ThumbnailCache::default());
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThumbnailPriority {
    High,
    Low,
}

#[derive(Default)]
struct ThumbnailQueue {
    running: usize,
    queued: VecDeque<ThumbnailKey>,
    low_priority: VecDeque<ThumbnailKey>,
}

impl ThumbnailQueue {
    #[cfg(test)]
    fn enqueue(&mut self, key: ThumbnailKey) -> bool {
        self.enqueue_with_priority(key, ThumbnailPriority::High)
    }

    fn enqueue_with_priority(&mut self, key: ThumbnailKey, priority: ThumbnailPriority) -> bool {
        match priority {
            ThumbnailPriority::High => {
                if self.queued.contains(&key) {
                    return true;
                }
                if let Some(pos) = self
                    .low_priority
                    .iter()
                    .position(|candidate| candidate == &key)
                {
                    if let Some(key) = self.low_priority.remove(pos) {
                        self.queued.push_back(key);
                    }
                    return true;
                }
                if self.queued.len() >= MAX_QUEUED_THUMBNAILS {
                    return false;
                }
                self.queued.push_back(key);
                true
            }
            ThumbnailPriority::Low => {
                if self.queued.contains(&key) || self.low_priority.contains(&key) {
                    return true;
                }
                if self.low_priority.len() >= MAX_LOOKAHEAD_ITEMS {
                    return false;
                }
                if self.queued.len() + self.low_priority.len() >= MAX_QUEUED_THUMBNAILS {
                    return false;
                }
                self.low_priority.push_back(key);
                true
            }
        }
    }

    fn promote(&mut self, key: &ThumbnailKey) {
        if let Some(pos) = self
            .low_priority
            .iter()
            .position(|candidate| candidate == key)
            && let Some(key) = self.low_priority.remove(pos)
        {
            self.queued.push_back(key);
        }
    }

    fn begin_next(&mut self) -> Option<ThumbnailKey> {
        if self.running >= MAX_THUMBNAIL_WORKERS {
            return None;
        }
        if let Some(key) = self.queued.pop_front() {
            self.running += 1;
            return Some(key);
        }
        if let Some(key) = self.low_priority.pop_front() {
            self.running += 1;
            return Some(key);
        }
        None
    }

    fn finish(&mut self) {
        self.running = self.running.saturating_sub(1);
    }

    fn cancel(&mut self, key: &ThumbnailKey) {
        self.queued.retain(|queued| queued != key);
        self.low_priority.retain(|queued| queued != key);
    }

    fn clear_low_priority(&mut self) -> Vec<ThumbnailKey> {
        self.low_priority.drain(..).collect()
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
    set_thumbnail_for_path(
        image,
        path,
        known_metadata(&entry.modified_unix_seconds),
        known_metadata(&entry.size),
        fallback_icon,
        icon_size,
        thumbnail_size,
    );
}

pub(super) fn set_thumbnail_or_icon_for_path(
    image: &gtk::Image,
    path: &Path,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    set_thumbnail_for_path(
        image,
        path,
        None,
        None,
        fallback_icon,
        icon_size,
        thumbnail_size,
    );
}

pub(super) fn update_lookahead(entries: &[FileEntry], thumbnail_size: i32) {
    let thumbnail_size = thumbnail_size.clamp(16, 256);
    let pruned = THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().clear_low_priority());
    PENDING_THUMBNAILS.with(|pending| {
        let mut pending = pending.borrow_mut();
        for key in pruned {
            if pending
                .get(&key)
                .is_some_and(|item| item.targets.is_empty())
            {
                pending.remove(&key);
            }
        }
    });

    for entry in entries.iter().take(MAX_LOOKAHEAD_ITEMS) {
        let Some(path) = entry.location.native_path() else {
            continue;
        };
        let Some(kind) = thumbnail_kind(path) else {
            continue;
        };
        let key = ThumbnailKey {
            path: path.to_path_buf(),
            modified: known_metadata(&entry.modified_unix_seconds),
            file_size: known_metadata(&entry.size),
            thumbnail_size,
        };

        let is_cached = THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&key).is_some());
        if is_cached {
            continue;
        }

        PENDING_THUMBNAILS.with(|pending| {
            let mut pending = pending.borrow_mut();
            if pending.contains_key(&key) {
                return;
            }
            let queued = THUMBNAIL_QUEUE.with(|queue| {
                queue
                    .borrow_mut()
                    .enqueue_with_priority(key.clone(), ThumbnailPriority::Low)
            });
            if queued {
                pending.insert(
                    key.clone(),
                    PendingThumbnail {
                        id: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
                        kind,
                        cancellation: Cancellation::default(),
                        targets: Vec::new(),
                    },
                );
            }
        });
    }

    start_thumbnail_jobs();
}

pub(super) fn clear_lookahead() {
    let pruned = THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().clear_low_priority());
    PENDING_THUMBNAILS.with(|pending| {
        let mut pending = pending.borrow_mut();
        for key in pruned {
            if pending
                .get(&key)
                .is_some_and(|item| item.targets.is_empty())
            {
                pending.remove(&key);
            }
        }
        pending.retain(|_, item| {
            if item.targets.is_empty() {
                item.cancellation.cancel();
                false
            } else {
                true
            }
        });
    });
}

fn set_thumbnail_for_path(
    image: &gtk::Image,
    path: &Path,
    modified: Option<i64>,
    file_size: Option<u64>,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    let (image_id, request) = set_fallback_icon(image, fallback_icon, icon_size);
    let path = path.to_path_buf();
    let Some(kind) = thumbnail_kind(&path) else {
        return;
    };
    let thumbnail_size = thumbnail_size.clamp(16, 256);
    let key = ThumbnailKey {
        path: path.clone(),
        modified,
        file_size,
        thumbnail_size,
    };
    match THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&key)) {
        Some(CacheHit::Ready(bytes)) => {
            apply_thumbnail(image, &bytes, thumbnail_size);
            return;
        }
        Some(CacheHit::Failed) => return,
        None => {}
    }

    let weak_image = glib::WeakRef::new();
    weak_image.set(Some(image));
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(
            image_id,
            ActiveRequest {
                id: request,
                image: weak_image.clone(),
                deferred: None,
            },
        );
    });
    let target = PendingTarget {
        image_id,
        request,
        image: weak_image,
    };
    schedule_or_defer(key, kind, target);
}

fn schedule_or_defer(key: ThumbnailKey, kind: ThumbnailKind, target: PendingTarget) {
    let image_id = target.image_id;
    let request = target.request;
    if schedule_thumbnail(key.clone(), kind, target) {
        start_thumbnail_jobs();
    } else {
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
}

fn schedule_thumbnail(key: ThumbnailKey, kind: ThumbnailKind, target: PendingTarget) -> bool {
    PENDING_THUMBNAILS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if let Some(pending) = pending.get_mut(&key) {
            pending.targets.push(target);
            THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().promote(&key));
            true
        } else {
            let queued = THUMBNAIL_QUEUE.with(|queue| {
                queue
                    .borrow_mut()
                    .enqueue_with_priority(key.clone(), ThumbnailPriority::High)
            });
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
        glib::MainContext::default().spawn_local(run_thumbnail_job(job));
    }
}

async fn run_thumbnail_job(job: ThumbnailJob) {
    let job_id = job.id;
    let key = job.key.clone();
    let thumbnail_size = key.thumbnail_size;
    let result = gio::spawn_blocking(move || {
        render_thumbnail(
            &job.key.path,
            job.kind,
            job.key.thumbnail_size,
            &job.cancellation,
        )
    })
    .await;
    let targets = take_pending_targets(&key, job_id);
    THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().finish());

    if let Some(targets) = targets {
        match result {
            Ok(Ok(png)) => {
                let bytes = glib::Bytes::from_owned(png);
                THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert(key, bytes.clone()));
                finish_thumbnail_targets(targets, Some(&bytes), thumbnail_size);
            }
            Ok(Err(_)) | Err(_) => {
                THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert_failure(key));
                finish_thumbnail_targets(targets, None, thumbnail_size);
            }
        }
    }
    start_thumbnail_jobs();
    retry_deferred_thumbnails();
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
            continue;
        }
        let Some(bytes) = bytes else {
            continue;
        };
        let Some(image) = target.image.upgrade() else {
            continue;
        };
        apply_thumbnail(&image, bytes, thumbnail_size);
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
        image.set_pixel_size(thumbnail_size);
        image.set_size_request(thumbnail_size, thumbnail_size);
        image.set_paintable(Some(&texture));
        image.set_opacity(1.0);
    }
}

pub(super) fn show_fallback_icon(image: &gtk::Image, icon: &str, size: i32) {
    set_fallback_icon(image, icon, size);
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

fn set_fallback_icon(image: &gtk::Image, icon: &str, size: i32) -> (usize, u64) {
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let image_id = image.as_ptr() as usize;
    cancel_thumbnail(image_id);
    image.set_pixel_size(size);
    image.set_size_request(size, size);
    crate::assets::set_primary_icon(image, icon);
    (image_id, request)
}

fn cancel_thumbnail(image_id: usize) {
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().remove(&image_id);
    });
    let cancelled = PENDING_THUMBNAILS.with(|pending| {
        let mut pending = pending.borrow_mut();
        let mut cancelled = Vec::new();
        pending.retain(|key, thumbnail| {
            let had_targets = !thumbnail.targets.is_empty();
            thumbnail
                .targets
                .retain(|target| target.image_id != image_id);
            if had_targets && thumbnail.targets.is_empty() {
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

// SPDX-License-Identifier: MIT

//! Browser-only sandbox supervisors. Preview sessions deliberately remain independent.

use std::{
    collections::VecDeque,
    fs::File,
    io,
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use super::{Cancellation, ParseOperation, metadata::MediaMetadata};
use crate::services::ModelFormat;
use wire::{Operation, Response};

mod process;
pub(crate) mod wire;
mod worker;
pub use worker::run;

const CACHE_ENTRIES: usize = 64;
const THUMB_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;
const PREVIEW_CACHE_MAX_BYTES: usize = 16 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(30);
const WAIT_QUANTUM: Duration = Duration::from_millis(20);
pub const MAX_WORKERS: usize = 16;
const DEFAULT_WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub fn default_worker_limit() -> usize {
    let default = std::thread::available_parallelism().map_or(2, |n| n.get().min(4));
    configured_limit(
        std::env::var("STRATA_THUMBNAIL_WORKERS").ok().as_deref(),
        default,
    )
}

pub fn worker_limit() -> usize {
    pool().limit.load(Ordering::Relaxed)
}

pub fn set_worker_limit(limit: usize) {
    pool().set_limit(limit);
    preview_pool().set_limit(limit);
    if let Some(Ok(launcher)) = LAUNCHER.get() {
        let _ = launcher.send(LauncherMessage::Idle);
    }
}

fn configured_limit(value: Option<&str>, default: usize) -> usize {
    value
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
        .clamp(1, MAX_WORKERS)
}

fn configured_idle_timeout(value: Option<&str>) -> Duration {
    Duration::from_secs(
        value
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .unwrap_or(DEFAULT_WORKER_IDLE_TIMEOUT.as_secs())
            .min(86_400),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileKey {
    path: PathBuf,
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl FileKey {
    fn read(path: &Path, file: &File) -> io::Result<Self> {
        let stat = file.metadata()?;
        if !stat.is_file() {
            return Err(io::Error::other("Browser input is not a regular file"));
        }
        Ok(Self {
            path: path.to_path_buf(),
            device: stat.dev(),
            inode: stat.ino(),
            size: stat.len(),
            modified: (stat.mtime(), stat.mtime_nsec()),
            changed: (stat.ctime(), stat.ctime_nsec()),
        })
    }
}

#[derive(Default)]
struct Cached {
    png: Option<Vec<u8>>,
    metadata: Option<MediaMetadata>,
    failed_thumbnail: bool,
    failed_metadata: bool,
    completed: Option<Instant>,
}

// A source version may have distinct render sizes; keep operations separate.
struct CacheSlot {
    key: (FileKey, Operation),
    entry: Arc<Mutex<Cached>>,
    bytes: usize,
    completed: Option<Instant>,
}

struct Cache {
    entries: VecDeque<CacheSlot>,
    byte_count: usize,
    max_bytes: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(THUMB_CACHE_MAX_BYTES)
    }
}

impl Cache {
    fn new(max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            byte_count: 0,
            max_bytes,
        }
    }

    fn sweep_expired(&mut self, now: Instant) {
        let mut retained_bytes = 0usize;
        self.entries.retain(|slot| {
            let expired = Arc::strong_count(&slot.entry) == 1
                && slot
                    .completed
                    .is_some_and(|at| now.saturating_duration_since(at) >= CACHE_TTL);
            if !expired {
                retained_bytes = retained_bytes.saturating_add(slot.bytes);
            }
            !expired
        });
        self.byte_count = retained_bytes;
    }

    fn evict_one_idle(&mut self) -> bool {
        let Some(index) = self
            .entries
            .iter()
            .position(|slot| Arc::strong_count(&slot.entry) == 1)
        else {
            return false;
        };
        if let Some(removed) = self.entries.remove(index) {
            self.byte_count = self.byte_count.saturating_sub(removed.bytes);
        }
        true
    }

    fn evict_to_budget(&mut self) {
        while self.entries.len() > CACHE_ENTRIES || self.byte_count > self.max_bytes {
            if !self.evict_one_idle() {
                break;
            }
        }
    }

    fn acquire_entry(&mut self, key: (FileKey, Operation), now: Instant) -> Arc<Mutex<Cached>> {
        self.sweep_expired(now);
        self.evict_to_budget();
        if let Some(index) = self.entries.iter().position(|slot| slot.key == key) {
            let slot = self.entries.remove(index).expect("existing cache entry");
            let result = slot.entry.clone();
            self.entries.push_back(slot);
            return result;
        }
        let result = Arc::new(Mutex::new(Cached::default()));
        // Never evict an in-flight gate: duplicate callers must share its work.
        if self.entries.len() >= CACHE_ENTRIES && !self.evict_one_idle() {
            return result;
        }
        self.entries.push_back(CacheSlot {
            key,
            entry: result.clone(),
            bytes: 0,
            completed: None,
        });
        result
    }

    fn record_usage(&mut self, entry: &Arc<Mutex<Cached>>, bytes: usize, completed: Instant) {
        self.sweep_expired(completed);
        let Some(index) = self
            .entries
            .iter()
            .position(|slot| Arc::ptr_eq(&slot.entry, entry))
        else {
            return;
        };
        if bytes > self.max_bytes {
            if let Some(removed) = self.entries.remove(index) {
                self.byte_count = self.byte_count.saturating_sub(removed.bytes);
            }
            return;
        }
        let slot = &mut self.entries[index];
        self.byte_count = self
            .byte_count
            .saturating_sub(slot.bytes)
            .saturating_add(bytes);
        slot.bytes = bytes;
        slot.completed = Some(completed);
        self.evict_to_budget();
    }

    fn clear(&mut self) {
        let mut retained_bytes = 0usize;
        self.entries.retain(|slot| {
            let in_flight = Arc::strong_count(&slot.entry) > 1;
            if in_flight {
                retained_bytes = retained_bytes.saturating_add(slot.bytes);
            }
            in_flight
        });
        self.byte_count = retained_bytes;
        if self.entries.is_empty() {
            self.entries.shrink_to_fit();
        }
    }
}

fn acquire_cache_entry(
    cache: &Mutex<Cache>,
    key: FileKey,
    operation: Operation,
) -> (Arc<Mutex<Cached>>, usize) {
    let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
    let entry = cache.acquire_entry((key, operation), Instant::now());
    (entry, cache.max_bytes)
}

#[cfg(test)]
fn cache_entry(cache: &Mutex<Cache>, key: FileKey, operation: Operation) -> Arc<Mutex<Cached>> {
    acquire_cache_entry(cache, key, operation).0
}

pub fn clear_preview_cache() {
    preview_pool().clear_cache();
}

pub struct Thumbnail {
    pub png: Vec<u8>,
    pub metadata: Option<MediaMetadata>,
}

pub fn thumbnail(
    path: &Path,
    operation: ParseOperation,
    cancellation: &Cancellation,
) -> Result<Thumbnail, String> {
    if matches!(operation, ParseOperation::ThumbnailCover(_)) {
        // Image loaders launch glycin, which cannot nest inside the persistent worker's sandbox.
        let png = super::parse(
            path,
            operation,
            256,
            super::MediaPreviewBackend::Software,
            cancellation,
        )?
        .data;
        return Ok(Thumbnail {
            png,
            metadata: None,
        });
    }
    let operation = match operation {
        ParseOperation::ThumbnailImage => Operation::Image,
        ParseOperation::ThumbnailRaw => Operation::Raw,
        ParseOperation::ThumbnailPdf => Operation::Pdf,
        ParseOperation::ThumbnailVideo => Operation::Video,
        ParseOperation::ThumbnailModel(ModelFormat::ThreeMf) => Operation::ThreeMfThumbnail,
        ParseOperation::ThumbnailModel(ModelFormat::FreeCad) => Operation::FreeCadThumbnail,
        ParseOperation::ThumbnailModel(ModelFormat::Gcode) => Operation::GcodeThumbnail,
        _ => return Err("Not a browser thumbnail operation".into()),
    };
    let result = request(pool(), path, operation, cancellation)?;
    Ok(Thumbnail {
        png: result.png.ok_or("Thumbnail unavailable")?,
        metadata: result.metadata,
    })
}

pub fn metadata(
    path: &Path,
    image: bool,
    cancellation: &Cancellation,
) -> Result<MediaMetadata, String> {
    let operation = if super::raw_metadata::is_raw(path) {
        Operation::RawMetadata
    } else if image {
        Operation::ImageMetadata
    } else {
        Operation::MediaMetadata
    };
    request(pool(), path, operation, cancellation)?
        .metadata
        .ok_or_else(|| "Media details unavailable".into())
}

pub fn preview(
    path: &Path,
    operation: &ParseOperation,
    cancellation: &Cancellation,
) -> Option<Result<Vec<u8>, String>> {
    let operation = match operation {
        ParseOperation::PreviewImage | ParseOperation::DocumentImage => Operation::PreviewImage,
        ParseOperation::PreviewRaw => Operation::PreviewRaw,
        ParseOperation::DocumentMermaid => Operation::DocumentMermaid,
        ParseOperation::DocumentMath { display: true } => Operation::DocumentMath,
        ParseOperation::DocumentMath { display: false } => Operation::DocumentMathInline,
        _ => return None,
    };
    if !workers_supported() {
        return None;
    }
    Some(
        request(preview_pool(), path, operation, cancellation)
            .and_then(|parts| parts.png.ok_or_else(|| "Preview unavailable".to_owned())),
    )
}

fn invalid_output_label(operation: Operation) -> &'static str {
    match operation {
        Operation::PreviewImage
        | Operation::PreviewRaw
        | Operation::DocumentMermaid
        | Operation::DocumentMath
        | Operation::DocumentMathInline => "Invalid preview render",
        _ => "Invalid browser thumbnail",
    }
}

fn parse_operation(operation: Operation) -> ParseOperation {
    match operation {
        Operation::Image => ParseOperation::ThumbnailImage,
        Operation::Raw => ParseOperation::ThumbnailRaw,
        Operation::Pdf => ParseOperation::ThumbnailPdf,
        Operation::Video => ParseOperation::ThumbnailVideo,
        Operation::ThreeMfThumbnail => ParseOperation::ThumbnailModel(ModelFormat::ThreeMf),
        Operation::FreeCadThumbnail => ParseOperation::ThumbnailModel(ModelFormat::FreeCad),
        Operation::GcodeThumbnail => ParseOperation::ThumbnailModel(ModelFormat::Gcode),
        Operation::ImageMetadata | Operation::MediaMetadata => ParseOperation::MediaMetadata,
        Operation::RawMetadata => ParseOperation::RawMetadata,
        Operation::PreviewImage => ParseOperation::PreviewImage,
        Operation::PreviewRaw => ParseOperation::PreviewRaw,
        Operation::DocumentMermaid => ParseOperation::DocumentMermaid,
        Operation::DocumentMath => ParseOperation::DocumentMath { display: true },
        Operation::DocumentMathInline => ParseOperation::DocumentMath { display: false },
    }
}

struct ResultParts {
    png: Option<Vec<u8>>,
    metadata: Option<MediaMetadata>,
}

fn request(
    pool: &Pool,
    path: &Path,
    operation: Operation,
    cancellation: &Cancellation,
) -> Result<ResultParts, String> {
    if cancellation.is_cancelled() {
        return Err("Browser request cancelled".into());
    }
    let real = path
        .canonicalize()
        .map_err(|e| crate::error_text::io_error_message(&e))?;
    let stop = cancellation.clone();
    let hold = crate::services::file_activity::hold([path, &real], move || stop.cancel())?;
    hold.settle(request_held(pool, path, operation, cancellation))
}

struct RenderedResponse {
    png: Option<Vec<u8>>,
    metadata: Option<MediaMetadata>,
    payload_bytes: usize,
}

fn lock_cached<'a>(
    entry: &'a Mutex<Cached>,
    cancellation: &Cancellation,
) -> Result<std::sync::MutexGuard<'a, Cached>, String> {
    loop {
        if cancellation.is_cancelled() {
            return Err("Browser request cancelled".into());
        }
        match entry.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(p)) => return Ok(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(WAIT_QUANTUM),
        }
    }
}

fn decode_metadata(operation: Operation, raw: &[u8]) -> Result<MediaMetadata, String> {
    if operation == Operation::RawMetadata {
        return super::raw_metadata::RawMetadata::from_json(raw).map(|raw| MediaMetadata {
            dimensions: raw.dimensions,
            ..MediaMetadata::default()
        });
    }
    let image = matches!(
        operation,
        Operation::Image | Operation::Raw | Operation::ImageMetadata
    );
    MediaMetadata::from_json(raw, image)
}

fn execute_browser_request(
    pool: &Pool,
    path: &Path,
    file: &File,
    key: &FileKey,
    operation: Operation,
    cancellation: &Cancellation,
) -> Result<RenderedResponse, String> {
    let queued = Instant::now();
    let mut lease = pool.acquire(operation, cancellation)?;
    let queue_ms = queued.elapsed().as_millis() as u64;
    if cancellation.is_cancelled() {
        return Err("Browser request cancelled".into());
    }
    let started = Instant::now();
    let response = lease.execute(file, operation, cancellation)?;
    if FileKey::read(path, file).map_err(|e| e.to_string())? != *key {
        return Err("Browser input changed while rendering".into());
    }
    let payload_bytes = response.png.len().saturating_add(response.metadata.len());
    let png = if response.png.is_empty() {
        None
    } else if response.png.len() as u64 > wire::MAX_OUTPUT_BYTES
        || !super::valid_output(parse_operation(operation), &response.png)
    {
        lease.discard();
        return Err(invalid_output_label(operation).into());
    } else {
        Some(response.png)
    };
    let metadata = if response.metadata.is_empty() {
        None
    } else {
        match decode_metadata(operation, &response.metadata) {
            Ok(metadata) => Some(metadata),
            Err(error) => {
                lease.discard();
                return Err(error);
            }
        }
    };
    tracing::debug!(
        ?operation,
        queue_ms,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "browser worker completed"
    );
    Ok(RenderedResponse {
        png,
        metadata,
        payload_bytes,
    })
}

fn store_cached(
    cached: &mut Cached,
    rendered: RenderedResponse,
    metadata_only: bool,
    completed: Instant,
) {
    if let Some(png) = rendered.png {
        cached.png = Some(png);
    } else if !metadata_only {
        cached.failed_thumbnail = true;
    }
    if let Some(metadata) = rendered.metadata {
        cached.metadata = Some(metadata);
    } else if metadata_only {
        cached.failed_metadata = true;
    }
    cached.completed = Some(completed);
}

fn check_input_size(operation: Operation, size: u64, metadata_only: bool) -> Result<(), String> {
    // The thumbnail is in a bounded header or block; the toolpath is not read.
    if metadata_only || operation == Operation::Video || operation == Operation::GcodeThumbnail {
        return Ok(());
    }
    let input_limit = if matches!(
        operation,
        Operation::ThreeMfThumbnail | Operation::FreeCadThumbnail
    ) {
        crate::services::model_preview::MAX_MODEL_INPUT_BYTES
    } else {
        super::MAX_RASTER_INPUT_BYTES
    };
    if size > input_limit {
        return Err("Browser input exceeds the supported size limit".into());
    }
    Ok(())
}

fn request_held(
    pool: &Pool,
    path: &Path,
    operation: Operation,
    cancellation: &Cancellation,
) -> Result<ResultParts, String> {
    let file = open_source(path).map_err(|e| crate::error_text::io_error_message(&e))?;
    let key = FileKey::read(path, &file).map_err(|e| crate::error_text::io_error_message(&e))?;
    let metadata_only = matches!(
        operation,
        Operation::ImageMetadata | Operation::MediaMetadata | Operation::RawMetadata
    );
    check_input_size(operation, key.size, metadata_only)?;
    let (entry, max_cache_bytes) = acquire_cache_entry(&pool.cache, key.clone(), operation);
    let mut cached = lock_cached(&entry, cancellation)?;
    if cached
        .completed
        .is_some_and(|time| time.elapsed() >= CACHE_TTL)
    {
        *cached = Cached::default();
    }
    let hit = if metadata_only {
        cached.metadata.is_some() || cached.failed_metadata
    } else {
        cached.png.is_some() || cached.failed_thumbnail
    };
    let mut oversized_parts = None;
    let mut usage = None;
    if !hit {
        let rendered = execute_browser_request(pool, path, &file, &key, operation, cancellation)?;
        let completed = Instant::now();
        let payload_bytes = rendered.payload_bytes;
        if payload_bytes <= max_cache_bytes {
            store_cached(&mut cached, rendered, metadata_only, completed);
        } else {
            oversized_parts = Some(ResultParts {
                png: (!metadata_only).then_some(rendered.png).flatten(),
                metadata: rendered.metadata,
            });
        }
        usage = Some((payload_bytes, completed));
    }
    let result = if cancellation.is_cancelled() {
        Err("Browser request cancelled".into())
    } else {
        Ok(oversized_parts.unwrap_or_else(|| ResultParts {
            png: (!metadata_only).then(|| cached.png.clone()).flatten(),
            metadata: cached.metadata.clone(),
        }))
    };
    drop(cached);
    if let Some((payload_bytes, completed)) = usage {
        pool.cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .record_usage(&entry, payload_bytes, completed);
    }
    result
}

fn open_source(path: &Path) -> io::Result<File> {
    use rustix::fs::{FileType, Mode, OFlags, fstat, open};
    use std::os::fd::AsRawFd;
    // O_PATH inspects special files without opening a device or blocking on a
    // FIFO. Reopening this descriptor, not the pathname, closes the type race.
    let source = open(path, OFlags::PATH | OFlags::CLOEXEC, Mode::empty())?;
    if FileType::from_raw_mode(fstat(&source)?.st_mode) != FileType::RegularFile {
        return Err(io::Error::other("Browser input is not a regular file"));
    }
    Ok(File::from(open(
        format!("/proc/self/fd/{}", source.as_raw_fd()),
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )?))
}

struct Pool {
    state: Mutex<PoolState>,
    changed: Condvar,
    limit: AtomicUsize,
    idle_timeout: Duration,
    cache: Mutex<Cache>,
}

struct IdleWorker {
    worker: Worker,
    since: Instant,
}

#[derive(Default)]
struct PoolState {
    idle: Vec<IdleWorker>,
    count: usize,
    slow_running: usize,
    thumbnail_waiters: usize,
    metadata_waiters: usize,
    metadata_running: usize,
    thumbnail_streak: usize,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        state: Mutex::default(),
        changed: Condvar::new(),
        limit: AtomicUsize::new(default_worker_limit()),
        idle_timeout: configured_idle_timeout(
            std::env::var("STRATA_THUMBNAIL_IDLE_SECONDS")
                .ok()
                .as_deref(),
        ),
        cache: Mutex::new(Cache::new(THUMB_CACHE_MAX_BYTES)),
    })
}

// Preview renders share the worker implementation but not the thumbnail pool:
// a scrolled directory flood must not delay an interactive Space preview.
fn preview_pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        state: Mutex::default(),
        changed: Condvar::new(),
        limit: AtomicUsize::new(default_worker_limit()),
        idle_timeout: configured_idle_timeout(
            std::env::var("STRATA_THUMBNAIL_IDLE_SECONDS")
                .ok()
                .as_deref(),
        ),
        cache: Mutex::new(Cache::new(PREVIEW_CACHE_MAX_BYTES)),
    })
}

static NAMESPACES_RESTRICTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerSupport {
    Available,
    LandlockUnavailable,
    /// Discovered when a supervisor starts; it holds for the rest of the process.
    NamespacesRestricted,
}

pub fn worker_support() -> WorkerSupport {
    static LANDLOCK_SUPPORTED: OnceLock<bool> = OnceLock::new();
    let landlock = *LANDLOCK_SUPPORTED.get_or_init(|| {
        let supported = worker::supported();
        if !supported {
            tracing::warn!("Landlock ABI 3 unavailable; retaining one-shot sandboxes");
        }
        supported
    });
    if !landlock {
        WorkerSupport::LandlockUnavailable
    } else if NAMESPACES_RESTRICTED.load(Ordering::Relaxed) {
        WorkerSupport::NamespacesRestricted
    } else {
        WorkerSupport::Available
    }
}

pub fn workers_supported() -> bool {
    worker_support() == WorkerSupport::Available
}

impl Pool {
    fn clear_cache(&self) {
        self.cache.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    fn set_limit(&self, limit: usize) {
        let _state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        self.limit
            .store(limit.clamp(1, MAX_WORKERS), Ordering::Relaxed);
        self.changed.notify_all();
    }

    fn next_expiration(&self, now: Instant) -> Option<Duration> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .idle
            .iter()
            .map(|idle| {
                self.idle_timeout
                    .saturating_sub(now.saturating_duration_since(idle.since))
            })
            .min()
    }

    fn retire_idle(&self, now: Instant) -> usize {
        let expired = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let mut excess = state
                .count
                .saturating_sub(self.limit.load(Ordering::Relaxed));
            state
                .idle
                .extract_if(.., |idle| {
                    if excess > 0 || now.saturating_duration_since(idle.since) >= self.idle_timeout
                    {
                        excess = excess.saturating_sub(1);
                        true
                    } else {
                        false
                    }
                })
                .collect::<Vec<_>>()
        };
        let count = expired.len();
        if count == 0 {
            return 0;
        }
        // Teardown never holds the pool lock. Retiring processes still count
        // against admission until they have actually been stopped and reaped.
        for idle in expired {
            let pid = match &idle.worker {
                Worker::Persistent(worker) => Some(worker.child.id()),
                Worker::OneShot => None,
            };
            drop(idle.worker);
            if let Some(pid) = pid {
                tracing::debug!(pid, "idle browser sandbox retired");
            }
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.count -= count;
        self.changed.notify_all();
        count
    }

    fn acquire(
        &self,
        operation: Operation,
        cancellation: &Cancellation,
    ) -> Result<Lease<'_>, String> {
        let metadata = matches!(
            operation,
            Operation::ImageMetadata | Operation::MediaMetadata | Operation::RawMetadata
        );
        let slow = matches!(
            operation,
            Operation::Raw
                | Operation::Pdf
                | Operation::Video
                | Operation::ThreeMfThumbnail
                | Operation::FreeCadThumbnail
                | Operation::GcodeThumbnail
                | Operation::ImageMetadata
                | Operation::MediaMetadata
                | Operation::RawMetadata
        );
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if metadata {
            state.metadata_waiters += 1;
        } else {
            state.thumbnail_waiters += 1;
        }
        loop {
            if cancellation.is_cancelled() {
                if metadata {
                    state.metadata_waiters -= 1;
                } else {
                    state.thumbnail_waiters -= 1;
                }
                self.changed.notify_all();
                return Err("Browser request cancelled".into());
            }
            let limit = self.limit.load(Ordering::Relaxed);
            let slow_available = state.slow_running < limit.saturating_sub(1).max(1);
            // At most one probe competes with thumbnails; continuous scrolling still
            // yields a turn after four thumbnail admissions, without reserving an idle worker.
            let metadata_turn = state.metadata_waiters > 0
                && state.metadata_running == 0
                && slow_available
                && (state.thumbnail_waiters == 0 || state.thumbnail_streak >= 4);
            let admitted = if metadata {
                metadata_turn
            } else {
                !metadata_turn && (!slow || slow_available)
            };
            if admitted
                && state.count.saturating_sub(state.idle.len()) < limit
                && (!state.idle.is_empty() || state.count < limit)
            {
                if metadata {
                    state.metadata_waiters -= 1;
                    state.metadata_running += 1;
                    state.thumbnail_streak = 0;
                } else {
                    state.thumbnail_waiters -= 1;
                    state.thumbnail_streak = (state.thumbnail_streak + 1).min(4);
                }
                if slow {
                    state.slow_running += 1;
                }
                let worker = state.idle.pop().map(|idle| idle.worker);
                if worker.is_none() {
                    state.count += 1;
                }
                drop(state);
                let mut lease = Lease {
                    pool: self,
                    worker,
                    slow,
                    metadata,
                };
                if lease.worker.is_none() {
                    lease.worker = Some(Worker::spawn().map_err(|e| e.to_string())?);
                }
                return Ok(lease);
            }
            state = self
                .changed
                .wait_timeout(state, WAIT_QUANTUM)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}

struct Lease<'a> {
    pool: &'a Pool,
    worker: Option<Worker>,
    slow: bool,
    metadata: bool,
}

impl Lease<'_> {
    fn execute(
        &mut self,
        file: &File,
        operation: Operation,
        cancellation: &Cancellation,
    ) -> Result<Response, String> {
        let mut result = self
            .worker
            .as_mut()
            .ok_or("Missing browser worker")?
            .execute(file, operation, cancellation);
        if result.as_ref().is_err_and(|error| {
            matches!(
                error.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
            )
        }) {
            self.discard();
            self.worker = Some(Worker::spawn().map_err(|e| e.to_string())?);
            result = self.worker.as_mut().expect("replacement worker").execute(
                file,
                operation,
                cancellation,
            );
        }
        if result.is_err() {
            self.discard();
        }
        result.map_err(|e| e.to_string())
    }

    fn discard(&mut self) {
        self.worker.take();
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let mut state = self.pool.state.lock().unwrap_or_else(|p| p.into_inner());
        let wake_launcher = state.idle.is_empty();
        if let Some(worker) = self.worker.take() {
            state.idle.push(IdleWorker {
                worker,
                since: Instant::now(),
            });
        } else {
            state.count = state.count.saturating_sub(1);
        }
        if self.slow {
            state.slow_running = state.slow_running.saturating_sub(1);
        }
        if self.metadata {
            state.metadata_running = state.metadata_running.saturating_sub(1);
        }
        self.pool.changed.notify_all();
        drop(state);
        if wake_launcher && let Some(Ok(launcher)) = LAUNCHER.get() {
            let _ = launcher.send(LauncherMessage::Idle);
        }
    }
}

enum Worker {
    Persistent(ProcessWorker),
    OneShot,
}

impl Worker {
    fn spawn() -> io::Result<Self> {
        if !workers_supported() {
            return Ok(Self::OneShot);
        }
        Ok(match ProcessWorker::spawn()? {
            Startup::Ready(worker) => Self::Persistent(worker),
            Startup::NamespacesRestricted => {
                if !NAMESPACES_RESTRICTED.swap(true, Ordering::Relaxed) {
                    tracing::warn!("Job namespaces restricted; retaining one-shot sandboxes");
                }
                Self::OneShot
            }
        })
    }

    fn execute(
        &mut self,
        file: &File,
        operation: Operation,
        cancellation: &Cancellation,
    ) -> io::Result<Response> {
        match self {
            Self::Persistent(worker) => worker.execute(file, operation, cancellation),
            Self::OneShot => {
                use std::os::fd::AsRawFd;
                let operation = parse_operation(operation);
                let output = super::parse(
                    Path::new(&format!("/proc/self/fd/{}", file.as_raw_fd())),
                    operation.clone(),
                    256,
                    super::MediaPreviewBackend::Software,
                    cancellation,
                )
                .map_err(io::Error::other)?;
                Ok(
                    if matches!(
                        operation,
                        ParseOperation::MediaMetadata | ParseOperation::RawMetadata
                    ) {
                        Response {
                            png: Vec::new(),
                            metadata: output.data,
                        }
                    } else {
                        Response {
                            png: output.data,
                            metadata: Vec::new(),
                        }
                    },
                )
            }
        }
    }
}

struct ProcessWorker {
    child: super::Renderer,
    socket: UnixStream,
    _snapshot: super::PrivateOutput,
}

enum Startup {
    Ready(ProcessWorker),
    NamespacesRestricted,
}

#[derive(Debug, PartialEq)]
enum Readiness {
    Ready,
    NamespacesRestricted,
}

fn read_readiness(socket: &mut impl io::Read) -> io::Result<Readiness> {
    let mut byte = [0];
    socket.read_exact(&mut byte)?;
    match byte[0] {
        wire::READY => Ok(Readiness::Ready),
        wire::NAMESPACES_RESTRICTED => Ok(Readiness::NamespacesRestricted),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid browser worker handshake",
        )),
    }
}

enum LauncherMessage {
    Spawn(std::sync::mpsc::SyncSender<io::Result<Startup>>),
    Idle,
}

static LAUNCHER: OnceLock<Result<std::sync::mpsc::Sender<LauncherMessage>, String>> =
    OnceLock::new();

impl ProcessWorker {
    fn spawn() -> io::Result<Startup> {
        let launcher = LAUNCHER
            .get_or_init(|| {
                let (sender, receiver) = std::sync::mpsc::channel::<LauncherMessage>();
                // PR_SET_PDEATHSIG follows the spawning *thread*. Metadata's scoped
                // threads and GIO's expiring pool threads must not own bwrap's life.
                std::thread::Builder::new()
                    .name("thumbnail-launcher".into())
                    .spawn(move || {
                        use std::sync::mpsc::RecvTimeoutError;
                        loop {
                            let now = Instant::now();
                            pool().retire_idle(now);
                            preview_pool().retire_idle(now);
                            let expiration = [pool(), preview_pool()]
                                .into_iter()
                                .filter_map(|pool| pool.next_expiration(Instant::now()))
                                .min();
                            let message = match expiration {
                                Some(timeout) => receiver.recv_timeout(timeout),
                                None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
                            };
                            match message {
                                Ok(LauncherMessage::Spawn(reply)) => {
                                    let _ = reply.send(Self::spawn_on_launcher());
                                }
                                Ok(LauncherMessage::Idle) | Err(RecvTimeoutError::Timeout) => {}
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                    })
                    .map_err(|e| e.to_string())?;
                Ok(sender)
            })
            .as_ref()
            .map_err(|error| io::Error::other(error.clone()))?;
        let (reply, result) = std::sync::mpsc::sync_channel(1);
        launcher
            .send(LauncherMessage::Spawn(reply))
            .map_err(io::Error::other)?;
        result.recv().map_err(io::Error::other)?
    }

    fn spawn_on_launcher() -> io::Result<Startup> {
        let snapshot = super::PrivateOutput::create()?;
        let executable = super::resolve_renderer_executable(
            &std::env::current_exe()?,
            &PathBuf::from(format!("/proc/{}/exe", std::process::id())),
            snapshot.path(),
        )
        .map_err(io::Error::other)?;
        let (mut socket, child_socket) = UnixStream::pair()?;
        socket.set_read_timeout(Some(super::WALL_TIME_LIMIT))?;
        socket.set_write_timeout(Some(super::WALL_TIME_LIMIT))?;
        let bwrap = crate::trusted_command::resolve_unprivileged("bwrap")
            .map_err(|error| io::Error::other(super::sandbox_unavailable(&error)))?;
        let mut command = super::runtime_command(&bwrap, true);
        command
            .args(["--ro-bind"])
            .arg(executable)
            .arg("/app/strata")
            .args([
                "--setenv",
                "MALLOC_ARENA_MAX",
                "1",
                "--setenv",
                "OMP_NUM_THREADS",
                "1",
                "--setenv",
                "OPENBLAS_NUM_THREADS",
                "1",
                "--setenv",
                "MAGICK_THREAD_LIMIT",
                "1",
                "--setenv",
                "GLYCIN_DISABLE_SANDBOX",
                "i-know-the-risks",
                "--",
                "/app/strata",
                "--browser-worker",
            ])
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(child_socket)))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = super::spawn_renderer(&mut command)?;
        // The command still owns the supervisor's socket end, which would hide
        // an early exit until the read timeout.
        drop(command);
        match read_readiness(&mut socket) {
            Ok(Readiness::Ready) => {}
            Ok(Readiness::NamespacesRestricted) => {
                super::terminate(&mut child);
                return Ok(Startup::NamespacesRestricted);
            }
            Err(error) => {
                super::terminate(&mut child);
                return Err(io::Error::new(
                    error.kind(),
                    format!("Browser sandbox failed to start: {error}"),
                ));
            }
        }
        tracing::debug!(pid = child.id(), "browser sandbox started");
        Ok(Startup::Ready(Self {
            child,
            socket,
            _snapshot: snapshot,
        }))
    }

    fn execute(
        &mut self,
        file: &File,
        operation: Operation,
        cancellation: &Cancellation,
    ) -> io::Result<Response> {
        use io::Read;
        let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
        let deadline = Instant::now() + super::WALL_TIME_LIMIT;
        wire::send(&self.socket, file, &write, operation)?;
        drop(write);
        let mut output = File::from(read);
        let mut reader = DeadlineReader {
            reader: &mut output,
            deadline,
            cancellation,
        };
        let response = Response::read(&mut reader);
        if response
            .as_ref()
            .is_err_and(|error| error.kind() != io::ErrorKind::UnexpectedEof)
        {
            return response;
        }
        let mut status = [0];
        DeadlineReader {
            reader: &mut self.socket,
            deadline,
            cancellation,
        }
        .read_exact(&mut status)?;
        match status[0] {
            0 => Ok(Response::default()),
            1 => {
                let response = response?;
                if reader.read(&mut [0])? != 0 {
                    return Err(io::Error::other("Trailing decoder output"));
                }
                Ok(response)
            }
            _ => Err(io::Error::other("Invalid browser completion")),
        }
    }
}

struct DeadlineReader<'a, R> {
    reader: &'a mut R,
    deadline: Instant,
    cancellation: &'a Cancellation,
}
impl<R: io::Read + std::os::fd::AsFd> io::Read for DeadlineReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            if self.cancellation.is_cancelled() {
                return Err(io::Error::other("Browser request cancelled"));
            }
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Browser renderer timed out",
                ));
            }
            let wait = remaining.min(WAIT_QUANTUM);
            let timeout = Timespec {
                tv_sec: wait.as_secs() as i64,
                tv_nsec: i64::from(wait.subsec_nanos()),
            };
            let mut fds = [PollFd::new(&*self.reader, PollFlags::IN)];
            match poll(&mut fds, Some(&timeout)) {
                Ok(0) => continue,
                Ok(_) => return self.reader.read(bytes),
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Drop for ProcessWorker {
    fn drop(&mut self) {
        super::terminate(&mut self.child);
    }
}

#[cfg(test)]
mod tests;

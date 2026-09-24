// SPDX-License-Identifier: MIT

//! Browser-only sandbox supervisors. Preview sessions deliberately remain independent.

use std::{
    collections::VecDeque,
    fs::File,
    io,
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use super::{Cancellation, ParseOperation, metadata::MediaMetadata};
use wire::{Operation, Response};

mod process;
pub(crate) mod wire;
mod worker;
pub(crate) use worker::run;

const CACHE_ENTRIES: usize = 64;
const CACHE_TTL: Duration = Duration::from_secs(30);
const WAIT_QUANTUM: Duration = Duration::from_millis(20);
pub(crate) const MAX_WORKERS: usize = 16;
const DEFAULT_WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) fn default_worker_limit() -> usize {
    let default = std::thread::available_parallelism().map_or(2, |n| n.get().min(4));
    configured_limit(
        std::env::var("STRATA_THUMBNAIL_WORKERS").ok().as_deref(),
        default,
    )
}

pub(crate) fn worker_limit() -> usize {
    pool().limit.load(Ordering::Relaxed)
}

pub(crate) fn set_worker_limit(limit: usize) {
    pool().set_limit(limit);
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

// The operation distinguishes render sizes: thumbnails and previews of one file
// produce different output and must not share an entry.
type Cache = VecDeque<((FileKey, Operation), Arc<Mutex<Cached>>)>;

fn cache_entry(key: FileKey, operation: Operation) -> Arc<Mutex<Cached>> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let key = (key, operation);
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(index) = cache.iter().position(|(candidate, _)| candidate == &key) {
        let entry = cache.remove(index).expect("existing cache entry");
        let result = entry.1.clone();
        cache.push_back(entry);
        return result;
    }
    let result = Arc::new(Mutex::new(Cached::default()));
    // Never evict an in-flight gate: duplicate callers must share its work.
    if cache.len() >= CACHE_ENTRIES {
        if let Some(index) = cache
            .iter()
            .position(|(_, entry)| Arc::strong_count(entry) == 1)
        {
            cache.remove(index);
        } else {
            return result;
        }
    }
    cache.push_back((key, result.clone()));
    result
}

pub(crate) struct Thumbnail {
    pub(crate) png: Vec<u8>,
    pub(crate) metadata: Option<MediaMetadata>,
}

pub(crate) fn thumbnail(
    path: &Path,
    operation: ParseOperation,
    cancellation: &Cancellation,
) -> Result<Thumbnail, String> {
    let operation = match operation {
        ParseOperation::ThumbnailImage => Operation::Image,
        ParseOperation::ThumbnailRaw => Operation::Raw,
        ParseOperation::ThumbnailPdf => Operation::Pdf,
        ParseOperation::ThumbnailVideo => Operation::Video,
        _ => return Err("Not a browser thumbnail operation".into()),
    };
    let result = request(pool(), path, operation, cancellation)?;
    Ok(Thumbnail {
        png: result.png.ok_or("Thumbnail unavailable")?,
        metadata: result.metadata,
    })
}

pub(crate) fn metadata(
    path: &Path,
    image: bool,
    cancellation: &Cancellation,
) -> Result<MediaMetadata, String> {
    request(
        pool(),
        path,
        if image {
            Operation::ImageMetadata
        } else {
            Operation::MediaMetadata
        },
        cancellation,
    )?
    .metadata
    .ok_or_else(|| "Media details unavailable".into())
}

/// Quick previews and document media reuse the pooled workers instead of
/// spawning a one-shot sandbox per render. `None` keeps the caller's fallback.
pub(crate) fn preview(
    path: &Path,
    operation: &ParseOperation,
    cancellation: &Cancellation,
) -> Option<Result<Vec<u8>, String>> {
    let operation = match operation {
        ParseOperation::PreviewImage | ParseOperation::DocumentImage => Operation::PreviewImage,
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

fn parse_operation(operation: Operation) -> ParseOperation {
    match operation {
        Operation::Image => ParseOperation::ThumbnailImage,
        Operation::Raw => ParseOperation::ThumbnailRaw,
        Operation::Pdf => ParseOperation::ThumbnailPdf,
        Operation::Video => ParseOperation::ThumbnailVideo,
        Operation::ImageMetadata | Operation::MediaMetadata => ParseOperation::MediaMetadata,
        Operation::PreviewImage => ParseOperation::PreviewImage,
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
    let file = open_source(path).map_err(|e| e.to_string())?;
    let key = FileKey::read(path, &file).map_err(|e| e.to_string())?;
    let metadata_only = matches!(
        operation,
        Operation::ImageMetadata | Operation::MediaMetadata
    );
    if !metadata_only && operation != Operation::Video && key.size > super::MAX_RASTER_INPUT_BYTES {
        return Err("Browser input exceeds the supported size limit".into());
    }
    let entry = cache_entry(key.clone(), operation);
    let mut cached = loop {
        if cancellation.is_cancelled() {
            return Err("Browser request cancelled".into());
        }
        match entry.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(p)) => break p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(WAIT_QUANTUM),
        }
    };
    if cached
        .completed
        .is_some_and(|time| time.elapsed() > CACHE_TTL)
    {
        *cached = Cached::default();
    }
    let hit = if metadata_only {
        cached.metadata.is_some() || cached.failed_metadata
    } else {
        cached.png.is_some() || cached.failed_thumbnail
    };
    if !hit {
        let queued = Instant::now();
        let mut lease = pool.acquire(operation, cancellation)?;
        let queue_ms = queued.elapsed().as_millis() as u64;
        if cancellation.is_cancelled() {
            return Err("Browser request cancelled".into());
        }
        let started = Instant::now();
        let response = lease.execute(&file, operation)?;
        if FileKey::read(path, &file).map_err(|e| e.to_string())? != key {
            return Err("Browser input changed while rendering".into());
        }
        if !response.png.is_empty() {
            if response.png.len() as u64 > wire::MAX_OUTPUT_BYTES
                || !super::valid_output(parse_operation(operation), &response.png)
            {
                lease.discard();
                return Err("Invalid browser thumbnail".into());
            }
            cached.png = Some(response.png);
        } else if !metadata_only {
            cached.failed_thumbnail = true;
        }
        if !response.metadata.is_empty() {
            let image = matches!(
                operation,
                Operation::Image | Operation::Raw | Operation::ImageMetadata
            );
            match MediaMetadata::from_json(&response.metadata, image) {
                Ok(metadata) => cached.metadata = Some(metadata),
                Err(error) => {
                    lease.discard();
                    return Err(error);
                }
            }
        } else if metadata_only {
            cached.failed_metadata = true;
        }
        cached.completed = Some(Instant::now());
        tracing::debug!(
            ?operation,
            queue_ms,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "browser worker completed"
        );
    }
    if cancellation.is_cancelled() {
        return Err("Browser request cancelled".into());
    }
    Ok(ResultParts {
        png: (!metadata_only).then(|| cached.png.clone()).flatten(),
        metadata: cached.metadata.clone(),
    })
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
    })
}

fn workers_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let supported = worker::supported();
        if !supported {
            tracing::warn!("Landlock ABI 3 unavailable; retaining one-shot sandboxes");
        }
        supported
    })
}

impl Pool {
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
            Operation::ImageMetadata | Operation::MediaMetadata
        );
        let slow = matches!(
            operation,
            Operation::Raw
                | Operation::Pdf
                | Operation::Video
                | Operation::ImageMetadata
                | Operation::MediaMetadata
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
    fn execute(&mut self, file: &File, operation: Operation) -> Result<Response, String> {
        let mut result = self
            .worker
            .as_mut()
            .ok_or("Missing browser worker")?
            .execute(file, operation);
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
            result = self
                .worker
                .as_mut()
                .expect("replacement worker")
                .execute(file, operation);
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
        if workers_supported() {
            ProcessWorker::spawn().map(Self::Persistent)
        } else {
            Ok(Self::OneShot)
        }
    }

    fn execute(&mut self, file: &File, operation: Operation) -> io::Result<Response> {
        match self {
            Self::Persistent(worker) => worker.execute(file, operation),
            Self::OneShot => {
                use std::os::fd::AsRawFd;
                let operation = parse_operation(operation);
                let output = super::parse(
                    Path::new(&format!("/proc/self/fd/{}", file.as_raw_fd())),
                    operation.clone(),
                    256,
                    super::MediaPreviewBackend::Software,
                    &Cancellation::default(),
                )
                .map_err(io::Error::other)?;
                Ok(if operation == ParseOperation::MediaMetadata {
                    Response {
                        png: Vec::new(),
                        metadata: output.data,
                    }
                } else {
                    Response {
                        png: output.data,
                        metadata: Vec::new(),
                    }
                })
            }
        }
    }
}

struct ProcessWorker {
    child: Child,
    socket: UnixStream,
    _snapshot: super::PrivateOutput,
}

enum LauncherMessage {
    Spawn(std::sync::mpsc::SyncSender<io::Result<ProcessWorker>>),
    Idle,
}

static LAUNCHER: OnceLock<Result<std::sync::mpsc::Sender<LauncherMessage>, String>> =
    OnceLock::new();

impl ProcessWorker {
    fn spawn() -> io::Result<Self> {
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

    fn spawn_on_launcher() -> io::Result<Self> {
        let snapshot = super::PrivateOutput::create()?;
        let executable = super::resolve_renderer_executable(
            &std::env::current_exe()?,
            &PathBuf::from(format!("/proc/{}/exe", std::process::id())),
            snapshot.path(),
        )
        .map_err(io::Error::other)?;
        let (socket, child_socket) = UnixStream::pair()?;
        socket.set_read_timeout(Some(super::WALL_TIME_LIMIT))?;
        socket.set_write_timeout(Some(super::WALL_TIME_LIMIT))?;
        let bwrap = crate::trusted_command::resolve("bwrap").map_err(io::Error::other)?;
        let mut command = super::runtime_command(&bwrap, ParseOperation::MediaMetadata);
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
                "--",
                "/app/strata",
                "--browser-worker",
            ])
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(child_socket)))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = super::spawn_renderer(&mut command)?;
        tracing::debug!(pid = child.id(), "browser sandbox started");
        Ok(Self {
            child,
            socket,
            _snapshot: snapshot,
        })
    }

    fn execute(&mut self, file: &File, operation: Operation) -> io::Result<Response> {
        use io::Read;
        let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
        let deadline = Instant::now() + super::WALL_TIME_LIMIT;
        wire::send(&self.socket, file, &write, operation)?;
        drop(write);
        let mut output = File::from(read);
        let mut reader = DeadlineReader {
            reader: &mut output,
            deadline,
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
}
impl<R: io::Read + std::os::fd::AsFd> io::Read for DeadlineReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Browser renderer timed out",
                ));
            }
            let timeout = Timespec {
                tv_sec: remaining.as_secs() as i64,
                tv_nsec: i64::from(remaining.subsec_nanos()),
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

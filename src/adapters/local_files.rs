// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    fs,
    future::Future,
    io::ErrorKind,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{gio_file_for_location, location_for_file},
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
        LocationValidationError, MetadataOutcome, MetadataRequest, MetadataUpdate, RequestId,
        backend_unavailable_message, is_hidden_name, is_image_path, is_media_path,
        native_hidden_names, native_kind,
    },
};

mod camera_photos;

const LIST_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::is-symlink,standard::target-uri,access::can-trash,access::can-delete";
const FULL_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::is-symlink,standard::size,standard::target-uri,time::modified,unix::mode,access::can-trash,access::can-delete";
const RECENT_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::target-uri,recent::modified";
const METADATA_ATTRIBUTES: &str = "standard::type,standard::size,time::modified,unix::mode";
const MAX_PENDING_MONITOR_CHANGES: usize = 256;
const MAX_ICON_DETAILS_CACHE_ENTRIES: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Eq)]
struct IconDetailsFingerprint {
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl IconDetailsFingerprint {
    fn read(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        Some(Self {
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }
}

#[derive(Clone)]
struct IconDetails {
    image_dimensions: MetadataValue<(u32, u32)>,
    child_count: MetadataValue<u64>,
    duration_seconds: MetadataValue<u64>,
}

impl IconDetails {
    fn from_update(update: &MetadataUpdate) -> Self {
        Self {
            image_dimensions: update.image_dimensions.clone(),
            child_count: update.child_count.clone(),
            duration_seconds: update.duration_seconds.clone(),
        }
    }

    fn apply_to(&self, update: &mut MetadataUpdate) {
        update.image_dimensions = self.image_dimensions.clone();
        update.child_count = self.child_count.clone();
        update.duration_seconds = self.duration_seconds.clone();
    }

    fn apply_to_entry(&self, entry: &mut FileEntry) {
        entry.image_dimensions = self.image_dimensions.clone();
        entry.child_count = self.child_count.clone();
        entry.duration_seconds = self.duration_seconds.clone();
    }
}

struct CachedIconDetails {
    fingerprint: IconDetailsFingerprint,
    details: IconDetails,
    generation: u64,
}

#[derive(Default)]
struct IconDetailsCache {
    entries: HashMap<PathBuf, CachedIconDetails>,
    recent: VecDeque<(PathBuf, u64)>,
    generation: u64,
}

impl IconDetailsCache {
    fn get(&mut self, path: &Path, fingerprint: IconDetailsFingerprint) -> Option<IconDetails> {
        if self
            .entries
            .get(path)
            .is_some_and(|cached| cached.fingerprint != fingerprint)
        {
            self.entries.remove(path);
            return None;
        }
        let cached = self.entries.get_mut(path)?;
        self.generation = self.generation.saturating_add(1);
        cached.generation = self.generation;
        self.recent.push_back((path.to_path_buf(), self.generation));
        let details = cached.details.clone();
        self.compact_recent();
        Some(details)
    }

    fn insert(&mut self, path: PathBuf, fingerprint: IconDetailsFingerprint, details: IconDetails) {
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        self.entries.insert(
            path.clone(),
            CachedIconDetails {
                fingerprint,
                details,
                generation,
            },
        );
        self.recent.push_back((path, generation));
        while self.entries.len() > MAX_ICON_DETAILS_CACHE_ENTRIES {
            let Some((oldest_path, oldest_generation)) = self.recent.pop_front() else {
                break;
            };
            if self
                .entries
                .get(&oldest_path)
                .is_some_and(|cached| cached.generation == oldest_generation)
            {
                self.entries.remove(&oldest_path);
            }
        }
        self.compact_recent();
    }

    fn compact_recent(&mut self) {
        if self.recent.len() > MAX_ICON_DETAILS_CACHE_ENTRIES * 4 {
            self.recent.retain(|(path, generation)| {
                self.entries
                    .get(path)
                    .is_some_and(|cached| cached.generation == *generation)
            });
        }
    }
}

fn icon_details_cache() -> &'static Mutex<IconDetailsCache> {
    static CACHE: OnceLock<Mutex<IconDetailsCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(IconDetailsCache::default()))
}

fn cached_icon_details(path: &Path, fingerprint: IconDetailsFingerprint) -> Option<IconDetails> {
    icon_details_cache().lock().ok()?.get(path, fingerprint)
}

fn cached_icon_details_for_revisit(path: &Path) -> Option<IconDetails> {
    let was_cached = icon_details_cache().lock().ok()?.entries.contains_key(path);
    if !was_cached {
        return None;
    }
    cached_icon_details(path, IconDetailsFingerprint::read(path)?)
}

fn cache_icon_details(
    path: &Path,
    fingerprint: Option<IconDetailsFingerprint>,
    update: &MetadataUpdate,
) {
    let Some(fingerprint) = fingerprint else {
        return;
    };
    if let Ok(mut cache) = icon_details_cache().lock() {
        cache.insert(
            path.to_path_buf(),
            fingerprint,
            IconDetails::from_update(update),
        );
    }
}

#[derive(Default)]
pub struct LocalFileSource;

#[derive(Clone)]
enum PendingMonitorChange {
    Upsert(Location),
    Remove(Location),
    Move { from: Location, to: Location },
    Rescan,
}

type PendingMonitorKey = Option<Location>;

enum NativeEnumeration {
    Complete {
        entries: Vec<FileEntry>,
        truncated: bool,
        metadata_complete: bool,
        can_trash: Option<bool>,
        can_delete: Option<bool>,
    },
    Failed(String),
    Cancelled,
}

enum RecentEntryResolution {
    Entry(Box<FileEntry>),
    Stale,
    TimedOut,
}

type RecentEnumerationFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;

enum RecentSourceError {
    Failed(String),
    TimedOut,
}

trait RecentEnumerationSource {
    fn open(&self, deadline: Instant) -> RecentEnumerationFuture<Result<(), RecentSourceError>>;

    fn next_batch(
        &self,
        batch_size: usize,
        include_metadata: bool,
        deadline: Instant,
    ) -> RecentEnumerationFuture<Result<Option<Vec<RecentEntryResolution>>, RecentSourceError>>;
}

fn map_validation_error(error: std::io::Error) -> LocationValidationError {
    match error.kind() {
        ErrorKind::NotFound => LocationValidationError::Missing,
        ErrorKind::PermissionDenied => LocationValidationError::Inaccessible,
        _ => LocationValidationError::Unavailable(error.to_string()),
    }
}

fn uri_validation_result(
    location: &Location,
    result: Result<gio::FileInfo, glib::Error>,
) -> Result<(), LocationValidationError> {
    let info = result.map_err(|error| {
        if error.matches(gio::IOErrorEnum::NotMounted) {
            LocationValidationError::NotMounted(location.clone())
        } else if error.matches(gio::IOErrorEnum::NotSupported) {
            LocationValidationError::BackendUnavailable(backend_unavailable_message(
                location.uri_value().unwrap_or_default(),
            ))
        } else {
            LocationValidationError::Unavailable(error.to_string())
        }
    })?;
    match info.file_type() {
        gio::FileType::Directory => Ok(()),
        gio::FileType::Mountable => Err(LocationValidationError::Mountable(location.clone())),
        _ => Err(LocationValidationError::NotDirectory),
    }
}

fn info_is_hidden(info: &gio::FileInfo) -> bool {
    info.has_attribute(gio::FILE_ATTRIBUTE_STANDARD_IS_HIDDEN) && info.is_hidden()
}

fn info_is_symlink(info: &gio::FileInfo) -> bool {
    info.has_attribute(gio::FILE_ATTRIBUTE_STANDARD_IS_SYMLINK) && info.is_symlink()
}

fn info_can_trash(info: &gio::FileInfo) -> Option<bool> {
    info.has_attribute(gio::FILE_ATTRIBUTE_ACCESS_CAN_TRASH)
        .then(|| info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_TRASH))
}

fn info_can_delete(info: &gio::FileInfo) -> Option<bool> {
    info.has_attribute(gio::FILE_ATTRIBUTE_ACCESS_CAN_DELETE)
        .then(|| info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_DELETE))
}

fn info_mode(info: &gio::FileInfo) -> MetadataValue<u32> {
    if info.has_attribute(gio::FILE_ATTRIBUTE_UNIX_MODE) {
        MetadataValue::Known(info.attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_MODE))
    } else {
        MetadataValue::Unavailable
    }
}

pub(crate) async fn query_file_entry(location: Location) -> Result<FileEntry, glib::Error> {
    let info = gio_file_for_location(&location)
        .query_info_future(
            FULL_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    Ok(entry_from_info(location, info))
}

fn entry_from_info(location: Location, info: gio::FileInfo) -> FileEntry {
    let location = if matches!(
        info.file_type(),
        gio::FileType::Shortcut | gio::FileType::Mountable
    ) {
        info.attribute_string(gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI)
            .and_then(|uri| location_for_file(&gio::File::for_uri(&uri)))
            .unwrap_or(location)
    } else {
        location
    };
    let native_name = info.name().into_os_string();
    let kind = match (info.file_type(), info_is_symlink(&info)) {
        (gio::FileType::Directory, true) => EntryKind::DirectorySymbolicLink,
        (gio::FileType::Regular, true) => EntryKind::FileSymbolicLink,
        // GVfs browse entries must use directory navigation, including mount-and-retry,
        // rather than launching the desktop's URI handler.
        (gio::FileType::Directory | gio::FileType::Shortcut | gio::FileType::Mountable, false) => {
            EntryKind::Directory
        }
        (gio::FileType::Regular, false) => EntryKind::File,
        (gio::FileType::SymbolicLink, _) => EntryKind::SymbolicLink,
        _ => EntryKind::Other,
    };
    let size = if matches!(
        kind,
        EntryKind::Directory | EntryKind::DirectorySymbolicLink
    ) {
        MetadataValue::Unknown
    } else if info.has_attribute(gio::FILE_ATTRIBUTE_STANDARD_SIZE) {
        u64::try_from(info.size())
            .map(MetadataValue::Known)
            .unwrap_or(MetadataValue::Unavailable)
    } else {
        MetadataValue::Unknown
    };
    let modified_unix_seconds = if info.has_attribute(gio::FILE_ATTRIBUTE_TIME_MODIFIED) {
        info.modification_date_time()
            .map(|modified| MetadataValue::Known(modified.to_unix()))
            .unwrap_or(MetadataValue::Unavailable)
    } else {
        MetadataValue::Unknown
    };
    FileEntry {
        thumbnail_path: trash_thumbnail_path(&location, &info),
        location,
        native_name,
        display_name: info.display_name().to_string(),
        kind,
        size,
        modified_unix_seconds,
        recent_unix_seconds: MetadataValue::Unknown,
        mode: info_mode(&info),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        is_hidden: info_is_hidden(&info),
    }
}

fn recent_unix_seconds(info: &gio::FileInfo) -> MetadataValue<i64> {
    if info.has_attribute(gio::FILE_ATTRIBUTE_RECENT_MODIFIED) {
        MetadataValue::Known(info.attribute_int64(gio::FILE_ATTRIBUTE_RECENT_MODIFIED))
    } else {
        MetadataValue::Unknown
    }
}

fn recent_target_location(info: &gio::FileInfo) -> Option<Location> {
    let target_uri = info.attribute_string(gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI)?;
    let target_file = gio::File::for_uri(target_uri.as_str());
    if target_file.has_uri_scheme("recent") {
        return None;
    }
    location_for_file(&target_file)
}

fn recent_entry_from_target(
    recent_info: &gio::FileInfo,
    target_location: Location,
    target_info: gio::FileInfo,
) -> Option<FileEntry> {
    let mut entry = entry_from_info(target_location, target_info);
    entry.recent_unix_seconds = recent_unix_seconds(recent_info);
    let final_location_is_recent = entry
        .location
        .uri_value()
        .is_some_and(|uri| gio::File::for_uri(uri).has_uri_scheme("recent"));
    (!final_location_is_recent).then_some(entry)
}

// Resolve concurrently so one unreachable target cannot consume the batch's deadline.
async fn resolve_recent_batch(
    infos: Vec<gio::FileInfo>,
    include_metadata: bool,
    deadline: Instant,
) -> Vec<RecentEntryResolution> {
    let context = glib::MainContext::default();
    let pending: Vec<_> = infos
        .into_iter()
        .map(|info| context.spawn_local(resolve_recent_entry(info, include_metadata, deadline)))
        .collect();
    let mut resolutions = Vec::with_capacity(pending.len());
    for handle in pending {
        resolutions.push(handle.await.unwrap_or(RecentEntryResolution::Stale));
    }
    resolutions
}

async fn resolve_recent_entry(
    recent_info: gio::FileInfo,
    include_metadata: bool,
    deadline: Instant,
) -> RecentEntryResolution {
    let Some(target_location) = recent_target_location(&recent_info) else {
        return RecentEntryResolution::Stale;
    };
    let target_file = gio_file_for_location(&target_location);
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return RecentEntryResolution::TimedOut;
    }
    let attributes = if include_metadata {
        FULL_ATTRIBUTES
    } else {
        LIST_ATTRIBUTES
    };
    match glib::future_with_timeout(
        remaining,
        target_file.query_info_future(
            attributes,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        ),
    )
    .await
    {
        Ok(Ok(target_info)) => recent_entry_from_target(&recent_info, target_location, target_info)
            .map(Box::new)
            .map_or(RecentEntryResolution::Stale, RecentEntryResolution::Entry),
        Ok(Err(_)) => RecentEntryResolution::Stale,
        Err(_) => RecentEntryResolution::TimedOut,
    }
}

fn trash_thumbnail_path(location: &Location, info: &gio::FileInfo) -> Option<PathBuf> {
    if !location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
    {
        return None;
    }
    let target = info.attribute_string(gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI)?;
    let (path, hostname) = glib::filename_from_uri(&target).ok()?;
    hostname
        .is_none_or(|host| host.eq_ignore_ascii_case("localhost"))
        .then_some(path)
}

fn unix_seconds(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs()).ok(),
        Err(error) => {
            let duration = error.duration();
            let seconds = i64::try_from(duration.as_secs()).ok()?;
            seconds
                .checked_neg()?
                .checked_sub(i64::from(duration.subsec_nanos() != 0))
        }
    }
}

fn fill_native_entry_metadata(entry: &mut FileEntry) {
    let Some(path) = entry.location.native_path() else {
        return;
    };
    let Ok(metadata) = fs::metadata(path) else {
        entry.size = MetadataValue::Unknown;
        entry.modified_unix_seconds = MetadataValue::Unknown;
        entry.mode = MetadataValue::Unknown;
        return;
    };
    entry.size = if metadata.is_dir() {
        MetadataValue::Unknown
    } else {
        MetadataValue::Known(metadata.len())
    };
    entry.modified_unix_seconds = metadata
        .modified()
        .ok()
        .and_then(unix_seconds)
        .map(MetadataValue::Known)
        .unwrap_or(MetadataValue::Unavailable);
    entry.mode = MetadataValue::Known(metadata.mode());
}

fn scan_native_directory(
    path: &Path,
    request: &DirectoryRequest,
    cancellable: &gio::Cancellable,
    deadline: Instant,
) -> NativeEnumeration {
    let children = match fs::read_dir(path) {
        Ok(children) => children,
        Err(error) => return NativeEnumeration::Failed(error.to_string()),
    };
    let hidden_names = native_hidden_names(path);
    let mut entries = Vec::new();
    let mut truncated = false;
    for child in children {
        if cancellable.is_cancelled() {
            return NativeEnumeration::Cancelled;
        }
        if Instant::now() >= deadline {
            truncated = true;
            break;
        }
        let child = match child {
            Ok(child) => child,
            Err(error) => return NativeEnumeration::Failed(error.to_string()),
        };
        let native_name = child.file_name();
        let is_hidden = is_hidden_name(&native_name, &hidden_names);
        if entries.len() == request.max_entries {
            truncated = true;
            break;
        }
        let file_type = match child.file_type() {
            Ok(file_type) => file_type,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return NativeEnumeration::Failed(error.to_string()),
        };
        let path = child.path();
        let kind = native_kind(file_type, &path);
        let cached_details = cached_icon_details_for_revisit(&path);
        let mut entry = FileEntry {
            location: Location::local(path),
            display_name: native_name.to_string_lossy().into_owned(),
            thumbnail_path: None,
            native_name,
            kind,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            recent_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
            is_hidden,
        };
        if let Some(details) = cached_details {
            details.apply_to_entry(&mut entry);
        }
        entries.push(entry);
    }

    // `access::can-trash`/`access::can-delete` describe the queried item, not its
    // children. Probe one actual entry so a directory that cannot itself be removed
    // (such as `$HOME`) does not incorrectly hide Trash/delete for the entries it
    // contains.
    let probed_capabilities = entries.first().and_then(|entry| {
        gio::File::for_path(entry.location.native_path()?)
            .query_info(
                "access::can-trash,access::can-delete",
                gio::FileQueryInfoFlags::NONE,
                Some(cancellable),
            )
            .ok()
    });
    let can_trash = probed_capabilities.as_ref().and_then(info_can_trash);
    let can_delete = probed_capabilities.as_ref().and_then(info_can_delete);

    let mut metadata_complete = true;
    if request.include_metadata && !entries.is_empty() && Instant::now() < deadline {
        let width = metadata_fill_width().min(entries.len());
        let chunk = entries.len().div_ceil(width);
        std::thread::scope(|scope| {
            for piece in entries.chunks_mut(chunk) {
                let cancellable = cancellable.clone();
                scope.spawn(move || {
                    for entry in piece {
                        if cancellable.is_cancelled() || Instant::now() >= deadline {
                            break;
                        }
                        fill_native_entry_metadata(entry);
                    }
                });
            }
        });
        if cancellable.is_cancelled() {
            return NativeEnumeration::Cancelled;
        }
        metadata_complete = Instant::now() < deadline;
    } else if request.include_metadata && !entries.is_empty() {
        metadata_complete = false;
    }

    NativeEnumeration::Complete {
        entries,
        truncated,
        metadata_complete,
        can_trash,
        can_delete,
    }
}

fn enumerate_native(
    request: DirectoryRequest,
    emit: Rc<dyn Fn(DirectoryEvent)>,
    started: Instant,
    path: PathBuf,
) -> LoadHandle {
    let request_id = request.id;
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        let deadline = started + request.time_budget;
        let outcome = gio::spawn_blocking(move || {
            scan_native_directory(&path, &request, &cancellable, deadline)
        })
        .await;
        let Ok(outcome) = outcome else {
            emit(DirectoryEvent::Failed {
                request_id,
                message: "Native directory worker failed".to_owned(),
            });
            return;
        };
        match outcome {
            NativeEnumeration::Complete {
                entries,
                truncated,
                metadata_complete,
                can_trash,
                can_delete,
            } => {
                let total_entries = entries.len();
                if !entries.is_empty() {
                    tracing::info!(
                        request_id = request_id.0,
                        entries = total_entries,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "first directory batch ready"
                    );
                    emit(DirectoryEvent::Batch {
                        request_id,
                        entries,
                    });
                }
                if !metadata_complete {
                    tracing::warn!(
                        request_id = request_id.0,
                        entries = total_entries,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        reason = "metadata budget",
                        "initial metadata pass incomplete"
                    );
                    emit(DirectoryEvent::MetadataIncomplete { request_id });
                }
                if truncated {
                    tracing::warn!(
                        request_id = request_id.0,
                        entries = total_entries,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        reason = "budget",
                        "directory load truncated"
                    );
                } else {
                    tracing::info!(
                        request_id = request_id.0,
                        entries = total_entries,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "directory load finished"
                    );
                }
                emit(DirectoryEvent::Finished {
                    request_id,
                    truncated,
                    can_trash,
                    can_delete,
                });
            }
            NativeEnumeration::Failed(message) => {
                tracing::warn!(request_id = request_id.0, "directory load failed");
                emit(DirectoryEvent::Failed {
                    request_id,
                    message,
                });
            }
            NativeEnumeration::Cancelled => {}
        }
    });
    LoadHandle::new(move || {
        tracing::debug!(request_id = request_id.0, "directory load cancelled");
        cancel.cancel();
        task.abort();
    })
}

fn enumerate_recent(
    request: DirectoryRequest,
    emit: Rc<dyn Fn(DirectoryEvent)>,
    started: Instant,
) -> LoadHandle {
    let source = Box::new(GioRecentEnumerationSource::new(gio_file_for_location(
        &request.location,
    )));
    enumerate_recent_with_source(request, emit, started, source)
}

struct GioRecentEnumerationSource {
    directory: gio::File,
    enumerator: Rc<RefCell<Option<gio::FileEnumerator>>>,
}

impl GioRecentEnumerationSource {
    fn new(directory: gio::File) -> Self {
        Self {
            directory,
            enumerator: Rc::new(RefCell::new(None)),
        }
    }
}

impl RecentEnumerationSource for GioRecentEnumerationSource {
    fn open(&self, deadline: Instant) -> RecentEnumerationFuture<Result<(), RecentSourceError>> {
        let directory = self.directory.clone();
        let enumerator = self.enumerator.clone();
        Box::pin(async move {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RecentSourceError::TimedOut);
            }
            match glib::future_with_timeout(
                remaining,
                directory.enumerate_children_future(
                    RECENT_ATTRIBUTES,
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                ),
            )
            .await
            {
                Ok(Ok(value)) => {
                    enumerator.replace(Some(value));
                    Ok(())
                }
                Ok(Err(error)) => Err(RecentSourceError::Failed(error.to_string())),
                Err(_) => Err(RecentSourceError::TimedOut),
            }
        })
    }

    fn next_batch(
        &self,
        batch_size: usize,
        include_metadata: bool,
        deadline: Instant,
    ) -> RecentEnumerationFuture<Result<Option<Vec<RecentEntryResolution>>, RecentSourceError>>
    {
        let enumerator = self.enumerator.borrow().clone();
        Box::pin(async move {
            let Some(enumerator) = enumerator else {
                return Err(RecentSourceError::Failed(
                    "Recent enumeration was not opened".to_owned(),
                ));
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RecentSourceError::TimedOut);
            }
            let files = match glib::future_with_timeout(
                remaining,
                enumerator.next_files_future(batch_size as i32, glib::Priority::DEFAULT),
            )
            .await
            {
                Ok(Ok(files)) => files,
                Ok(Err(error)) => return Err(RecentSourceError::Failed(error.to_string())),
                Err(_) => return Err(RecentSourceError::TimedOut),
            };
            if files.is_empty() {
                return Ok(None);
            }
            Ok(Some(
                resolve_recent_batch(files, include_metadata, deadline).await,
            ))
        })
    }
}

fn enumerate_recent_with_source(
    request: DirectoryRequest,
    emit: Rc<dyn Fn(DirectoryEvent)>,
    started: Instant,
    source: Box<dyn RecentEnumerationSource>,
) -> LoadHandle {
    let request_id = request.id;
    let task = glib::MainContext::default().spawn_local(async move {
        let deadline = started + request.time_budget;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            emit(DirectoryEvent::Finished {
                request_id,
                truncated: true,
                can_trash: None,
                can_delete: None,
            });
            return;
        }
        match source.open(deadline).await {
            Ok(()) => {}
            Err(RecentSourceError::Failed(message)) => {
                emit(DirectoryEvent::Failed {
                    request_id,
                    message,
                });
                return;
            }
            Err(RecentSourceError::TimedOut) => {
                emit(DirectoryEvent::Finished {
                    request_id,
                    truncated: true,
                    can_trash: None,
                    can_delete: None,
                });
                return;
            }
        }

        let mut total_entries = 0usize;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                emit(DirectoryEvent::Finished {
                    request_id,
                    truncated: true,
                    can_trash: None,
                    can_delete: None,
                });
                break;
            }
            let resolutions = match source
                .next_batch(request.batch_size, request.include_metadata, deadline)
                .await
            {
                Ok(Some(resolutions)) => resolutions,
                Ok(None) => {
                    emit(DirectoryEvent::Finished {
                        request_id,
                        truncated: false,
                        can_trash: None,
                        can_delete: None,
                    });
                    break;
                }
                Err(RecentSourceError::Failed(message)) => {
                    emit(DirectoryEvent::Failed {
                        request_id,
                        message,
                    });
                    break;
                }
                Err(RecentSourceError::TimedOut) => {
                    emit(DirectoryEvent::Finished {
                        request_id,
                        truncated: true,
                        can_trash: None,
                        can_delete: None,
                    });
                    break;
                }
            };

            let mut entries = Vec::new();
            let mut truncated = false;
            for resolution in resolutions {
                if total_entries >= request.max_entries {
                    truncated = true;
                    break;
                }
                match resolution {
                    RecentEntryResolution::Entry(entry) => {
                        total_entries += 1;
                        entries.push(*entry);
                    }
                    RecentEntryResolution::Stale => {}
                    RecentEntryResolution::TimedOut => truncated = true,
                }
            }
            if !entries.is_empty() {
                emit(DirectoryEvent::Batch {
                    request_id,
                    entries,
                });
            }
            if truncated {
                emit(DirectoryEvent::Finished {
                    request_id,
                    truncated: true,
                    can_trash: None,
                    can_delete: None,
                });
                break;
            }
        }
    });
    LoadHandle::new(move || task.abort())
}

impl FileSource for LocalFileSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if let Some(path) = location.native_path() {
            let metadata = std::fs::metadata(path).map_err(map_validation_error)?;
            if !metadata.is_dir() {
                return Err(LocationValidationError::NotDirectory);
            }
            return std::fs::read_dir(path)
                .map(|_| ())
                .map_err(map_validation_error);
        }

        let file = gio::File::for_uri(
            location
                .uri_value()
                .ok_or_else(|| LocationValidationError::Unavailable("invalid URI".into()))?,
        );
        uri_validation_result(
            location,
            file.query_info(
                "standard::type",
                gio::FileQueryInfoFlags::NONE,
                None::<&gio::Cancellable>,
            ),
        )
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), LocationValidationError>)>,
    ) -> LoadHandle {
        if location.native_path().is_some() {
            emit(self.validate_location(&location));
            return LoadHandle::new(|| {});
        }
        let file = gio::File::for_uri(location.uri_value().unwrap_or_default());
        let task = glib::MainContext::default().spawn_local(async move {
            let result = file
                .query_info_future(
                    "standard::type",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await;
            emit(uri_validation_result(&location, result));
        });
        LoadHandle::new(move || task.abort())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let request_id = request.id;
        let location = request.location.clone();
        let started = Instant::now();
        log_directory_load_started(request_id, &location);

        if let Some(path) = location.native_path() {
            return enumerate_native(request, emit, started, path.to_path_buf());
        }
        if location.is_recent_root() {
            return enumerate_recent(request, emit, started);
        }
        if location.is_camera_photo_root() {
            return camera_photos::enumerate(request, emit);
        }

        let task = glib::MainContext::default().spawn_local(async move {
            let directory = gio_file_for_location(&location);
            let deadline = started + request.time_budget;
            let finish_truncated = |entries: usize,
                                    reason: &'static str,
                                    can_trash: Option<bool>,
                                    can_delete: Option<bool>| {
                tracing::warn!(
                    request_id = request_id.0,
                    entries,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    reason,
                    "directory load truncated"
                );
                emit(DirectoryEvent::Finished {
                    request_id,
                    truncated: true,
                    can_trash,
                    can_delete,
                });
            };
            let mut can_trash = None;
            let mut can_delete = None;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                finish_truncated(0, "time budget", can_trash, can_delete);
                return;
            }
            let attributes = if request.include_metadata {
                FULL_ATTRIBUTES
            } else {
                LIST_ATTRIBUTES
            };
            let enumerator = match glib::future_with_timeout(
                remaining,
                directory.enumerate_children_future(
                    attributes,
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                ),
            )
            .await
            {
                Ok(Ok(enumerator)) => enumerator,
                Ok(Err(error)) => {
                    tracing::warn!(
                        request_id = request_id.0,
                        error_domain = ?error.domain(),
                        error_code = error.code(),
                        "directory load failed"
                    );
                    emit(DirectoryEvent::Failed {
                        request_id,
                        message: error.to_string(),
                    });
                    return;
                }
                Err(_) => {
                    finish_truncated(0, "time budget", can_trash, can_delete);
                    return;
                }
            };

            let mut total_entries = 0usize;
            let mut first_batch = true;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    finish_truncated(total_entries, "time budget", can_trash, can_delete);
                    break;
                }
                match glib::future_with_timeout(
                    remaining,
                    enumerator
                        .next_files_future(request.batch_size as i32, glib::Priority::DEFAULT),
                )
                .await
                {
                    Ok(Ok(files)) if files.is_empty() => {
                        tracing::info!(
                            request_id = request_id.0,
                            entries = total_entries,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "directory load finished"
                        );
                        emit(DirectoryEvent::Finished {
                            request_id,
                            truncated: false,
                            can_trash,
                            can_delete,
                        });
                        break;
                    }
                    Ok(Ok(files)) => {
                        if can_trash.is_none() {
                            can_trash = files.iter().find_map(info_can_trash);
                        }
                        if can_delete.is_none() {
                            can_delete = files.iter().find_map(info_can_delete);
                        }
                        let mut entries: Vec<_> = files
                            .into_iter()
                            .filter_map(|info| {
                                let child = directory.child(info.name());
                                Some(entry_from_info(location_for_file(&child)?, info))
                            })
                            .collect();
                        let remaining_capacity = request.max_entries.saturating_sub(total_entries);
                        let entry_budget_exhausted = entries.len() > remaining_capacity;
                        entries.truncate(remaining_capacity);
                        total_entries += entries.len();
                        if first_batch {
                            tracing::info!(
                                request_id = request_id.0,
                                entries = entries.len(),
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                "first directory batch ready"
                            );
                            first_batch = false;
                        }
                        emit(DirectoryEvent::Batch {
                            request_id,
                            entries,
                        });
                        if entry_budget_exhausted {
                            finish_truncated(total_entries, "entry budget", can_trash, can_delete);
                            break;
                        }
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(
                            request_id = request_id.0,
                            error_domain = ?error.domain(),
                            error_code = error.code(),
                            "directory load interrupted"
                        );
                        emit(DirectoryEvent::Failed {
                            request_id,
                            message: error.to_string(),
                        });
                        break;
                    }
                    Err(_) => {
                        finish_truncated(total_entries, "time budget", can_trash, can_delete);
                        break;
                    }
                }
            }
        });

        LoadHandle::new(move || {
            tracing::debug!(request_id = request_id.0, "directory load cancelled");
            task.abort();
        })
    }

    fn supports_metadata_fill(&self, _location: &Location) -> bool {
        true
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        // Cap viewport fills so a buggy caller cannot stat a full directory.
        const MAX_FILL_ENTRIES: usize = 1024;
        let request_id = request.id;
        let include_icon_details = request.include_icon_details;
        let mut locations = request.entries;
        if !request.full {
            locations.truncate(MAX_FILL_ENTRIES);
        }
        let all_native = locations
            .iter()
            .all(|location| location.native_path().is_some());
        if all_native && !locations.is_empty() {
            return fill_parallel(
                request_id,
                locations,
                include_icon_details,
                request.time_budget,
                emit,
            );
        }
        let task = glib::MainContext::default().spawn_local(async move {
            let deadline = Instant::now() + request.time_budget;
            let mut updates = Vec::with_capacity(locations.len());
            let mut truncated = false;
            let mut attempted = 0usize;
            let mut failed = 0usize;
            for location in &locations {
                if Instant::now() >= deadline {
                    truncated = true;
                    break;
                }
                let Some(file) = location
                    .native_path()
                    .map(gio::File::for_path)
                    .or_else(|| location.uri_value().map(gio::File::for_uri))
                else {
                    continue;
                };
                attempted += 1;
                // Bound each stat by the remaining budget so one hung mount cannot stall the fill.
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    truncated = true;
                    break;
                }
                let (update, ok) = match glib::future_with_timeout(
                    remaining,
                    file.query_info_future(
                        METADATA_ATTRIBUTES,
                        gio::FileQueryInfoFlags::NONE,
                        glib::Priority::DEFAULT,
                    ),
                )
                .await
                {
                    Ok(Ok(info)) => {
                        let (mut update, ok) = update_from_info(&info, location);
                        if include_icon_details {
                            update.image_dimensions = MetadataValue::Unavailable;
                            update.child_count = MetadataValue::Unavailable;
                            update.duration_seconds = MetadataValue::Unavailable;
                        }
                        (update, ok)
                    }
                    Ok(Err(_)) => (
                        MetadataUpdate {
                            location: location.clone(),
                            size: MetadataValue::Unknown,
                            modified_unix_seconds: MetadataValue::Unknown,
                            mode: MetadataValue::Unknown,
                            image_dimensions: MetadataValue::Unknown,
                            child_count: MetadataValue::Unknown,
                            duration_seconds: MetadataValue::Unknown,
                        },
                        false,
                    ),
                    Err(_) => {
                        truncated = true;
                        break;
                    }
                };
                failed += usize::from(!ok);
                updates.push(update);
            }
            emit_fill_outcome(
                &emit,
                request_id,
                updates,
                truncated,
                attempted,
                failed,
                locations.len(),
            );
        });
        LoadHandle::new(move || task.abort())
    }

    fn watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        let _ = include_hidden;
        let file = gio_file_for_location(&location);
        let monitor = match file.monitor_directory(
            gio::FileMonitorFlags::WATCH_MOVES,
            None::<&gio::Cancellable>,
        ) {
            Ok(monitor) => monitor,
            Err(error) => {
                tracing::warn!(
                    backend = %location.backend_name(),
                    error_domain = ?error.domain(),
                    error_code = error.code(),
                    "directory monitoring unavailable"
                );
                tracing::debug!(
                    location = %location.diagnostic_path(),
                    "directory monitoring location"
                );
                return None;
            }
        };

        let cancelled = Rc::new(Cell::new(false));
        let pending = Rc::new(RefCell::new(HashMap::<
            PendingMonitorKey,
            PendingMonitorChange,
        >::new()));
        let timeout = Rc::new(RefCell::new(None::<glib::SourceId>));
        let pending_for_change = pending.clone();
        let timeout_for_change = timeout.clone();
        let cancelled_for_change = cancelled.clone();
        let watched = location.clone();
        monitor.connect_changed(move |_, file, other_file, event| {
            if pending_for_change.borrow().contains_key(&None) {
                return;
            }
            let change = pending_monitor_change(
                &watched,
                location_for_file(file),
                other_file.and_then(location_for_file),
                event,
            );
            let Some(change) = change else {
                return;
            };
            let key = match &change {
                PendingMonitorChange::Upsert(location) | PendingMonitorChange::Remove(location) => {
                    Some(location.clone())
                }
                PendingMonitorChange::Move { to, .. } => Some(to.clone()),
                PendingMonitorChange::Rescan => None,
            };
            if !queue_monitor_change(&mut pending_for_change.borrow_mut(), key, change) {
                return;
            }

            if let Some(source) = timeout_for_change.take() {
                source.remove();
            }
            let pending = pending_for_change.clone();
            let timeout = timeout_for_change.clone();
            let notify = notify.clone();
            let cancelled = cancelled_for_change.clone();
            let source = glib::timeout_add_local_once(Duration::from_millis(100), move || {
                timeout.take();
                flush_monitor_changes(&pending, &notify, &cancelled);
            });
            timeout_for_change.replace(Some(source));
        });

        Some(LoadHandle::new(move || {
            cancelled.set(true);
            if let Some(source) = timeout.take() {
                source.remove();
            }
            pending.borrow_mut().clear();
            let _cancelled = monitor.cancel();
        }))
    }
}

fn metadata_fill_width() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().min(8))
        .unwrap_or(4)
        .max(1)
}
fn fill_parallel(
    request_id: RequestId,
    locations: Vec<Location>,
    include_icon_details: bool,
    time_budget: Duration,
    emit: Rc<dyn Fn(DirectoryEvent)>,
) -> LoadHandle {
    fill_parallel_with(
        metadata_fill_width(),
        request_id,
        locations,
        include_icon_details,
        time_budget,
        emit,
    )
}

fn fill_parallel_with(
    width: usize,
    request_id: RequestId,
    locations: Vec<Location>,
    include_icon_details: bool,
    time_budget: Duration,
    emit: Rc<dyn Fn(DirectoryEvent)>,
) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let cancelled = cancellable.clone();
    let (tx, mut rx) = futures_channel::mpsc::unbounded::<Vec<MetadataUpdate>>();
    let task = glib::MainContext::default().spawn_local(async move {
        let deadline = Instant::now() + time_budget;
        let locations_len = locations.len();
        let blocking_task = gio::spawn_blocking(move || {
            let worker_count = width.max(1).min(locations.len());
            let next = AtomicUsize::new(0);
            let mut attempted = 0usize;
            let mut failed = 0usize;
            let mut truncated = false;
            std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(worker_count);
                for _ in 0..worker_count {
                    let cancellable = cancellable.clone();
                    let tx = tx.clone();
                    let locations = &locations;
                    let next = &next;
                    handles.push(scope.spawn(move || {
                        let mut updates = Vec::new();
                        let mut sent_first = false;
                        let mut attempted = 0usize;
                        let mut failed = 0usize;
                        let mut truncated = false;
                        loop {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some(location) = locations.get(index) else {
                                break;
                            };
                            if cancellable.is_cancelled() || Instant::now() >= deadline {
                                truncated = true;
                                break;
                            }
                            let Some(path) = location.native_path() else {
                                continue;
                            };
                            attempted += 1;
                            let (mut update, ok, details_complete) = match gio::File::for_path(path)
                                .query_info(
                                    METADATA_ATTRIBUTES,
                                    gio::FileQueryInfoFlags::NONE,
                                    Some(&cancellable),
                                ) {
                                Ok(info) => {
                                    let (mut update, ok) = update_from_info(&info, location);
                                    let details_complete = !include_icon_details
                                        || fill_icon_details(
                                            &mut update,
                                            &info,
                                            location,
                                            &cancellable,
                                            deadline,
                                        );
                                    (update, ok, details_complete)
                                }
                                Err(_) => (
                                    MetadataUpdate {
                                        location: location.clone(),
                                        size: MetadataValue::Unknown,
                                        modified_unix_seconds: MetadataValue::Unknown,
                                        mode: MetadataValue::Unknown,
                                        image_dimensions: MetadataValue::Unknown,
                                        child_count: MetadataValue::Unknown,
                                        duration_seconds: MetadataValue::Unknown,
                                    },
                                    false,
                                    true,
                                ),
                            };
                            if include_icon_details && !details_complete {
                                update.image_dimensions = MetadataValue::Unknown;
                                update.child_count = MetadataValue::Unknown;
                                update.duration_seconds = MetadataValue::Unknown;
                                truncated = true;
                            }
                            failed += usize::from(!ok);
                            updates.push(update);
                            if !sent_first || updates.len() >= 8 {
                                let _ = tx.unbounded_send(std::mem::take(&mut updates));
                                sent_first = true;
                            }
                            if !details_complete {
                                break;
                            }
                        }
                        if !updates.is_empty() {
                            let _ = tx.unbounded_send(updates);
                        }
                        (attempted, failed, truncated)
                    }));
                }
                for handle in handles {
                    let Ok((attempted_piece, failed_piece, truncated_piece)) = handle.join() else {
                        truncated = true;
                        continue;
                    };
                    attempted += attempted_piece;
                    failed += failed_piece;
                    truncated = truncated || truncated_piece;
                }
            });
            (attempted, failed, truncated, locations_len)
        });

        use futures_lite::StreamExt;
        while let Some(chunk) = rx.next().await {
            if !chunk.is_empty() && !cancelled.is_cancelled() {
                emit(DirectoryEvent::MetadataFilled {
                    request_id,
                    updates: chunk,
                });
            }
        }

        let Ok((attempted, failed, truncated, total)) = blocking_task.await else {
            return;
        };
        if cancelled.is_cancelled() {
            emit(DirectoryEvent::MetadataFinished {
                request_id,
                outcome: MetadataOutcome::Cancelled,
            });
            return;
        }
        let outcome = if truncated {
            MetadataOutcome::Truncated
        } else if attempted == 0 && total > 0 {
            MetadataOutcome::Unsupported
        } else if attempted > 0 && failed == attempted {
            MetadataOutcome::Failed
        } else {
            MetadataOutcome::Complete
        };
        emit(DirectoryEvent::MetadataFinished {
            request_id,
            outcome,
        });
    });
    LoadHandle::new(move || {
        cancel.cancel();
        task.abort();
    })
}

#[cfg(test)]
fn media_metadata_probe_counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
    static COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn media_metadata_probe_count(path: &Path) -> usize {
    media_metadata_probe_counts()
        .lock()
        .ok()
        .and_then(|counts| counts.get(path).copied())
        .unwrap_or(0)
}

fn probe_sandboxed_media_metadata(
    path: &Path,
    image: bool,
) -> Result<crate::sandbox::metadata::MediaMetadata, String> {
    #[cfg(not(test))]
    {
        let cancellation = crate::sandbox::Cancellation::default();
        crate::sandbox::parse(
            path,
            crate::sandbox::ParseOperation::MediaMetadata,
            0,
            crate::sandbox::MediaPreviewBackend::Software,
            &cancellation,
        )
        .and_then(|output| crate::sandbox::metadata::MediaMetadata::from_json(&output.data, image))
    }
    #[cfg(test)]
    {
        let output_dir = tempfile::tempdir().map_err(|error| error.to_string())?;
        let output_path = output_dir.path().join("result.json");
        crate::sandbox_helper::run(&[
            "media-metadata".to_owned(),
            path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
            "0".to_owned(),
            "software".to_owned(),
        ])?;
        let data = std::fs::read(&output_path).map_err(|error| error.to_string())?;
        crate::sandbox::metadata::MediaMetadata::from_json(&data, image)
    }
}

fn fill_icon_details(
    update: &mut MetadataUpdate,
    info: &gio::FileInfo,
    location: &Location,
    cancellable: &gio::Cancellable,
    deadline: Instant,
) -> bool {
    let Some(path) = location.native_path() else {
        update.image_dimensions = MetadataValue::Unavailable;
        update.child_count = MetadataValue::Unavailable;
        update.duration_seconds = MetadataValue::Unavailable;
        return true;
    };
    if cancellable.is_cancelled() || Instant::now() >= deadline {
        return false;
    }
    let fingerprint = IconDetailsFingerprint::read(path);
    if let Some(details) =
        fingerprint.and_then(|fingerprint| cached_icon_details(path, fingerprint))
    {
        details.apply_to(update);
        return true;
    }

    update.child_count = if info.file_type() == gio::FileType::Directory {
        let Ok(entries) = fs::read_dir(path) else {
            update.image_dimensions = MetadataValue::Unavailable;
            update.child_count = MetadataValue::Unavailable;
            update.duration_seconds = MetadataValue::Unavailable;
            cache_icon_details(path, fingerprint, update);
            return true;
        };
        let mut count = 0u64;
        for entry in entries {
            if cancellable.is_cancelled() || Instant::now() >= deadline {
                return false;
            }
            if entry.is_ok() {
                count = count.saturating_add(1);
            }
        }
        MetadataValue::Known(count)
    } else {
        MetadataValue::Unavailable
    };

    if cancellable.is_cancelled() || Instant::now() >= deadline {
        return false;
    }
    let needs_metadata =
        info.file_type() == gio::FileType::Regular && (is_image_path(path) || is_media_path(path));
    if needs_metadata {
        #[cfg(test)]
        if let Ok(mut counts) = media_metadata_probe_counts().lock() {
            *counts.entry(path.to_path_buf()).or_default() += 1;
        }
        let image = is_image_path(path);
        match probe_sandboxed_media_metadata(path, image) {
            Ok(metadata) => {
                update.image_dimensions = metadata
                    .dimensions
                    .map(MetadataValue::Known)
                    .unwrap_or(MetadataValue::Unavailable);
                update.duration_seconds = metadata
                    .duration
                    .filter(|d| *d > 0.0)
                    .map(|d| MetadataValue::Known(d.round() as u64))
                    .unwrap_or(MetadataValue::Unavailable);
            }
            Err(_) => {
                update.image_dimensions = MetadataValue::Unavailable;
                update.duration_seconds = MetadataValue::Unavailable;
            }
        }
    } else {
        update.image_dimensions = MetadataValue::Unavailable;
        update.duration_seconds = MetadataValue::Unavailable;
    }
    cache_icon_details(path, fingerprint, update);
    true
}

fn update_from_info(info: &gio::FileInfo, location: &Location) -> (MetadataUpdate, bool) {
    let size = if info.file_type() == gio::FileType::Directory {
        MetadataValue::Unknown
    } else {
        u64::try_from(info.size())
            .map(MetadataValue::Known)
            .unwrap_or(MetadataValue::Unavailable)
    };
    let modified_unix_seconds = info
        .modification_date_time()
        .map(|modified| MetadataValue::Known(modified.to_unix()))
        .unwrap_or(MetadataValue::Unavailable);
    let mode = info_mode(info);
    let ok = size != MetadataValue::Unknown
        || modified_unix_seconds != MetadataValue::Unknown
        || mode != MetadataValue::Unknown;
    (
        MetadataUpdate {
            location: location.clone(),
            size,
            modified_unix_seconds,
            mode,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
        },
        ok,
    )
}

fn emit_fill_outcome(
    emit: &Rc<dyn Fn(DirectoryEvent)>,
    request_id: RequestId,
    updates: Vec<MetadataUpdate>,
    truncated: bool,
    attempted: usize,
    failed: usize,
    total: usize,
) {
    if !updates.is_empty() {
        emit(DirectoryEvent::MetadataFilled {
            request_id,
            updates,
        });
    }
    let outcome = if truncated {
        MetadataOutcome::Truncated
    } else if attempted == 0 && total > 0 {
        MetadataOutcome::Unsupported
    } else if attempted > 0 && failed == attempted {
        MetadataOutcome::Failed
    } else {
        MetadataOutcome::Complete
    };
    emit(DirectoryEvent::MetadataFinished {
        request_id,
        outcome,
    });
}

fn log_directory_load_started(request_id: RequestId, location: &Location) {
    tracing::info!(
        request_id = request_id.0,
        backend = %location.backend_name(),
        "directory load started"
    );
    tracing::debug!(
        request_id = request_id.0,
        location = %location.diagnostic_path(),
        "directory load location"
    );
}

fn pending_monitor_change(
    watched: &Location,
    changed: Option<Location>,
    other: Option<Location>,
    event: gio::FileMonitorEvent,
) -> Option<PendingMonitorChange> {
    if watched.is_recent_root() {
        return Some(PendingMonitorChange::Rescan);
    }

    let changed = monitored_change_target(watched, changed, event);
    let change = match event {
        gio::FileMonitorEvent::Deleted | gio::FileMonitorEvent::MovedOut => {
            changed.map(PendingMonitorChange::Remove)
        }
        gio::FileMonitorEvent::Created | gio::FileMonitorEvent::MovedIn => {
            changed.map(PendingMonitorChange::Upsert)
        }
        gio::FileMonitorEvent::Changed
        | gio::FileMonitorEvent::ChangesDoneHint
        | gio::FileMonitorEvent::AttributeChanged => changed.map(PendingMonitorChange::Upsert),
        gio::FileMonitorEvent::Moved | gio::FileMonitorEvent::Renamed => changed
            .zip(other)
            .map(|(from, to)| PendingMonitorChange::Move { from, to }),
        gio::FileMonitorEvent::PreUnmount | gio::FileMonitorEvent::Unmounted => {
            Some(PendingMonitorChange::Rescan)
        }
        _ => Some(PendingMonitorChange::Rescan),
    };
    if watched.is_camera_photo_root() {
        Some(PendingMonitorChange::Rescan)
    } else {
        change
    }
}

// GVfs can report content changes against the watched directory itself; keep only departures.
fn monitored_change_target(
    watched: &Location,
    changed: Option<Location>,
    event: gio::FileMonitorEvent,
) -> Option<Location> {
    let changed = changed?;
    let departed = matches!(
        event,
        gio::FileMonitorEvent::Deleted | gio::FileMonitorEvent::MovedOut
    );
    (&changed != watched || departed).then_some(changed)
}

fn queue_monitor_change(
    pending: &mut HashMap<PendingMonitorKey, PendingMonitorChange>,
    key: PendingMonitorKey,
    change: PendingMonitorChange,
) -> bool {
    if pending.contains_key(&None) {
        return false;
    }
    if let PendingMonitorChange::Move { ref from, .. } = change
        && matches!(
            pending.get(&Some(from.clone())),
            Some(PendingMonitorChange::Upsert(_))
        )
    {
        pending.remove(&Some(from.clone()));
    }
    pending
        .entry(key)
        .and_modify(|pending| {
            *pending = merge_pending_change(pending.clone(), change.clone());
        })
        .or_insert(change);
    if pending.len() > MAX_PENDING_MONITOR_CHANGES {
        pending.clear();
        pending.insert(None, PendingMonitorChange::Rescan);
    }
    true
}

fn merge_pending_change(
    existing: PendingMonitorChange,
    incoming: PendingMonitorChange,
) -> PendingMonitorChange {
    match (&existing, &incoming) {
        (PendingMonitorChange::Rescan, _) | (_, PendingMonitorChange::Rescan) => {
            PendingMonitorChange::Rescan
        }
        (PendingMonitorChange::Move { .. }, PendingMonitorChange::Upsert(_)) => existing,
        (PendingMonitorChange::Move { .. }, PendingMonitorChange::Remove(_)) => {
            PendingMonitorChange::Rescan
        }
        (_, PendingMonitorChange::Move { .. }) => incoming,
        _ => incoming,
    }
}

fn flush_monitor_changes(
    pending: &RefCell<HashMap<PendingMonitorKey, PendingMonitorChange>>,
    notify: &Rc<dyn Fn(DirectoryChange)>,
    cancelled: &Rc<Cell<bool>>,
) {
    let changes: Vec<_> = pending
        .borrow_mut()
        .drain()
        .map(|(_, change)| change)
        .collect();
    if changes
        .iter()
        .any(|change| matches!(change, PendingMonitorChange::Rescan))
    {
        notify(DirectoryChange::Rescan);
        return;
    }

    for change in changes {
        match change {
            PendingMonitorChange::Remove(location) => {
                notify(DirectoryChange::Remove(location));
            }
            PendingMonitorChange::Upsert(location) => {
                query_monitored_entry(location, None, notify.clone(), cancelled.clone())
            }
            PendingMonitorChange::Move { from, to } => {
                query_monitored_entry(to, Some(from), notify.clone(), cancelled.clone())
            }
            PendingMonitorChange::Rescan => {}
        }
    }
}

fn query_monitored_entry(
    location: Location,
    moved_from: Option<Location>,
    notify: Rc<dyn Fn(DirectoryChange)>,
    cancelled: Rc<Cell<bool>>,
) {
    glib::MainContext::default().spawn_local(async move {
        let file = gio_file_for_location(&location);
        let result = file
            .query_info_future(
                FULL_ATTRIBUTES,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await;
        if cancelled.get() {
            return;
        }
        match result {
            Ok(info) => {
                let entry = entry_from_info(location, info);
                if let Some(from) = moved_from {
                    notify(DirectoryChange::Move { from, entry });
                } else {
                    notify(DirectoryChange::Upsert(entry));
                }
            }
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => {
                let removed = moved_from.unwrap_or(location);
                if !cancelled.get() {
                    notify(DirectoryChange::Remove(removed));
                }
            }
            Err(error) => {
                tracing::debug!(
                    location = %location.diagnostic_path(),
                    error = %error,
                    "monitor metadata unavailable"
                );
                if !cancelled.get() {
                    notify(DirectoryChange::Rescan);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests;

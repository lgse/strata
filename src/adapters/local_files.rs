// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    ffi::{OsStr, OsString},
    fs,
    future::Future,
    io::ErrorKind,
    ops::ControlFlow,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gtk::{gio, glib, prelude::*};
use rustix::fs::FileType;

use super::local_operations::FolderStamp;
use crate::{
    adapters::{
        gio_close::CancelOnDrop,
        gio_file_for_location, location_for_file,
        mount_lanes::{self, Class, Progress, Unanswered},
        mount_watch,
        volume::CoarseTimes,
    },
    model::{EntryKind, FileEntry, Location, MetadataValue, diagnostic_uri},
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, DirectoryStamp, Evidence, FileSource,
        IdentityTraits, ItemIdentity, LoadHandle, LocationValidationError, MetadataOutcome,
        MetadataRequest, MetadataUpdate, MountToken, RequestId, Responsiveness, StampAnswer,
        Watching, backend_unavailable_message, bytes_support_named_family, file_activity,
        is_hidden_name, is_image_path, is_media_path, link_target_kind, listed_kind,
        native_hidden_names, sanitize_failure_message, symlink_kind,
    },
};
use futures_channel::oneshot;

mod camera_photos;
mod monitors;

pub(crate) use monitors::watch;

const LIST_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::is-symlink,standard::target-uri,access::can-trash,access::can-delete";
const FULL_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::is-symlink,standard::size,standard::target-uri,time::modified,unix::mode,access::can-trash,access::can-delete";
const RECENT_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::target-uri,recent::modified";
const METADATA_ATTRIBUTES: &str = "standard::type,standard::size,time::modified,unix::mode";
const STAMP_ATTRIBUTES: &str =
    "standard::type,time::modified,time::modified-usec,time::changed,time::changed-usec";
const MAX_PENDING_MONITOR_CHANGES: usize = 4_096;
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

    fn of(target: &FolderStamp) -> Self {
        Self {
            size: target.size,
            modified_seconds: target.modified.0,
            modified_nanoseconds: i64::from(target.modified.1),
            changed_seconds: target.changed.0,
            changed_nanoseconds: i64::from(target.changed.1),
        }
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

static CACHE_HAS_ENTRIES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn revisits(path: &Path) -> bool {
    CACHE_HAS_ENTRIES.load(std::sync::atomic::Ordering::Acquire)
        && icon_details_cache()
            .lock()
            .is_ok_and(|cache| cache.entries.contains_key(path))
}

fn cached_icon_details_for_revisit(path: &Path) -> Option<IconDetails> {
    if !revisits(path) {
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
        CACHE_HAS_ENTRIES.store(true, std::sync::atomic::Ordering::Release);
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

#[derive(Clone)]
enum NativeEnumeration {
    Complete {
        stamp: Box<DirectoryStamp>,
        entries: Vec<FileEntry>,
        truncated: bool,
        names_digest: u64,
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
            LocationValidationError::Unavailable(sanitize_failure_message(
                &crate::services::gio_error_detail(&error),
            ))
        }
    })?;
    match info.file_type() {
        gio::FileType::Directory => Ok(()),
        gio::FileType::Mountable => Err(LocationValidationError::Mountable(location.clone())),
        _ => Err(LocationValidationError::NotDirectory),
    }
}

fn directory_stamp(path: &Path, stamp: &FolderStamp, read_at: SystemTime) -> DirectoryStamp {
    let mount = super::local_operations::stamp_mount(path, stamp);
    DirectoryStamp {
        modified: stamp.modified,
        changed: Some(stamp.changed),
        identity: Some(ItemIdentity {
            evidence: Evidence {
                device: stamp.device,
                inode: stamp.inode,
                kind: stamp.kind,
                birth: stamp.birth,
                handle: None,
                attributes: None,
            },
            traits: mount
                .as_ref()
                .map_or_else(IdentityTraits::default, |mount| mount.identity),
        }),
        mount: stamp
            .mount_id
            .or(mount.as_ref().map(|mount| mount.id))
            .map(MountToken::Native),
        outside_writers: mount.as_ref().is_some_and(|mount| mount.outside_writers),
        daemon_cache: mount.as_ref().and_then(|mount| mount.daemon_cache),
        coarse_window: mount
            .and_then(|mount| mount.coarse_window)
            .or_else(|| CoarseTimes::whole_second(stamp.modified.1).map(|coarse| coarse.window)),
        read_at,
    }
}

fn remote_stamp(uri: &str, info: &gio::FileInfo, read_at: SystemTime) -> DirectoryStamp {
    let time = |seconds: &str, microseconds: &str| {
        info.has_attribute(seconds).then(|| {
            (
                i64::try_from(info.attribute_uint64(seconds)).unwrap_or(i64::MAX),
                info.attribute_uint32(microseconds).saturating_mul(1000),
            )
        })
    };
    let modified = time("time::modified", "time::modified-usec").unwrap_or_default();
    let scheme = glib::Uri::parse_scheme(uri).unwrap_or_default();
    DirectoryStamp {
        modified,
        changed: time("time::changed", "time::changed-usec"),
        identity: None,
        mount: Some(MountToken::Remote(mount_lanes::authority(uri))),
        outside_writers: !matches!(scheme.as_str(), "admin" | "mtp" | "gphoto2"),
        coarse_window: CoarseTimes::whole_second(modified.1).map(|coarse| coarse.window),
        daemon_cache: None,
        read_at,
    }
}

fn stamped_remotely(location: &Location) -> bool {
    location.uri_value().is_some_and(|uri| {
        !location.is_recent_location()
            && !location.is_camera_photo_root()
            && glib::Uri::parse_scheme(uri).is_some_and(|scheme| scheme != "trash")
    })
}

fn stamp_failure(error: rustix::io::Errno) -> StampAnswer {
    use rustix::io::Errno;
    match error {
        Errno::NOENT | Errno::NOTDIR | Errno::STALE => StampAnswer::Missing,
        Errno::ACCESS | Errno::PERM => StampAnswer::Inaccessible,
        _ => StampAnswer::Unreachable,
    }
}

fn remote_stamp_failure(error: &glib::Error) -> StampAnswer {
    if error.matches(gio::IOErrorEnum::NotFound) || error.matches(gio::IOErrorEnum::NotDirectory) {
        StampAnswer::Missing
    } else if error.matches(gio::IOErrorEnum::PermissionDenied) {
        StampAnswer::Inaccessible
    } else {
        StampAnswer::Unreachable
    }
}

fn read_stamp(path: &Path, progress: &Progress) -> Result<DirectoryStamp, rustix::io::Errno> {
    let read_at = SystemTime::now();
    let stamp = super::local_operations::folder_stamp(path, true);
    progress.answered(&stamp);
    stamp.map(|stamp| {
        if let Some(id) = stamp.mount_id {
            progress.learn(id);
        }
        directory_stamp(path, &stamp, read_at)
    })
}

fn stamp_answer(stamp: Result<DirectoryStamp, rustix::io::Errno>) -> StampAnswer {
    match stamp {
        Ok(stamp) => StampAnswer::Stamp(Box::new(stamp)),
        Err(error) => stamp_failure(error),
    }
}

fn unanswered_stamp(refusal: Unanswered) -> StampAnswer {
    match refusal {
        Unanswered::Failed => StampAnswer::Unreachable,
        Unanswered::TimedOut | Unanswered::Skipped => StampAnswer::Unresponsive,
    }
}

async fn query_remote_stamp(uri: String, class: Class) -> StampAnswer {
    let read_at = SystemTime::now();
    let query = gio::File::for_uri(&uri).query_info_future(
        STAMP_ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        glib::Priority::LOW,
    );
    match mount_lanes::run_remote(mount_lanes::authority(&uri), class, query).await {
        Ok(Ok(info)) if info.file_type() == gio::FileType::Directory => {
            StampAnswer::Stamp(Box::new(remote_stamp(&uri, &info, read_at)))
        }
        Ok(Ok(_)) => StampAnswer::Missing,
        Ok(Err(error)) => remote_stamp_failure(&error),
        Err(refusal) => unanswered_stamp(refusal),
    }
}

fn check_native_stamps(
    locations: Vec<Location>,
    emit: Rc<dyn Fn(Location, StampAnswer)>,
) -> glib::JoinHandle<()> {
    let paths: Vec<PathBuf> = locations
        .iter()
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect();
    let job = mount_lanes::run(&paths[0].clone(), Class::Background, move |progress| {
        paths
            .iter()
            .map(|path| stamp_answer(read_stamp(path, progress)))
            .collect::<Vec<_>>()
    });
    glib::MainContext::default().spawn_local(async move {
        let answers = match job.await {
            Ok(answers) => answers,
            Err(refusal) => vec![unanswered_stamp(refusal); locations.len()],
        };
        for (location, answer) in locations.into_iter().zip(answers) {
            emit(location, answer);
        }
    })
}

fn name_hash(name: &OsStr) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    name.as_encoded_bytes().hash(&mut hasher);
    hasher.finish()
}

fn remote_directory_failure(request_id: RequestId, error: &glib::Error) -> DirectoryEvent {
    DirectoryEvent::Failed {
        request_id,
        message: sanitize_failure_message(&error.to_string()),
        refused: false,
    }
}

pub(super) fn info_is_hidden(info: &gio::FileInfo) -> bool {
    info.has_attribute(gio::FILE_ATTRIBUTE_STANDARD_IS_HIDDEN) && info.is_hidden()
}

pub(super) fn info_is_symlink(info: &gio::FileInfo) -> bool {
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
    query_entry(
        location,
        FULL_ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
    )
    .await
}

async fn query_entry(
    location: Location,
    attributes: &'static str,
    flags: gio::FileQueryInfoFlags,
) -> Result<FileEntry, glib::Error> {
    let file = gio_file_for_location(&location);
    if location.native_path().is_none() {
        let info = file
            .query_info_future(attributes, flags, glib::Priority::DEFAULT)
            .await?;
        return Ok(entry_from_info(location, info));
    }
    let abandoned = CancelOnDrop(gio::Cancellable::new());
    let cancellable = abandoned.0.clone();
    gio::spawn_blocking(move || {
        let info = file.query_info(attributes, flags, Some(&cancellable))?;
        cancellable.set_error_if_cancelled()?;
        Ok(entry_from_info(location, info))
    })
    .await
    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn listed_entry(
    request_id: RequestId,
    location: &Location,
    directory: &gio::File,
    info: gio::FileInfo,
) -> Option<FileEntry> {
    let Some(child) = location_for_file(&directory.child(info.name())) else {
        tracing::warn!(
            request_id = request_id.0,
            backend = %location.backend_name(),
            "skipped a directory entry whose URI could not be parsed"
        );
        let display_name = info
            .has_attribute(gio::FILE_ATTRIBUTE_STANDARD_DISPLAY_NAME)
            .then(|| info.display_name());
        tracing::debug!(
            request_id = request_id.0,
            location = %location.diagnostic_path(),
            display_name = ?display_name,
            name_bytes = info.name().as_os_str().len(),
            "skipped directory entry"
        );
        return None;
    };
    Some(entry_from_info(child, info))
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
        (gio::FileType::SymbolicLink, _) => location
            .native_path()
            .map_or(EntryKind::SymbolicLink, symlink_kind),
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
        recent_uri: None,
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
    recent_uri: String,
    mut entry: FileEntry,
) -> Option<FileEntry> {
    entry.recent_unix_seconds = recent_unix_seconds(recent_info);
    entry.recent_uri = Some(recent_uri);
    let final_location_is_recent = entry
        .location
        .uri_value()
        .is_some_and(|uri| gio::File::for_uri(uri).has_uri_scheme("recent"));
    (!final_location_is_recent).then_some(entry)
}

// Resolve concurrently so one unreachable target cannot consume the batch's deadline.
async fn resolve_recent_batch(
    infos: Vec<(gio::FileInfo, String)>,
    include_metadata: bool,
    deadline: Instant,
) -> Vec<RecentEntryResolution> {
    let context = glib::MainContext::default();
    let pending: Vec<_> = infos
        .into_iter()
        .map(|(info, recent_uri)| {
            context.spawn_local(resolve_recent_entry(
                info,
                recent_uri,
                include_metadata,
                deadline,
            ))
        })
        .collect();
    let mut resolutions = Vec::with_capacity(pending.len());
    for handle in pending {
        resolutions.push(handle.await.unwrap_or(RecentEntryResolution::Stale));
    }
    resolutions
}

async fn resolve_recent_entry(
    recent_info: gio::FileInfo,
    recent_uri: String,
    include_metadata: bool,
    deadline: Instant,
) -> RecentEntryResolution {
    let Some(target_location) = recent_target_location(&recent_info) else {
        return RecentEntryResolution::Stale;
    };
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
        query_entry(target_location, attributes, gio::FileQueryInfoFlags::NONE),
    )
    .await
    {
        Ok(Ok(entry)) => recent_entry_from_target(&recent_info, recent_uri, entry)
            .map(Box::new)
            .map_or(RecentEntryResolution::Stale, RecentEntryResolution::Entry),
        Ok(Err(_)) => RecentEntryResolution::Stale,
        Err(_) => RecentEntryResolution::TimedOut,
    }
}

pub(super) fn trash_thumbnail_path(location: &Location, info: &gio::FileInfo) -> Option<PathBuf> {
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

fn fill_native_entry_metadata(entry: &mut FileEntry) -> std::io::Result<()> {
    let Some(path) = entry.location.native_path() else {
        return Ok(());
    };
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            entry.size = MetadataValue::Unknown;
            entry.modified_unix_seconds = MetadataValue::Unknown;
            entry.mode = MetadataValue::Unknown;
            return Err(error);
        }
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
    Ok(())
}

fn fill_followed_metadata(entry: &mut FileEntry, target: &FolderStamp) {
    entry.size = if target.kind == FileType::Directory {
        MetadataValue::Unknown
    } else {
        MetadataValue::Known(target.size)
    };
    entry.modified_unix_seconds = MetadataValue::Known(target.modified.0);
    entry.mode = MetadataValue::Known(target.mode);
}

fn native_entry(
    path: PathBuf,
    native_name: OsString,
    file_type: FileType,
    is_hidden: bool,
    mounted: Option<u64>,
    include_metadata: bool,
    progress: &Progress,
) -> (FileEntry, bool) {
    let link = file_type == FileType::Symlink;
    let leads_elsewhere = link || mounted.is_some();
    let revisit = leads_elsewhere && revisits(&path);
    let target = (leads_elsewhere && (link || revisit || include_metadata))
        .then(|| {
            progress.reach(&path, mounted, || {
                super::local_operations::folder_stamp(&path, false)
            })
        })
        .flatten()
        .and_then(Result::ok);
    let kind = if link {
        link_target_kind(target.as_ref().map(|target| target.kind))
    } else {
        listed_kind(file_type, &path)
    };
    let cached_details = match (leads_elsewhere, &target) {
        (false, _) => cached_icon_details_for_revisit(&path),
        (true, Some(target)) if revisit => {
            cached_icon_details(&path, IconDetailsFingerprint::of(target))
        }
        (true, _) => None,
    };
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
        recent_uri: None,
    };
    if let Some(details) = cached_details {
        details.apply_to_entry(&mut entry);
    }
    if let Some(target) = target.as_ref().filter(|_| include_metadata) {
        fill_followed_metadata(&mut entry, target);
    }
    (entry, leads_elsewhere)
}

fn scan_native_directory(
    path: &Path,
    request: &DirectoryRequest,
    cancellable: &gio::Cancellable,
    deadline: Instant,
    progress: &Progress,
    stamped: oneshot::Sender<DirectoryStamp>,
) -> NativeEnumeration {
    let held = request.background.then(|| {
        let real = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        let stop = cancellable.clone();
        file_activity::hold([path, &real], move || stop.cancel())
    });
    let hold = match held.transpose() {
        Ok(hold) => hold,
        Err(refused) => return NativeEnumeration::Failed(refused.into()),
    };
    match scan_native_listing(path, request, cancellable, deadline, progress, stamped) {
        NativeEnumeration::Cancelled if hold.as_ref().is_some_and(file_activity::Hold::stopped) => {
            NativeEnumeration::Failed(file_activity::HELD_FOR_OPERATION.to_owned())
        }
        scanned => scanned,
    }
}

fn scan_native_listing(
    path: &Path,
    request: &DirectoryRequest,
    cancellable: &gio::Cancellable,
    deadline: Instant,
    progress: &Progress,
    stamped: oneshot::Sender<DirectoryStamp>,
) -> NativeEnumeration {
    let failed = |error: rustix::io::Errno| {
        NativeEnumeration::Failed(crate::services::io_error_message(&error.into()))
    };
    let stamp = match read_stamp(path, progress) {
        Ok(stamp) => stamp,
        Err(error) => return failed(error),
    };
    let _ = stamped.send(stamp.clone());
    let mounted: HashMap<OsString, u64> = mount_watch::latest()
        .points()
        .filter(|(_, point)| point.parent() == Some(path))
        .filter_map(|(id, point)| Some((point.file_name()?.to_owned(), id)))
        .collect();
    let hidden_names = native_hidden_names(path);
    let mut entries = Vec::with_capacity(1024);
    let mut followed = Vec::with_capacity(1024);
    let mut truncated = false;
    let mut cancelled = false;
    let mut names_digest = 0u64;
    progress.begin_reading();
    let listed = super::local_operations::list_folder(path, &mut |chunk| {
        progress.beat();
        for (native_name, file_type) in chunk {
            if cancellable.is_cancelled() {
                cancelled = true;
                return ControlFlow::Break(());
            }
            if Instant::now() >= deadline || entries.len() == request.max_entries {
                truncated = true;
                return ControlFlow::Break(());
            }
            names_digest = names_digest.wrapping_add(name_hash(&native_name));
            let is_hidden = is_hidden_name(&native_name, &hidden_names);
            let mounted = mounted.get(&native_name).copied();
            let (entry, leads_elsewhere) = native_entry(
                path.join(&native_name),
                native_name,
                file_type,
                is_hidden,
                mounted,
                request.include_metadata,
                progress,
            );
            entries.push(entry);
            followed.push(leads_elsewhere);
        }
        ControlFlow::Continue(())
    });
    if cancelled {
        return NativeEnumeration::Cancelled;
    }
    if let Err(error) = listed {
        return failed(error);
    }

    // `access::can-trash`/`access::can-delete` describe the queried item, not its
    // children. Probe one actual entry so a directory that cannot itself be removed
    // (such as `$HOME`) does not incorrectly hide Trash/delete for the entries it
    // contains. Query the entry itself: a symlink is trashed as a link, and its
    // target may be slow or unreachable to stat.
    let probed_capabilities = entries
        .iter()
        .find(|entry| !mounted.contains_key(&entry.native_name))
        .and_then(|entry| {
            let probed = gio::File::for_path(entry.location.native_path()?).query_info(
                "access::can-trash,access::can-delete",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                Some(cancellable),
            );
            progress.answered(&probed);
            probed.ok()
        });
    let can_trash = match probed_capabilities.as_ref().and_then(info_can_trash) {
        Some(true) if super::trash_support::unsupported_reason_for_directory(path).is_some() => {
            Some(false)
        }
        other => other,
    };
    let can_delete = probed_capabilities.as_ref().and_then(info_can_delete);

    let mut metadata_complete = true;
    if request.include_metadata && !entries.is_empty() && Instant::now() < deadline {
        let width = metadata_fill_width().min(entries.len());
        let chunk = entries.len().div_ceil(width);
        std::thread::scope(|scope| {
            for (piece, followed) in entries.chunks_mut(chunk).zip(followed.chunks(chunk)) {
                let cancellable = cancellable.clone();
                scope.spawn(move || {
                    for (entry, _) in piece
                        .iter_mut()
                        .zip(followed)
                        .filter(|(_, followed)| !**followed)
                    {
                        if cancellable.is_cancelled() || Instant::now() >= deadline {
                            break;
                        }
                        progress.answered(&fill_native_entry_metadata(entry));
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
        stamp: Box::new(stamp),
        entries,
        truncated,
        names_digest,
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
    let task = glib::MainContext::default().spawn_local(async move {
        let deadline = started + request.time_budget;
        let listing = format!(
            "listing with metadata {} up to {}, in the background {}",
            request.include_metadata, request.max_entries, request.background
        );
        let class = match request.background {
            true => Class::Background,
            false => Class::Interactive,
        };
        let folder = path.clone();
        let (stamped, stamp) = oneshot::channel();
        let work =
            mount_lanes::run_joined_in(&folder, listing, class, move |progress, cancellable| {
                scan_native_directory(&path, &request, cancellable, deadline, progress, stamped)
            });
        let mut work = std::pin::pin!(work);
        let early = futures_lite::future::or(async { Err(stamp.await) }, async {
            Ok(work.as_mut().await)
        })
        .await;
        let mut stamp_sent = false;
        let outcome = match early {
            Ok(outcome) => outcome,
            Err(stamp) => {
                if let Ok(stamp) = stamp {
                    emit(DirectoryEvent::Stamp {
                        request_id,
                        stamp: Box::new(stamp),
                    });
                    stamp_sent = true;
                }
                work.await
            }
        };
        let outcome = match outcome {
            Ok(outcome) => Arc::try_unwrap(outcome).unwrap_or_else(|shared| (*shared).clone()),
            Err(Unanswered::TimedOut | Unanswered::Skipped) => {
                tracing::warn!(
                    request_id = request_id.0,
                    "directory load refused: the mount stopped answering"
                );
                emit(DirectoryEvent::Failed {
                    request_id,
                    message: crate::services::io_error_message(&ErrorKind::TimedOut.into()),
                    refused: true,
                });
                return;
            }
            Err(Unanswered::Failed) => {
                emit(DirectoryEvent::Failed {
                    request_id,
                    message: "Native directory worker failed".to_owned(),
                    refused: false,
                });
                return;
            }
        };
        match outcome {
            NativeEnumeration::Complete {
                stamp,
                entries,
                truncated,
                names_digest,
                metadata_complete,
                can_trash,
                can_delete,
            } => {
                if !stamp_sent {
                    emit(DirectoryEvent::Stamp { request_id, stamp });
                }
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
                    emit(DirectoryEvent::ListingDigest {
                        request_id,
                        names_digest,
                    });
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
                    refused: false,
                });
            }
            NativeEnumeration::Cancelled => {}
        }
    });
    LoadHandle::new(move || {
        tracing::debug!(request_id = request_id.0, "directory load cancelled");
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
            // Delete the registry handle, never standard::target-uri (the real file).
            let files_with_uri: Vec<(gio::FileInfo, String)> = files
                .into_iter()
                .map(|info| {
                    let uri = enumerator.child(&info).uri().to_string();
                    (info, uri)
                })
                .collect();
            Ok(Some(
                resolve_recent_batch(files_with_uri, include_metadata, deadline).await,
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
                    refused: false,
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
                        refused: false,
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

trait RecentRemovalBackend {
    fn delete(&self, uri: &str) -> RecentEnumerationFuture<Result<(), String>>;
}

// GVfs recent deletion removes the registry entry without touching its target.
struct GioRecentRemovalBackend;

impl RecentRemovalBackend for GioRecentRemovalBackend {
    fn delete(&self, uri: &str) -> RecentEnumerationFuture<Result<(), String>> {
        let file = gio::File::for_uri(uri);
        Box::pin(async move {
            file.delete_future(glib::Priority::DEFAULT)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecentRemovalState {
    in_flight: Rc<RefCell<HashSet<String>>>,
}

impl RecentRemovalState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

fn recent_remove_entries_with_backend(
    state: &RecentRemovalState,
    backend: &Rc<dyn RecentRemovalBackend>,
    uris: impl IntoIterator<Item = String>,
) {
    for uri in uris {
        if !gio::File::for_uri(&uri).has_uri_scheme("recent") {
            tracing::warn!("refusing to remove a non-recent URI from the registry");
            continue;
        }
        let logged = diagnostic_uri(&uri);
        if !state.in_flight.borrow_mut().insert(uri.clone()) {
            tracing::debug!(uri = %logged, "recent removal already in flight, skipping duplicate");
            continue;
        }
        tracing::debug!(uri = %logged, "removing recent entry");
        let in_flight = state.in_flight.clone();
        let backend = backend.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = backend.delete(&uri).await;
            in_flight.borrow_mut().remove(&uri);
            match result {
                Ok(()) => tracing::debug!(uri = %logged, "removed recent entry"),
                Err(error) => tracing::warn!(
                    uri = %logged,
                    error = %sanitize_failure_message(&error.to_string()),
                    "failed to remove recent entry; leaving it in the list"
                ),
            }
        });
    }
}

pub(crate) fn recent_remove_entries(
    state: &RecentRemovalState,
    uris: impl IntoIterator<Item = String>,
) {
    let backend: Rc<dyn RecentRemovalBackend> = Rc::new(GioRecentRemovalBackend);
    recent_remove_entries_with_backend(state, &backend, uris);
}

impl FileSource for LocalFileSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        crate::util::ui_thread::assert_off();
        if let Some(path) = location.native_path() {
            let metadata = std::fs::metadata(path).map_err(LocationValidationError::from)?;
            if !metadata.is_dir() {
                return Err(LocationValidationError::NotDirectory);
            }
            return std::fs::read_dir(path)
                .map(|_| ())
                .map_err(LocationValidationError::from);
        }

        let file = gio::File::for_uri(
            location
                .uri_value()
                .ok_or(LocationValidationError::InvalidUri)?,
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
        if let Some(path) = location.native_path().map(Path::to_path_buf) {
            let task = glib::MainContext::default().spawn_local(async move {
                emit(super::local_operations::probes::folder(&path).await);
            });
            return LoadHandle::new(move || task.abort());
        }
        let uri = location.uri_value().unwrap_or_default().to_owned();
        let task = glib::MainContext::default().spawn_local(async move {
            let query = gio::File::for_uri(&uri).query_info_future(
                "standard::type",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            );
            let laned =
                mount_lanes::run_remote(mount_lanes::authority(&uri), Class::Interactive, query);
            emit(match laned.await {
                Ok(result) => uri_validation_result(&location, result),
                Err(refused) => Err(super::local_operations::probes::unavailable(refused)),
            });
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
            let class = match request.background {
                true => Class::Background,
                false => Class::Interactive,
            };
            let admitted = mount_lanes::admit_remote(
                mount_lanes::authority(location.uri_value().unwrap_or_default()),
                class,
            );
            let Ok(admitted) = admitted else {
                emit(DirectoryEvent::Failed {
                    request_id,
                    message: crate::services::io_error_message(&ErrorKind::TimedOut.into()),
                    refused: true,
                });
                return;
            };
            let progress = admitted.progress();
            let beat = || progress.beat();
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
            if stamped_remotely(&location) {
                let read_at = SystemTime::now();
                match admitted
                    .call(glib::future_with_timeout(
                        remaining,
                        directory.query_info_future(
                            STAMP_ATTRIBUTES,
                            gio::FileQueryInfoFlags::NONE,
                            glib::Priority::LOW,
                        ),
                    ))
                    .await
                {
                    Ok(Ok(info)) => {
                        beat();
                        emit(DirectoryEvent::Stamp {
                            request_id,
                            stamp: Box::new(remote_stamp(
                                location.uri_value().unwrap_or_default(),
                                &info,
                                read_at,
                            )),
                        });
                    }
                    Ok(Err(error)) => progress.answered(&Err::<(), _>(error)),
                    Err(_) => {
                        finish_truncated(0, "time budget", can_trash, can_delete);
                        return;
                    }
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let attributes = if request.include_metadata {
                FULL_ATTRIBUTES
            } else {
                LIST_ATTRIBUTES
            };
            let enumerator = match admitted
                .call(glib::future_with_timeout(
                    remaining,
                    directory.enumerate_children_future(
                        attributes,
                        gio::FileQueryInfoFlags::NONE,
                        glib::Priority::DEFAULT,
                    ),
                ))
                .await
            {
                Ok(Ok(enumerator)) => {
                    beat();
                    enumerator
                }
                Ok(Err(error)) => {
                    tracing::warn!(
                        request_id = request_id.0,
                        error_domain = ?error.domain(),
                        error_code = error.code(),
                        "directory load failed"
                    );
                    progress.answered(&Err::<(), _>(error.clone()));
                    emit(remote_directory_failure(request_id, &error));
                    return;
                }
                Err(_) => {
                    finish_truncated(0, "time budget", can_trash, can_delete);
                    return;
                }
            };

            let mut total_entries = 0usize;
            let mut names_digest = 0u64;
            let mut first_batch = true;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    finish_truncated(total_entries, "time budget", can_trash, can_delete);
                    break;
                }
                match admitted
                    .call(glib::future_with_timeout(
                        remaining,
                        enumerator
                            .next_files_future(request.batch_size as i32, glib::Priority::DEFAULT),
                    ))
                    .await
                {
                    Ok(Ok(files)) if files.is_empty() => {
                        beat();
                        tracing::info!(
                            request_id = request_id.0,
                            entries = total_entries,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "directory load finished"
                        );
                        emit(DirectoryEvent::ListingDigest {
                            request_id,
                            names_digest,
                        });
                        emit(DirectoryEvent::Finished {
                            request_id,
                            truncated: false,
                            can_trash,
                            can_delete,
                        });
                        break;
                    }
                    Ok(Ok(files)) => {
                        beat();
                        if can_trash.is_none() {
                            can_trash = files.iter().find_map(info_can_trash);
                        }
                        if can_delete.is_none() {
                            can_delete = files.iter().find_map(info_can_delete);
                        }
                        let mut entries: Vec<_> = files
                            .into_iter()
                            .filter_map(|info| {
                                listed_entry(request_id, &location, &directory, info)
                            })
                            .collect();
                        let remaining_capacity = request.max_entries.saturating_sub(total_entries);
                        let entry_budget_exhausted = entries.len() > remaining_capacity;
                        entries.truncate(remaining_capacity);
                        total_entries += entries.len();
                        names_digest = entries.iter().fold(names_digest, |digest, entry| {
                            digest.wrapping_add(name_hash(&entry.native_name))
                        });
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
                        progress.answered(&Err::<(), _>(error.clone()));
                        emit(remote_directory_failure(request_id, &error));
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

    fn start_watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
        report: Rc<dyn Fn(Watching)>,
    ) -> LoadHandle {
        let started = monitors::start(
            &location,
            super::local_operations::Watched::Folder,
            gio::FileMonitorFlags::WATCH_MOVES,
        );
        let connected = Rc::new(RefCell::new(None));
        let slot = connected.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            match started.await {
                Some(monitor) => {
                    slot.replace(Some(connect_monitor(
                        monitor,
                        location,
                        include_hidden,
                        notify,
                    )));
                    report(Watching::Live);
                }
                None => {
                    tracing::debug!(
                        location = %location.diagnostic_path(),
                        "directory monitoring unavailable"
                    );
                    report(Watching::Unavailable);
                }
            }
        });
        LoadHandle::new(move || {
            task.abort();
            connected.take();
        })
    }

    fn responsiveness(&self, location: &Location) -> Responsiveness {
        match location.native_path() {
            Some(path) => mount_lanes::responsiveness(path),
            None if location.is_recent_root() || location.is_camera_photo_root() => {
                Responsiveness::Answering
            }
            None => mount_lanes::remote_responsiveness(mount_lanes::authority(
                location.uri_value().unwrap_or_default(),
            )),
        }
    }

    fn follow_responsiveness(&self, notify: Rc<dyn Fn()>) -> LoadHandle {
        mount_lanes::follow(&notify);
        LoadHandle::new(move || drop(notify))
    }

    fn probe_stamp(&self, location: Location, emit: Rc<dyn Fn(StampAnswer)>) -> LoadHandle {
        if let Some(path) = location.native_path().map(Path::to_path_buf) {
            let task = glib::MainContext::default().spawn_local(async move {
                let folder = path.clone();
                let stamp = mount_lanes::run_joined(
                    &path,
                    "change stamp".to_owned(),
                    move |progress, _| {
                        progress.begin_reading();
                        read_stamp(&folder, progress)
                    },
                )
                .await;
                emit(match stamp.as_deref() {
                    Ok(stamp) => stamp_answer(stamp.clone()),
                    Err(refusal) => unanswered_stamp(*refusal),
                });
            });
            return LoadHandle::new(move || task.abort());
        }
        if !stamped_remotely(&location) {
            emit(StampAnswer::Unsupported);
            return LoadHandle::new(|| {});
        }
        let uri = location.uri_value().unwrap_or_default().to_owned();
        let task = glib::MainContext::default().spawn_local(async move {
            emit(query_remote_stamp(uri, Class::Interactive).await);
        });
        LoadHandle::new(move || task.abort())
    }

    fn check_stamps(
        &self,
        locations: Vec<Location>,
        emit: Rc<dyn Fn(Location, StampAnswer)>,
    ) -> LoadHandle {
        let (native, others): (Vec<_>, Vec<_>) = locations
            .into_iter()
            .partition(|location| location.native_path().is_some());
        let (remote, unsupported): (Vec<_>, Vec<_>) =
            others.into_iter().partition(stamped_remotely);
        for location in unsupported {
            emit(location, StampAnswer::Unsupported);
        }
        let native = (!native.is_empty()).then(|| check_native_stamps(native, emit.clone()));
        let remote = (!remote.is_empty()).then(|| {
            glib::MainContext::default().spawn_local(async move {
                let mut answers = Vec::with_capacity(remote.len());
                for location in &remote {
                    let uri = location.uri_value().unwrap_or_default().to_owned();
                    answers.push(query_remote_stamp(uri, Class::Background).await);
                }
                for (location, answer) in remote.into_iter().zip(answers) {
                    emit(location, answer);
                }
            })
        });
        LoadHandle::new(move || {
            for task in [native, remote].into_iter().flatten() {
                task.abort();
            }
        })
    }

    fn find_by_identity(
        &self,
        parent: Location,
        identity: ItemIdentity,
        emit: Rc<dyn Fn(Option<Location>)>,
    ) -> LoadHandle {
        let Some(directory) = parent.native_path().map(Path::to_path_buf) else {
            emit(None);
            return LoadHandle::new(|| {});
        };
        let task = glib::MainContext::default().spawn_local(async move {
            let found =
                gio::spawn_blocking(move || native_entry_with_identity(&directory, identity))
                    .await
                    .ok()
                    .flatten();
            emit(found.map(Location::local));
        });
        LoadHandle::new(move || task.abort())
    }
}

struct ConnectedMonitor {
    handler: Option<glib::SignalHandlerId>,
    cancelled: Rc<Cell<bool>>,
    pending: Rc<RefCell<HashMap<PendingMonitorKey, PendingMonitorChange>>>,
    timeout: Rc<RefCell<Option<glib::SourceId>>>,
    monitor: monitors::BareMonitor,
}

impl Drop for ConnectedMonitor {
    fn drop(&mut self) {
        self.cancelled.set(true);
        if let Some(source) = self.timeout.take() {
            source.remove();
        }
        self.pending.borrow_mut().clear();
        if let Some(handler) = self.handler.take() {
            self.monitor.monitor().disconnect(handler);
        }
    }
}

fn connect_monitor(
    monitor: monitors::BareMonitor,
    location: Location,
    include_hidden: bool,
    notify: Rc<dyn Fn(DirectoryChange)>,
) -> ConnectedMonitor {
    let cancelled = Rc::new(Cell::new(false));
    let pending = Rc::new(RefCell::new(HashMap::<
        PendingMonitorKey,
        PendingMonitorChange,
    >::new()));
    let timeout = Rc::new(RefCell::new(None::<glib::SourceId>));
    let numbered = Rc::new(RefCell::new(NumberedNames::default()));
    let awaited = RefCell::new(AwaitedHints::default());
    let pending_for_change = pending.clone();
    let timeout_for_change = timeout.clone();
    let cancelled_for_change = cancelled.clone();
    let watched = location.clone();
    let handler = monitor
        .monitor()
        .connect_changed(move |_, file, other_file, event| {
            let change = pending_monitor_change(
                &watched,
                location_for_file(file),
                other_file.and_then(location_for_file),
                event,
            );
            let Some(change) =
                change.and_then(|change| visible_monitor_change(&watched, change, include_hidden))
            else {
                return;
            };
            // Arrivals are learned even while a rescan is pending.
            let admission = numbered.borrow_mut().admit(&change, event);
            match admission {
                NumberedAdmission::Queue => {}
                NumberedAdmission::Skip => return,
                NumberedAdmission::Read(folder) => {
                    read_numbered_names(
                        numbered.clone(),
                        folder,
                        notify.clone(),
                        cancelled_for_change.clone(),
                    );
                    return;
                }
            }
            if !awaited.borrow_mut().needs_query(&change, event) {
                return;
            }
            if pending_for_change.borrow().contains_key(&None) {
                return;
            }
            if matches!(change, PendingMonitorChange::Rescan) {
                tracing::debug!(?event, "directory monitor requested reconciliation");
            }
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

            if timeout_for_change.borrow().is_none() {
                let pending = pending_for_change.clone();
                let timeout = timeout_for_change.clone();
                let notify = notify.clone();
                let cancelled = cancelled_for_change.clone();
                let source = glib::timeout_add_local_once(Duration::from_millis(100), move || {
                    timeout.take();
                    flush_monitor_changes(&pending, &notify, &cancelled);
                });
                timeout_for_change.replace(Some(source));
            }
        });

    ConnectedMonitor {
        handler: Some(handler),
        cancelled,
        pending,
        timeout,
        monitor,
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
    fill_parallel_with_details(
        width,
        request_id,
        locations,
        include_icon_details,
        time_budget,
        emit,
        fill_icon_details,
    )
}

fn fill_parallel_with_details(
    width: usize,
    request_id: RequestId,
    locations: Vec<Location>,
    include_icon_details: bool,
    time_budget: Duration,
    emit: Rc<dyn Fn(DirectoryEvent)>,
    fill_details: impl Fn(
        &mut MetadataUpdate,
        &gio::FileInfo,
        &Location,
        &gio::Cancellable,
        Instant,
    ) -> bool
    + Send
    + Sync
    + 'static,
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
                    let fill_details = &fill_details;
                    handles.push(scope.spawn(move || {
                        let mut updates = Vec::new();
                        let mut pending_details = Vec::new();
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
                            let (update, ok) = match gio::File::for_path(path).query_info(
                                METADATA_ATTRIBUTES,
                                gio::FileQueryInfoFlags::NONE,
                                Some(&cancellable),
                            ) {
                                Ok(info) => {
                                    let (update, ok) = update_from_info(&info, location);
                                    if include_icon_details {
                                        pending_details.push((update.clone(), info, location));
                                    }
                                    (update, ok)
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
                                ),
                            };
                            failed += usize::from(!ok);
                            updates.push(update);
                            if !sent_first || updates.len() >= 8 {
                                let _ = tx.unbounded_send(std::mem::take(&mut updates));
                                sent_first = true;
                            }
                        }
                        if !updates.is_empty() {
                            let _ = tx.unbounded_send(std::mem::take(&mut updates));
                        }
                        for (mut update, info, location) in pending_details {
                            if !fill_details(&mut update, &info, location, &cancellable, deadline) {
                                truncated = true;
                                break;
                            }
                            let _ = tx.unbounded_send(vec![update]);
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
    cancellable: &gio::Cancellable,
) -> Result<crate::sandbox::metadata::MediaMetadata, String> {
    #[cfg(not(test))]
    {
        let cancellation = crate::sandbox::Cancellation::default();
        let cancel = cancellation.clone();
        let handler = cancellable.connect_cancelled(move |_| cancel.cancel());
        let result = crate::sandbox::browser::metadata(path, image, &cancellation);
        if let Some(handler) = handler {
            cancellable.disconnect_cancelled(handler);
        }
        result
    }
    #[cfg(test)]
    {
        if cancellable.is_cancelled() {
            return Err("Metadata probe cancelled".to_owned());
        }
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
    fill_icon_details_with_probe(
        update,
        info,
        location,
        cancellable,
        deadline,
        probe_sandboxed_media_metadata,
    )
}

fn fill_icon_details_with_probe(
    update: &mut MetadataUpdate,
    info: &gio::FileInfo,
    location: &Location,
    cancellable: &gio::Cancellable,
    deadline: Instant,
    probe: impl FnOnce(
        &Path,
        bool,
        &gio::Cancellable,
    ) -> Result<crate::sandbox::metadata::MediaMetadata, String>,
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
        match count_children(path, cancellable, deadline) {
            None => return false,
            Some(Children::Counted(count)) => MetadataValue::Known(count),
            Some(Children::Withheld) => {
                update.image_dimensions = MetadataValue::Unavailable;
                update.duration_seconds = MetadataValue::Unavailable;
                update.child_count = MetadataValue::Unknown;
                return true;
            }
            Some(Children::Unreadable) => {
                update.image_dimensions = MetadataValue::Unavailable;
                update.child_count = MetadataValue::Unavailable;
                update.duration_seconds = MetadataValue::Unavailable;
                if cancellable.is_cancelled() || Instant::now() >= deadline {
                    return false;
                }
                cache_icon_details(path, fingerprint, update);
                return true;
            }
        }
    } else {
        MetadataValue::Unavailable
    };

    if cancellable.is_cancelled() || Instant::now() >= deadline {
        return false;
    }
    let needs_metadata = info.file_type() == gio::FileType::Regular
        && (is_image_path(path) || is_media_path(path))
        && bytes_support_named_family(path);
    if needs_metadata {
        #[cfg(test)]
        if let Ok(mut counts) = media_metadata_probe_counts().lock() {
            *counts.entry(path.to_path_buf()).or_default() += 1;
        }
        let image = is_image_path(path);
        let result = probe(path, image, cancellable);
        if cancellable.is_cancelled() || Instant::now() >= deadline {
            return false;
        }
        match result {
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
            Err(error) if error == file_activity::HELD_FOR_OPERATION => return true,
            Err(_) => {
                update.image_dimensions = MetadataValue::Unavailable;
                update.duration_seconds = MetadataValue::Unavailable;
            }
        }
    } else {
        update.image_dimensions = MetadataValue::Unavailable;
        update.duration_seconds = MetadataValue::Unavailable;
    }
    if cancellable.is_cancelled() || Instant::now() >= deadline {
        return false;
    }
    cache_icon_details(path, fingerprint, update);
    true
}

enum Children {
    Counted(u64),
    Unreadable,
    Withheld,
}

fn count_children(
    path: &Path,
    cancellable: &gio::Cancellable,
    deadline: Instant,
) -> Option<Children> {
    let mut entries = match file_activity::list(path) {
        Ok(entries) => entries,
        Err(error) if file_activity::is_refusal(&error) => return Some(Children::Withheld),
        Err(_) => return Some(Children::Unreadable),
    };
    let mut count = 0u64;
    for entry in entries.by_ref() {
        if cancellable.is_cancelled() || Instant::now() >= deadline {
            return None;
        }
        if entry.is_ok() {
            count = count.saturating_add(1);
        }
    }
    Some(if entries.stopped() {
        Children::Withheld
    } else {
        Children::Counted(count)
    })
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

fn native_entry_with_identity(directory: &Path, identity: ItemIdentity) -> Option<PathBuf> {
    use std::os::unix::fs::DirEntryExt;
    let mut listing = file_activity::list(directory).ok()?;
    let folders: Vec<fs::DirEntry> = listing
        .by_ref()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .collect();
    if listing.stopped() {
        return None;
    }
    let matches = |entry: &&fs::DirEntry| {
        let path = entry.path();
        super::local_operations::folder_stamp(&path, false).is_ok_and(|stamp| {
            directory_stamp(&path, &stamp, SystemTime::now())
                .identity
                .is_some_and(|candidate| identity.same(&candidate))
        })
    };
    // Btrfs subvolumes, overlayfs and FUSE can list a different inode than stat reports.
    folders
        .iter()
        .filter(|entry| entry.ino() == identity.evidence.inode)
        .find(matches)
        .or_else(|| folders.iter().find(matches))
        .map(fs::DirEntry::path)
}

fn monitor_location_is_hidden(location: &Location) -> bool {
    location
        .file_name()
        .is_some_and(|name| name.as_encoded_bytes().first() == Some(&b'.'))
}

fn visible_monitor_change(
    watched: &Location,
    change: PendingMonitorChange,
    include_hidden: bool,
) -> Option<PendingMonitorChange> {
    if include_hidden {
        return Some(change);
    }
    match change {
        // The open folder's own departure counts even when its name is hidden.
        PendingMonitorChange::Remove(location) if location == *watched => {
            Some(PendingMonitorChange::Remove(location))
        }
        PendingMonitorChange::Upsert(location) if monitor_location_is_hidden(&location) => None,
        PendingMonitorChange::Remove(location) if monitor_location_is_hidden(&location) => None,
        PendingMonitorChange::Move { from, to } => {
            match (
                monitor_location_is_hidden(&from),
                monitor_location_is_hidden(&to),
            ) {
                (true, true) => None,
                (true, false) => Some(PendingMonitorChange::Upsert(to)),
                (false, true) => Some(PendingMonitorChange::Remove(from)),
                (false, false) => Some(PendingMonitorChange::Move { from, to }),
            }
        }
        change => Some(change),
    }
}

/// GLib follows every write with `Changed` before the `ChangesDoneHint` that ends the
/// name's changes, so a hint after a `Created` or `Changed` this watch queried adds nothing.
/// A file linked into place, as copies publish files, gets its hint two seconds late.
#[derive(Default)]
struct AwaitedHints(HashSet<Location>);

impl AwaitedHints {
    fn needs_query(&mut self, change: &PendingMonitorChange, event: gio::FileMonitorEvent) -> bool {
        let PendingMonitorChange::Upsert(location) = change else {
            return true;
        };
        if location.native_path().is_none() {
            return true;
        }
        match event {
            gio::FileMonitorEvent::Created | gio::FileMonitorEvent::Changed => {
                self.0.insert(location.clone());
                true
            }
            gio::FileMonitorEvent::ChangesDoneHint => !self.0.remove(location),
            _ => true,
        }
    }
}

/// Writes to an `O_TMPFILE` reach its folder's watch under the name `#<inode>`, which no
/// listing holds, so changes to such names are queried only once the folder shows them.
#[derive(Default)]
struct NumberedNames {
    known: HashSet<OsString>,
    listing: NumberedListing,
}

#[derive(Default)]
enum NumberedListing {
    #[default]
    Unread,
    Reading(HashSet<Location>),
    Read,
    Unavailable,
}

enum NumberedAdmission {
    Queue,
    Skip,
    Read(PathBuf),
}

fn is_numbered_name(name: &OsStr) -> bool {
    match name.as_encoded_bytes() {
        [b'#', digits @ ..] => !digits.is_empty() && digits.iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

fn numbered_name(location: &Location) -> Option<&OsStr> {
    location
        .native_path()
        .and_then(Path::file_name)
        .filter(|name| is_numbered_name(name))
}

impl NumberedNames {
    fn admit(
        &mut self,
        change: &PendingMonitorChange,
        event: gio::FileMonitorEvent,
    ) -> NumberedAdmission {
        let (location, arrived) = match change {
            PendingMonitorChange::Upsert(location) => (
                location,
                matches!(
                    event,
                    gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::MovedIn
                        | gio::FileMonitorEvent::Moved
                        | gio::FileMonitorEvent::Renamed
                ),
            ),
            PendingMonitorChange::Move { to, .. } => (to, true),
            PendingMonitorChange::Remove(_) | PendingMonitorChange::Rescan => {
                return NumberedAdmission::Queue;
            }
        };
        let Some(name) = numbered_name(location) else {
            return NumberedAdmission::Queue;
        };
        if arrived {
            self.known.insert(name.to_owned());
        }
        if self.known.contains(name) {
            return NumberedAdmission::Queue;
        }
        match &mut self.listing {
            NumberedListing::Unread => {
                let Some(folder) = location.native_path().and_then(Path::parent) else {
                    return NumberedAdmission::Queue;
                };
                self.listing = NumberedListing::Reading(HashSet::from([location.clone()]));
                NumberedAdmission::Read(folder.to_path_buf())
            }
            NumberedListing::Reading(held) => {
                held.insert(location.clone());
                NumberedAdmission::Skip
            }
            NumberedListing::Read => NumberedAdmission::Skip,
            NumberedListing::Unavailable => NumberedAdmission::Queue,
        }
    }

    fn finish_reading(&mut self, names: Option<HashSet<OsString>>) -> Vec<Location> {
        let NumberedListing::Reading(held) = std::mem::take(&mut self.listing) else {
            return Vec::new();
        };
        let Some(names) = names else {
            self.listing = NumberedListing::Unavailable;
            return held.into_iter().collect();
        };
        self.known.extend(names);
        self.listing = NumberedListing::Read;
        held.into_iter()
            .filter(|location| {
                numbered_name(location).is_some_and(|name| self.known.contains(name))
            })
            .collect()
    }
}

fn read_numbered_names(
    numbered: Rc<RefCell<NumberedNames>>,
    folder: PathBuf,
    notify: Rc<dyn Fn(DirectoryChange)>,
    cancelled: Rc<Cell<bool>>,
) {
    glib::MainContext::default().spawn_local(async move {
        let names = gio::spawn_blocking(move || numbered_names_in(&folder))
            .await
            .ok()
            .and_then(Result::ok);
        if cancelled.get() {
            return;
        }
        let listed = numbered.borrow_mut().finish_reading(names);
        for location in listed {
            query_monitored_entry(location, None, notify.clone(), cancelled.clone());
        }
    });
}

fn numbered_names_in(folder: &Path) -> std::io::Result<HashSet<OsString>> {
    let mut names = HashSet::new();
    for entry in file_activity::list(folder)? {
        let name = entry?.file_name();
        if is_numbered_name(&name) {
            names.insert(name);
        }
    }
    Ok(names)
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
        tracing::debug!(
            pending = pending.len(),
            "directory monitor burst requested reconciliation"
        );
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
        // An exchange can report arrival before departure; query the name's final state.
        (PendingMonitorChange::Move { .. }, PendingMonitorChange::Upsert(_))
        | (PendingMonitorChange::Upsert(_), PendingMonitorChange::Remove(_)) => existing,
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
    let needs_rescan = {
        let moved_sources: HashSet<_> = changes
            .iter()
            .filter_map(|change| match change {
                PendingMonitorChange::Move { from, .. } => Some(from),
                _ => None,
            })
            .collect();
        // A move's query can finish after the recreated source's query and remove its new row.
        changes.iter().any(|change| match change {
            PendingMonitorChange::Rescan => true,
            PendingMonitorChange::Upsert(location) => moved_sources.contains(location),
            _ => false,
        })
    };
    if needs_rescan {
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

fn log_monitor_metadata_error(location: &Location, error: &glib::Error) {
    tracing::debug!(
        location = %location.diagnostic_path(),
        error = %sanitize_failure_message(&error.to_string()),
        "monitor metadata unavailable"
    );
}

fn query_monitored_entry(
    location: Location,
    moved_from: Option<Location>,
    notify: Rc<dyn Fn(DirectoryChange)>,
    cancelled: Rc<Cell<bool>>,
) {
    glib::MainContext::default().spawn_local(async move {
        let result = query_entry(
            location.clone(),
            FULL_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
        )
        .await;
        if cancelled.get() {
            return;
        }
        match result {
            Ok(entry) => {
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
                log_monitor_metadata_error(&location, &error);
                if !cancelled.get() {
                    notify(DirectoryChange::Rescan);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests;

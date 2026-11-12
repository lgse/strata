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
        MountTable,
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
        MetadataRequest, MetadataUpdate, MountToken, Proof, RequestId, Responsiveness, StampAnswer,
        WatchRequest, Watching, backend_unavailable_message, bytes_support_named_family,
        file_activity, is_hidden_name, is_image_path, is_media_path, link_target_kind, listed_kind,
        native_hidden_names, sanitize_failure_message, symlink_kind,
    },
};
use futures_channel::oneshot;

mod camera_photos;
mod fill;
mod icon_cache;
mod monitor_changes;
mod monitors;
mod native;
mod recent;
mod remote;
mod stamps;

use fill::*;
use icon_cache::*;
use monitor_changes::*;
pub(crate) use monitors::watch;
use native::*;
use recent::*;
pub(crate) use recent::{RecentRemovalState, recent_remove_entries};
use remote::*;
use stamps::*;

const LIST_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::is-symlink,standard::target-uri,access::can-trash,access::can-delete";
const FULL_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::is-symlink,standard::size,standard::target-uri,time::created,time::modified,unix::mode,access::can-trash,access::can-delete";
const RECENT_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::target-uri,recent::modified";
const METADATA_ATTRIBUTES: &str =
    "standard::type,standard::size,time::created,time::modified,unix::mode";
const STAMP_ATTRIBUTES: &str =
    "standard::type,time::modified,time::modified-usec,time::changed,time::changed-usec";

#[derive(Default)]
pub struct LocalFileSource;

fn uri_validation_result(
    location: &Location,
    result: Result<gio::FileInfo, glib::Error>,
) -> Result<(), LocationValidationError> {
    let info = result.map_err(|error| {
        if error.matches(gio::IOErrorEnum::NotMounted) {
            LocationValidationError::NotMounted(location.clone())
        } else if location.is_administrator() && error.matches(gio::IOErrorEnum::PermissionDenied) {
            LocationValidationError::NotAuthorized
        } else if error.matches(gio::IOErrorEnum::NotSupported) {
            LocationValidationError::BackendUnavailable(backend_unavailable_message(
                location.uri_value().unwrap_or_default(),
            ))
        } else {
            LocationValidationError::Unavailable(crate::services::error_detail(
                crate::services::gio_backend_error_message(&error),
            ))
        }
    })?;
    match info.file_type() {
        gio::FileType::Directory => Ok(()),
        gio::FileType::Mountable => Err(LocationValidationError::Mountable(location.clone())),
        _ => Err(LocationValidationError::NotDirectory),
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

fn name_hash(name: &OsStr) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    name.as_encoded_bytes().hash(&mut hasher);
    hasher.finish()
}

fn remote_directory_failure(request_id: RequestId, error: &glib::Error) -> DirectoryEvent {
    DirectoryEvent::Failed {
        request_id,
        message: crate::services::gio_backend_error_message(error),
        refused: false,
        denied: false,
    }
}

pub(crate) fn info_is_hidden(info: &gio::FileInfo) -> bool {
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
    let is_computer = location
        .uri_value()
        .is_some_and(|uri| crate::model::uri_scheme_eq(uri, "computer"));
    let recent_uri = is_computer.then(|| location.uri_value().unwrap_or("computer:///").to_owned());
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
    let (size, child_count) = if is_computer {
        let usage = location
            .native_path()
            .and_then(crate::adapters::filesystem_usage);
        if let Some((total, available)) = usage {
            (MetadataValue::Known(total), MetadataValue::Known(available))
        } else {
            (MetadataValue::Unknown, MetadataValue::Unknown)
        }
    } else {
        let s = if matches!(
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
        (s, MetadataValue::Unknown)
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
        created_unix_seconds: match info.creation_date_time() {
            Some(created) => MetadataValue::Known(created.to_unix()),
            // Listings that read times request both, so only birth time is unsupported.
            None if info.has_attribute(gio::FILE_ATTRIBUTE_TIME_MODIFIED) => {
                MetadataValue::Unavailable
            }
            None => MetadataValue::Unknown,
        },
        recent_unix_seconds: MetadataValue::Unknown,
        mode: info_mode(&info),
        image_dimensions: MetadataValue::Unknown,
        child_count,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: info_is_hidden(&info),
        recent_uri,
    }
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
            entry.created_unix_seconds = MetadataValue::Unknown;
            entry.mode = MetadataValue::Unknown;
            entry.listable = MetadataValue::Unknown;
            return Err(error);
        }
    };
    entry.listable = folder_listable(path, metadata.is_dir());
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
    entry.created_unix_seconds = metadata
        .created()
        .ok()
        .and_then(unix_seconds)
        .map(MetadataValue::Known)
        .unwrap_or(MetadataValue::Unavailable);
    entry.mode = MetadataValue::Known(metadata.mode());
    Ok(())
}

fn fill_followed_metadata(entry: &mut FileEntry, target: &FolderStamp) {
    let directory = target.kind == FileType::Directory;
    entry.size = if directory {
        MetadataValue::Unknown
    } else {
        MetadataValue::Known(target.size)
    };
    entry.modified_unix_seconds = MetadataValue::Known(target.modified.0);
    entry.created_unix_seconds = target
        .birth
        .map(|time| MetadataValue::Known(time.0))
        .unwrap_or(MetadataValue::Unavailable);
    entry.mode = MetadataValue::Known(target.mode);
    if let Some(path) = entry.location.native_path() {
        entry.listable = folder_listable(path, directory);
    }
}

/// Like GIO's `access::can-read` and `access::can-execute` together, so ACLs count.
fn folder_listable(path: &Path, directory: bool) -> MetadataValue<bool> {
    use rustix::fs::Access;
    if !directory {
        return MetadataValue::Unavailable;
    }
    match rustix::fs::access(path, Access::READ_OK | Access::EXEC_OK) {
        Ok(()) => MetadataValue::Known(true),
        Err(rustix::io::Errno::ACCESS | rustix::io::Errno::PERM) => MetadataValue::Known(false),
        Err(_) => MetadataValue::Unavailable,
    }
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

        enumerate_remote(request, emit, started)
    }

    fn offers_administrator_access(&self) -> bool {
        crate::model::administrator_access_available()
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
        let mut locations = request.entries;
        if !request.full {
            locations.truncate(MAX_FILL_ENTRIES);
        }
        let all_native = locations
            .iter()
            .all(|location| location.native_path().is_some());
        let fill = FillRequest {
            request_id: request.id,
            locations,
            include_icon_details: request.include_icon_details,
            time_budget: request.time_budget,
        };
        if all_native && !fill.locations.is_empty() {
            return fill_parallel(fill, emit);
        }
        fill_queried(fill, emit)
    }

    fn start_watch(
        &self,
        request: WatchRequest,
        notify: Rc<dyn Fn(DirectoryChange)>,
        report: Rc<dyn Fn(Watching)>,
    ) -> LoadHandle {
        let started = monitors::start(
            &request.location,
            super::local_operations::Watched::Folder,
            gio::FileMonitorFlags::WATCH_MOVES,
        );
        let connected = Rc::new(RefCell::new(None));
        let slot = connected.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            match started.await {
                Some(monitor) => {
                    slot.replace(Some(connect_monitor(monitor, request, notify)));
                    report(Watching::Live);
                }
                None => {
                    tracing::debug!(
                        location = %request.location.diagnostic_path(),
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
            None if location.is_recent_root()
                || location.is_camera_photo_root()
                || location.is_computer_root() =>
            {
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
        departed: Location,
        identity: ItemIdentity,
        emit: Rc<dyn Fn(Option<Location>)>,
    ) -> LoadHandle {
        let Some(departed) = departed.native_path().map(Path::to_path_buf) else {
            emit(None);
            return LoadHandle::new(|| {});
        };
        let (_, mounts) = mount_watch::snapshot();
        let job = mount_lanes::run(&departed.clone(), Class::Interactive, move |progress| {
            native_relocated_folder(&departed, identity, &mounts, progress)
        });
        let task = glib::MainContext::default().spawn_local(async move {
            emit(job.await.ok().flatten().map(Location::local));
        });
        LoadHandle::new(move || task.abort())
    }
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

/// Where the folder last seen at `departed` is now: renamed in place, or moved along with
/// its deepest missing ancestor renamed within the nearest ancestor that remains, a rename
/// that inotify does not report to the folder's own watch.
fn native_relocated_folder(
    departed: &Path,
    identity: ItemIdentity,
    mounts: &MountTable,
    progress: &Progress,
) -> Option<PathBuf> {
    let mut renamed = departed;
    let directory = loop {
        let parent = renamed.parent()?;
        let metadata = fs::metadata(parent);
        progress.answered(&metadata);
        match metadata {
            Ok(metadata) if metadata.is_dir() => break parent,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(_) => return None,
        }
        renamed = parent;
    };
    let below = departed.strip_prefix(renamed).ok()?;
    if below.as_os_str().is_empty() {
        return native_entry_with_identity(directory, identity, mounts, progress);
    }
    native_folders(directory, below, mounts, progress)?
        .iter()
        .map(|folder| folder.path().join(below))
        .find(|candidate| has_identity(candidate, identity, progress))
}

/// The folders of `directory` whose path `below` stays on the mount of `directory`, judged
/// from its real path by the mount table: a mount point cannot be renamed, and looking into
/// another mount can wait on a server that stopped answering or trigger an automount.
fn native_folders(
    directory: &Path,
    below: &Path,
    mounts: &MountTable,
    progress: &Progress,
) -> Option<Vec<fs::DirEntry>> {
    let real = fs::canonicalize(directory);
    progress.answered(&real);
    let real = real.ok()?;
    let mount = mounts.mount_point_for(&real);
    let stays_on_the_mount = |entry: &fs::DirEntry| {
        mounts.mount_point_for(&real.join(entry.file_name()).join(below)) == mount
    };
    let mut listing = file_activity::list(directory).ok()?;
    let folders = listing
        .by_ref()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(stays_on_the_mount)
        .collect();
    progress.beat();
    (!listing.stopped()).then_some(folders)
}

/// The identity of the folder at `path` under its mount's identity rule.
fn folder_identity_at(
    path: &Path,
    progress: &Progress,
) -> Result<Option<ItemIdentity>, rustix::io::Errno> {
    let stamp = super::local_operations::folder_stamp(path, false);
    progress.answered(&stamp);
    Ok(directory_stamp(path, &stamp?, SystemTime::now()).identity)
}

fn has_identity(path: &Path, identity: ItemIdentity, progress: &Progress) -> bool {
    folder_identity_at(path, progress)
        .ok()
        .flatten()
        .is_some_and(|candidate| identity.same(&candidate))
}

fn native_entry_with_identity(
    directory: &Path,
    identity: ItemIdentity,
    mounts: &MountTable,
    progress: &Progress,
) -> Option<PathBuf> {
    use std::os::unix::fs::DirEntryExt;
    let folders = native_folders(directory, Path::new(""), mounts, progress)?;
    let matches = |entry: &&fs::DirEntry| has_identity(&entry.path(), identity, progress);
    // Btrfs subvolumes, overlayfs and FUSE can list a different inode than stat reports.
    folders
        .iter()
        .filter(|entry| entry.ino() == identity.evidence.inode)
        .find(matches)
        .or_else(|| folders.iter().find(matches))
        .map(fs::DirEntry::path)
}

#[cfg(test)]
mod tests;

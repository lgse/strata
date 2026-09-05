// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs,
    io::{ErrorKind, Read},
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
        LocationValidationError, MetadataOutcome, MetadataRequest, MetadataUpdate, RequestId,
        backend_unavailable_message, sanitize_uri_credentials,
    },
};

const LIST_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::is-symlink,access::can-trash,access::can-delete";
const FULL_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::is-symlink,standard::size,time::modified,unix::mode,access::can-trash,access::can-delete";
const METADATA_ATTRIBUTES: &str = "standard::type,standard::size,time::modified,unix::mode";
const MAX_PENDING_MONITOR_CHANGES: usize = 256;
const MAX_HIDDEN_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Default)]
pub struct LocalFileSource;

#[derive(Clone)]
enum PendingMonitorChange {
    Upsert(PathBuf),
    Remove(PathBuf),
    Move { from: PathBuf, to: PathBuf },
    Rescan,
}

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

fn map_validation_error(error: std::io::Error) -> LocationValidationError {
    match error.kind() {
        ErrorKind::NotFound => LocationValidationError::Missing,
        ErrorKind::PermissionDenied => LocationValidationError::Inaccessible,
        _ => LocationValidationError::Unavailable(error.to_string()),
    }
}

/// Builds a `Location` for a `gio::File`, preferring a native path only when
/// the file is genuinely on a local filesystem. A mounted GVfs backend (SMB,
/// SFTP, ...) can still return a `.path()` via its FUSE mirror even though the
/// file isn't native; using that path would leak the mirror's opaque
/// `/run/user/$UID/gvfs/...` location instead of the clean URI (lgse/strata#5).
/// Returns `None` when GIO provides a malformed URI.
pub(crate) fn location_for_file(file: &gio::File) -> Option<Location> {
    if file.is_native()
        && let Some(path) = file.path()
    {
        return Some(Location::local(path));
    }
    let uri = file.uri();
    let (sanitized, _) = sanitize_uri_credentials(&uri).ok()?;
    Some(Location::uri(sanitized))
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

fn entry_from_info(location: Location, info: gio::FileInfo) -> FileEntry {
    let native_name = info.name().into_os_string();
    let kind = match (info.file_type(), info_is_symlink(&info)) {
        (gio::FileType::Directory, true) => EntryKind::DirectorySymbolicLink,
        (gio::FileType::Regular, true) => EntryKind::FileSymbolicLink,
        // GVfs reports unmounted browsable children (an smb:// host's shares, a
        // "Connect to Server" bookmark, ...) as `Mountable` rather than
        // `Directory`. Treat them as directories so activation descends into
        // them (and can trigger the mount-and-retry flow) instead of asking
        // the desktop to "open" the location in a new application instance.
        (gio::FileType::Directory | gio::FileType::Mountable, false) => EntryKind::Directory,
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
        location,
        native_name,
        display_name: info.display_name().to_string(),
        kind,
        size,
        modified_unix_seconds,
        mode: info_mode(&info),
        is_hidden: info_is_hidden(&info),
    }
}

fn native_kind(file_type: fs::FileType, path: &Path) -> EntryKind {
    if file_type.is_dir() {
        return EntryKind::Directory;
    }
    if file_type.is_file() {
        return EntryKind::File;
    }
    if !file_type.is_symlink() {
        return EntryKind::Other;
    }
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => EntryKind::DirectorySymbolicLink,
        Ok(metadata) if metadata.is_file() => EntryKind::FileSymbolicLink,
        Ok(_) => EntryKind::Other,
        Err(_) => EntryKind::SymbolicLink,
    }
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

fn native_hidden_names(path: &Path) -> HashSet<OsString> {
    let Some(bytes) = read_hidden_bounded(&path.join(".hidden")) else {
        return HashSet::new();
    };
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|name| !name.is_empty())
        .map(|name| OsString::from_vec(name.strip_suffix(b"\r").unwrap_or(name).to_vec()))
        .collect()
}

fn read_hidden_bounded(path: &Path) -> Option<Vec<u8>> {
    // Opening a FIFO without `NONBLOCK` would block waiting for a writer.
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let stat = rustix::fs::fstat(&fd).ok()?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
        return None;
    }
    let file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    file.take(MAX_HIDDEN_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_HIDDEN_FILE_BYTES {
        return None;
    }
    Some(bytes)
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
        let is_hidden = native_name.as_encoded_bytes().first().copied() == Some(b'.')
            || hidden_names.contains(&native_name);
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
        entries.push(FileEntry {
            location: Location::local(path),
            display_name: native_name.to_string_lossy().into_owned(),
            native_name,
            kind,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            is_hidden,
        });
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
        let width = sort_fill_width().min(entries.len());
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

        let task = glib::MainContext::default().spawn_local(async move {
            let directory = location
                .native_path()
                .map(gio::File::for_path)
                .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()));
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
        let mut locations = request.entries;
        if !request.full {
            locations.truncate(MAX_FILL_ENTRIES);
        }
        let all_native = locations
            .iter()
            .all(|location| location.native_path().is_some());
        if request.full && all_native && !locations.is_empty() {
            return fill_parallel(request_id, locations, request.time_budget, emit);
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
                    Ok(Ok(info)) => update_from_info(&info, location),
                    Ok(Err(_)) => (
                        MetadataUpdate {
                            location: location.clone(),
                            size: MetadataValue::Unknown,
                            modified_unix_seconds: MetadataValue::Unknown,
                            mode: MetadataValue::Unknown,
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
        let path = location.native_path()?.to_path_buf();
        let file = gio::File::for_path(&path);
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
        let pending = Rc::new(RefCell::new(HashMap::<PathBuf, PendingMonitorChange>::new()));
        let timeout = Rc::new(RefCell::new(None::<glib::SourceId>));
        let pending_for_change = pending.clone();
        let timeout_for_change = timeout.clone();
        let cancelled_for_change = cancelled.clone();
        monitor.connect_changed(move |_, file, other_file, event| {
            if pending_for_change.borrow().contains_key(Path::new("")) {
                return;
            }
            let path = file.path();
            let other_path = other_file.and_then(gio::File::path);
            let change = match event {
                gio::FileMonitorEvent::Deleted | gio::FileMonitorEvent::MovedOut => {
                    path.clone().map(PendingMonitorChange::Remove)
                }
                gio::FileMonitorEvent::Created | gio::FileMonitorEvent::MovedIn => {
                    path.clone().map(PendingMonitorChange::Upsert)
                }
                gio::FileMonitorEvent::Changed
                | gio::FileMonitorEvent::ChangesDoneHint
                | gio::FileMonitorEvent::AttributeChanged => {
                    path.clone().map(PendingMonitorChange::Upsert)
                }
                gio::FileMonitorEvent::Moved | gio::FileMonitorEvent::Renamed => path
                    .clone()
                    .zip(other_path)
                    .map(|(from, to)| PendingMonitorChange::Move { from, to }),
                gio::FileMonitorEvent::PreUnmount | gio::FileMonitorEvent::Unmounted => {
                    Some(PendingMonitorChange::Rescan)
                }
                _ => Some(PendingMonitorChange::Rescan),
            };
            let Some(change) = change else {
                return;
            };
            let key = match &change {
                PendingMonitorChange::Upsert(path) | PendingMonitorChange::Remove(path) => {
                    path.clone()
                }
                PendingMonitorChange::Move { to, .. } => to.clone(),
                PendingMonitorChange::Rescan => PathBuf::new(),
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

fn sort_fill_width() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().min(8))
        .unwrap_or(4)
        .max(1)
}
fn fill_parallel(
    request_id: RequestId,
    locations: Vec<Location>,
    time_budget: Duration,
    emit: Rc<dyn Fn(DirectoryEvent)>,
) -> LoadHandle {
    fill_parallel_with(sort_fill_width(), request_id, locations, time_budget, emit)
}

fn fill_parallel_with(
    width: usize,
    request_id: RequestId,
    locations: Vec<Location>,
    time_budget: Duration,
    emit: Rc<dyn Fn(DirectoryEvent)>,
) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let cancelled = cancellable.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        let deadline = Instant::now() + time_budget;
        let outcome = gio::spawn_blocking(move || {
            let width = width.max(1);
            let chunk = locations.len().div_ceil(width);
            let mut updates = Vec::with_capacity(locations.len());
            let mut attempted = 0usize;
            let mut failed = 0usize;
            let mut truncated = false;
            std::thread::scope(|scope| {
                let mut handles = Vec::new();
                for piece in locations.chunks(chunk.max(1)) {
                    let cancellable = cancellable.clone();
                    handles.push(scope.spawn(move || {
                        let mut updates = Vec::with_capacity(piece.len());
                        let mut attempted = 0usize;
                        let mut failed = 0usize;
                        let mut truncated = false;
                        for location in piece {
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
                                Ok(info) => update_from_info(&info, location),
                                Err(_) => (
                                    MetadataUpdate {
                                        location: location.clone(),
                                        size: MetadataValue::Unknown,
                                        modified_unix_seconds: MetadataValue::Unknown,
                                        mode: MetadataValue::Unknown,
                                    },
                                    false,
                                ),
                            };
                            failed += usize::from(!ok);
                            updates.push(update);
                        }
                        (updates, attempted, failed, truncated)
                    }));
                }
                for handle in handles {
                    let Ok((piece, attempted_piece, failed_piece, truncated_piece)) = handle.join()
                    else {
                        truncated = true;
                        continue;
                    };
                    updates.extend(piece);
                    attempted += attempted_piece;
                    failed += failed_piece;
                    truncated = truncated || truncated_piece;
                }
            });
            (updates, attempted, failed, truncated, locations.len())
        })
        .await;
        let Ok((updates, attempted, failed, truncated, total)) = outcome else {
            return;
        };
        if cancelled.is_cancelled() {
            emit(DirectoryEvent::MetadataFinished {
                request_id,
                outcome: MetadataOutcome::Cancelled,
            });
            return;
        }
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
    });
    LoadHandle::new(move || {
        cancel.cancel();
        task.abort();
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

fn queue_monitor_change(
    pending: &mut HashMap<PathBuf, PendingMonitorChange>,
    key: PathBuf,
    change: PendingMonitorChange,
) -> bool {
    if pending.contains_key(Path::new("")) {
        return false;
    }
    pending
        .entry(key)
        .and_modify(|pending| {
            *pending = merge_pending_change(pending.clone(), change.clone());
        })
        .or_insert(change);
    if pending.len() > MAX_PENDING_MONITOR_CHANGES {
        pending.clear();
        pending.insert(PathBuf::new(), PendingMonitorChange::Rescan);
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
    pending: &RefCell<HashMap<PathBuf, PendingMonitorChange>>,
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
            PendingMonitorChange::Remove(path) => {
                notify(DirectoryChange::Remove(Location::local(path)));
            }
            PendingMonitorChange::Upsert(path) => {
                query_monitored_entry(path, None, notify.clone(), cancelled.clone())
            }
            PendingMonitorChange::Move { from, to } => {
                query_monitored_entry(to, Some(from), notify.clone(), cancelled.clone())
            }
            PendingMonitorChange::Rescan => {}
        }
    }
}

fn query_monitored_entry(
    path: PathBuf,
    moved_from: Option<PathBuf>,
    notify: Rc<dyn Fn(DirectoryChange)>,
    cancelled: Rc<Cell<bool>>,
) {
    glib::MainContext::default().spawn_local(async move {
        let file = gio::File::for_path(&path);
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
                let entry = entry_from_info(Location::local(path), info);
                if let Some(from) = moved_from {
                    notify(DirectoryChange::Move {
                        from: Location::local(from),
                        entry,
                    });
                } else {
                    notify(DirectoryChange::Upsert(entry));
                }
            }
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => {
                let removed = moved_from.unwrap_or(path);
                if !cancelled.get() {
                    notify(DirectoryChange::Remove(Location::local(removed)));
                }
            }
            Err(error) => {
                tracing::debug!(path = %path.display(), error = %error, "monitor metadata unavailable");
                if !cancelled.get() {
                    notify(DirectoryChange::Rescan);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests;

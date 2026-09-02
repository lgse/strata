// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::ErrorKind,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
        LocationValidationError, RequestId, backend_unavailable_message, sanitize_uri_credentials,
    },
};

const ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-hidden,standard::is-symlink,standard::size,time::modified";

#[derive(Default)]
pub struct LocalFileSource;

#[derive(Clone)]
enum PendingMonitorChange {
    Upsert(PathBuf),
    Remove(PathBuf),
    Move { from: PathBuf, to: PathBuf },
    Rescan,
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
    FileEntry {
        location,
        native_name,
        display_name: info.display_name().to_string(),
        kind,
        size: if matches!(
            kind,
            EntryKind::Directory | EntryKind::DirectorySymbolicLink
        ) {
            MetadataValue::Unknown
        } else {
            u64::try_from(info.size())
                .map(MetadataValue::Known)
                .unwrap_or(MetadataValue::Unavailable)
        },
        modified_unix_seconds: info
            .modification_date_time()
            .map(|modified| MetadataValue::Known(modified.to_unix()))
            .unwrap_or(MetadataValue::Unavailable),
    }
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

        let task = glib::MainContext::default().spawn_local(async move {
            let deadline = started + request.time_budget;
            let finish_truncated = |entries: usize, reason: &'static str| {
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
                });
            };
            let directory = location
                .native_path()
                .map(gio::File::for_path)
                .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()));
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                finish_truncated(0, "time budget");
                return;
            }
            let enumerator = match glib::future_with_timeout(
                remaining,
                directory.enumerate_children_future(
                    ATTRIBUTES,
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
                    finish_truncated(0, "time budget");
                    return;
                }
            };

            let mut total_entries = 0usize;
            let mut first_batch = true;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    finish_truncated(total_entries, "time budget");
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
                        });
                        break;
                    }
                    Ok(Ok(files)) => {
                        let mut entries: Vec<_> = files
                            .into_iter()
                            .filter(|info| request.include_hidden || !info_is_hidden(info))
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
                            finish_truncated(total_entries, "entry budget");
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
                        finish_truncated(total_entries, "time budget");
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

    fn watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
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
            pending_for_change
                .borrow_mut()
                .entry(key)
                .and_modify(|pending| {
                    *pending = merge_pending_change(pending.clone(), change.clone());
                })
                .or_insert(change);

            if let Some(source) = timeout_for_change.take() {
                source.remove();
            }
            let pending = pending_for_change.clone();
            let timeout = timeout_for_change.clone();
            let notify = notify.clone();
            let cancelled = cancelled_for_change.clone();
            let source = glib::timeout_add_local_once(Duration::from_millis(100), move || {
                timeout.take();
                flush_monitor_changes(&pending, include_hidden, &notify, &cancelled);
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
    include_hidden: bool,
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
            PendingMonitorChange::Upsert(path) => query_monitored_entry(
                path,
                include_hidden,
                None,
                notify.clone(),
                cancelled.clone(),
            ),
            PendingMonitorChange::Move { from, to } => query_monitored_entry(
                to,
                include_hidden,
                Some(from),
                notify.clone(),
                cancelled.clone(),
            ),
            PendingMonitorChange::Rescan => {}
        }
    }
}

fn query_monitored_entry(
    path: PathBuf,
    include_hidden: bool,
    moved_from: Option<PathBuf>,
    notify: Rc<dyn Fn(DirectoryChange)>,
    cancelled: Rc<Cell<bool>>,
) {
    glib::MainContext::default().spawn_local(async move {
        let file = gio::File::for_path(&path);
        let result = file
            .query_info_future(
                ATTRIBUTES,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await;
        if cancelled.get() {
            return;
        }
        match result {
            Ok(info) if include_hidden || !info_is_hidden(&info) => {
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
            Ok(_) => {
                if let Some(from) = moved_from {
                    notify(DirectoryChange::Remove(Location::local(from)));
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

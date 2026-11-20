// SPDX-License-Identifier: MIT
//! The `recent:///` registry: listing it and removing entries from it.

use super::*;

pub(super) enum RecentEntryResolution {
    Entry(Box<FileEntry>),
    Stale,
    TimedOut,
}

pub(super) type RecentEnumerationFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;

pub(super) enum RecentSourceError {
    Failed(String),
    TimedOut,
}

pub(super) trait RecentEnumerationSource {
    fn open(&self, deadline: Instant) -> RecentEnumerationFuture<Result<(), RecentSourceError>>;

    fn next_batch(
        &self,
        batch_size: usize,
        include_metadata: bool,
        deadline: Instant,
    ) -> RecentEnumerationFuture<Result<Option<Vec<RecentEntryResolution>>, RecentSourceError>>;
}

pub(super) fn recent_unix_seconds(info: &gio::FileInfo) -> MetadataValue<i64> {
    if info.has_attribute(gio::FILE_ATTRIBUTE_RECENT_MODIFIED) {
        MetadataValue::Known(info.attribute_int64(gio::FILE_ATTRIBUTE_RECENT_MODIFIED))
    } else {
        MetadataValue::Unknown
    }
}

pub(super) fn recent_target_location(info: &gio::FileInfo) -> Option<Location> {
    let target_uri = info.attribute_string(gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI)?;
    let target_file = gio::File::for_uri(target_uri.as_str());
    if target_file.has_uri_scheme("recent") {
        return None;
    }
    location_for_file(&target_file)
}

pub(super) fn recent_entry_from_target(
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
pub(super) async fn resolve_recent_batch(
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

pub(super) fn enumerate_recent(
    request: DirectoryRequest,
    emit: Rc<dyn Fn(DirectoryEvent)>,
    started: Instant,
) -> LoadHandle {
    let source = Box::new(GioRecentEnumerationSource::new(gio_file_for_location(
        &request.location,
    )));
    enumerate_recent_with_source(request, emit, started, source)
}

pub(super) struct GioRecentEnumerationSource {
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

pub(super) fn enumerate_recent_with_source(
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
                    denied: false,
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
                        denied: false,
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

pub(super) trait RecentRemovalBackend {
    fn delete(&self, uri: &str) -> RecentEnumerationFuture<Result<(), String>>;
}

// GVfs recent deletion removes the registry entry without touching its target.
pub(super) struct GioRecentRemovalBackend;

#[derive(Clone, Default)]
pub struct RecentRemovalState {
    pub(super) in_flight: Rc<RefCell<HashSet<String>>>,
}

impl RecentRemovalState {
    pub fn new() -> Self {
        Self::default()
    }
}

pub(super) fn recent_remove_entries_with_backend(
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

pub fn recent_remove_entries(state: &RecentRemovalState, uris: impl IntoIterator<Item = String>) {
    let backend: Rc<dyn RecentRemovalBackend> = Rc::new(GioRecentRemovalBackend);
    recent_remove_entries_with_backend(state, &backend, uris);
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

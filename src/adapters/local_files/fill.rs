// SPDX-License-Identifier: MIT
//! Filling metadata and icon details for entries already listed.

use super::*;

pub(super) fn metadata_fill_width() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().min(8))
        .unwrap_or(4)
        .max(1)
}

pub(super) fn fill_parallel(
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

pub(super) fn fill_parallel_with(
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

pub(super) fn fill_parallel_with_details(
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
                                        listable: MetadataValue::Unknown,
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
pub(super) fn media_metadata_probe_counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
    static COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
pub(super) fn media_metadata_probe_count(path: &Path) -> usize {
    media_metadata_probe_counts()
        .lock()
        .ok()
        .and_then(|counts| counts.get(path).copied())
        .unwrap_or(0)
}

pub(super) fn probe_sandboxed_media_metadata(
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

pub(super) fn fill_icon_details(
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

pub(super) fn fill_icon_details_with_probe(
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

pub(super) enum Children {
    Counted(u64),
    Unreadable,
    Withheld,
}

pub(super) fn count_children(
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

pub(super) fn update_from_info(
    info: &gio::FileInfo,
    location: &Location,
) -> (MetadataUpdate, bool) {
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
    let listable = match location.native_path() {
        Some(path) => folder_listable(path, info.file_type() == gio::FileType::Directory),
        None => MetadataValue::Unknown,
    };
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
            listable,
        },
        ok,
    )
}

pub(super) fn emit_fill_outcome(
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

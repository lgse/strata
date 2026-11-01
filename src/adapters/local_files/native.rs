// SPDX-License-Identifier: MIT
//! Listing native folders through the mount lanes.

use super::*;

#[derive(Clone)]
pub(super) enum NativeEnumeration {
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
    /// The folder exists but its permissions keep this user out.
    Denied(String),
    Cancelled,
}

pub(super) fn native_entry(
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
                crate::adapters::local_operations::folder_stamp(&path, false)
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
        listable: MetadataValue::Unknown,
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

pub(super) fn scan_native_directory(
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

pub(super) fn scan_native_listing(
    path: &Path,
    request: &DirectoryRequest,
    cancellable: &gio::Cancellable,
    deadline: Instant,
    progress: &Progress,
    stamped: oneshot::Sender<DirectoryStamp>,
) -> NativeEnumeration {
    let failed = |error: rustix::io::Errno| {
        let message = crate::services::io_error_message(&error.into());
        if matches!(error, rustix::io::Errno::ACCESS | rustix::io::Errno::PERM) {
            NativeEnumeration::Denied(message)
        } else {
            NativeEnumeration::Failed(message)
        }
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
    let listed = crate::adapters::local_operations::list_folder(path, &mut |chunk| {
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
        Some(true)
            if crate::adapters::trash_support::unsupported_reason_for_directory(path).is_some() =>
        {
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

pub(super) fn enumerate_native(
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
                    denied: false,
                });
                return;
            }
            Err(Unanswered::Failed) => {
                emit(DirectoryEvent::Failed {
                    request_id,
                    message: "Native directory worker failed".to_owned(),
                    refused: false,
                    denied: false,
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
                    denied: false,
                });
            }
            NativeEnumeration::Denied(message) => {
                tracing::info!(request_id = request_id.0, "directory load denied");
                emit(DirectoryEvent::Failed {
                    request_id,
                    message,
                    refused: false,
                    denied: true,
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

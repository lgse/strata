// SPDX-License-Identifier: MIT
//! Listing native folders through the mount lanes.

use super::*;

/// A finished native listing.
#[derive(Clone)]
pub(super) struct NativeListing {
    stamp: Box<DirectoryStamp>,
    entries: Vec<FileEntry>,
    truncated: bool,
    names_digest: u64,
    metadata_complete: bool,
    can_trash: Option<bool>,
    can_delete: Option<bool>,
}

#[derive(Clone)]
pub(super) enum NativeEnumeration {
    Complete(NativeListing),
    Failed(String),
    /// The folder exists but its permissions keep this user out.
    Denied(String),
    Cancelled,
}

/// One name read from a native folder.
struct ListedName {
    path: PathBuf,
    native_name: OsString,
    file_type: FileType,
    is_hidden: bool,
    mounted: Option<u64>,
}

fn bare_entry(path: PathBuf, native_name: OsString, kind: EntryKind, is_hidden: bool) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        display_name: native_name.to_string_lossy().into_owned(),
        thumbnail_path: None,
        native_name,
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden,
        recent_uri: None,
    }
}

/// Cached icon details: by fingerprint for revisited links and mounts, by path otherwise.
fn listed_icon_details(
    path: &Path,
    leads_elsewhere: bool,
    revisit: bool,
    target: Option<&FolderStamp>,
) -> Option<IconDetails> {
    match (leads_elsewhere, target) {
        (false, _) => cached_icon_details_for_revisit(path),
        (true, Some(target)) if revisit => {
            cached_icon_details(path, IconDetailsFingerprint::of(target))
        }
        (true, _) => None,
    }
}

fn native_entry(
    listed: ListedName,
    include_metadata: bool,
    progress: &Progress,
) -> (FileEntry, bool) {
    let ListedName {
        path,
        native_name,
        file_type,
        is_hidden,
        mounted,
    } = listed;
    let link = file_type == FileType::Symlink;
    let leads_elsewhere = link || mounted.is_some();
    let revisit = leads_elsewhere && revisits(&path);
    let wants_details = revisit || include_metadata;
    let follows = leads_elsewhere && (link || wants_details);
    let target = follows
        .then(|| {
            progress.reach(&path, mounted, || {
                crate::adapters::local_operations::folder_stamp(&path, false)
            })
        })
        .flatten()
        .and_then(Result::ok);
    let kind = match link {
        true => link_target_kind(target.as_ref().map(|target| target.kind)),
        false => listed_kind(file_type, &path),
    };
    let cached_details = listed_icon_details(&path, leads_elsewhere, revisit, target.as_ref());
    let mut entry = bare_entry(path, native_name, kind, is_hidden);
    if let Some(details) = cached_details {
        details.apply_to_entry(&mut entry);
    }
    if let Some(target) = target.as_ref().filter(|_| include_metadata) {
        fill_followed_metadata(&mut entry, target);
    }
    (entry, leads_elsewhere)
}

fn failed_listing(error: rustix::io::Errno) -> NativeEnumeration {
    let message = crate::services::io_error_message(&error.into());
    if matches!(error, rustix::io::Errno::ACCESS | rustix::io::Errno::PERM) {
        NativeEnumeration::Denied(message)
    } else {
        NativeEnumeration::Failed(message)
    }
}

/// The entries one listing read, with whether each leads to another folder or mount.
#[derive(Default)]
struct Listing {
    entries: Vec<FileEntry>,
    followed: Vec<bool>,
    truncated: bool,
    cancelled: bool,
    names_digest: u64,
}

/// One native folder listing in progress.
struct NativeScan<'a> {
    path: &'a Path,
    request: &'a DirectoryRequest,
    cancellable: &'a gio::Cancellable,
    deadline: Instant,
    progress: &'a Progress,
}

impl NativeScan<'_> {
    fn mounted_children(&self) -> HashMap<OsString, u64> {
        mount_watch::latest()
            .points()
            .filter(|(_, point)| point.parent() == Some(self.path))
            .filter_map(|(id, point)| Some((point.file_name()?.to_owned(), id)))
            .collect()
    }

    fn read_name(
        &self,
        listing: &mut Listing,
        (native_name, file_type): (OsString, FileType),
        names: (&HashSet<OsString>, &HashMap<OsString, u64>),
    ) -> ControlFlow<()> {
        if self.cancellable.is_cancelled() {
            listing.cancelled = true;
            return ControlFlow::Break(());
        }
        if Instant::now() >= self.deadline || listing.entries.len() == self.request.max_entries {
            listing.truncated = true;
            return ControlFlow::Break(());
        }
        let (hidden_names, mounted) = names;
        listing.names_digest = listing.names_digest.wrapping_add(name_hash(&native_name));
        let listed = ListedName {
            path: self.path.join(&native_name),
            is_hidden: is_hidden_name(&native_name, hidden_names),
            mounted: mounted.get(&native_name).copied(),
            native_name,
            file_type,
        };
        let (entry, leads_elsewhere) =
            native_entry(listed, self.request.include_metadata, self.progress);
        listing.entries.push(entry);
        listing.followed.push(leads_elsewhere);
        ControlFlow::Continue(())
    }

    fn list(&self, mounted: &HashMap<OsString, u64>) -> Result<Listing, NativeEnumeration> {
        let hidden_names = native_hidden_names(self.path);
        let mut listing = Listing {
            entries: Vec::with_capacity(1024),
            followed: Vec::with_capacity(1024),
            ..Listing::default()
        };
        self.progress.begin_reading();
        let listed = crate::adapters::local_operations::list_folder(self.path, &mut |chunk| {
            self.progress.beat();
            for name in chunk {
                self.read_name(&mut listing, name, (&hidden_names, mounted))?;
            }
            ControlFlow::Continue(())
        });
        if listing.cancelled {
            return Err(NativeEnumeration::Cancelled);
        }
        listed.map_err(failed_listing)?;
        Ok(listing)
    }

    // `access::can-trash`/`access::can-delete` describe the queried item, not its
    // children. Probe one actual entry so a directory that cannot itself be removed
    // (such as `$HOME`) does not incorrectly hide Trash/delete for the entries it
    // contains. Query the entry itself: a symlink is trashed as a link, and its
    // target may be slow or unreachable to stat.
    fn probe_capabilities(
        &self,
        entries: &[FileEntry],
        mounted: &HashMap<OsString, u64>,
    ) -> (Option<bool>, Option<bool>) {
        let probed_capabilities = entries
            .iter()
            .find(|entry| !mounted.contains_key(&entry.native_name))
            .and_then(|entry| {
                let probed = gio::File::for_path(entry.location.native_path()?).query_info(
                    "access::can-trash,access::can-delete",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    Some(self.cancellable),
                );
                self.progress.answered(&probed);
                probed.ok()
            });
        let can_trash = match probed_capabilities.as_ref().and_then(info_can_trash) {
            Some(true)
                if crate::adapters::trash_support::unsupported_reason_for_directory(self.path)
                    .is_some() =>
            {
                Some(false)
            }
            other => other,
        };
        let can_delete = probed_capabilities.as_ref().and_then(info_can_delete);
        (can_trash, can_delete)
    }

    fn out_of_time(&self) -> bool {
        self.cancellable.is_cancelled() || Instant::now() >= self.deadline
    }

    /// Stats the entries that lead nowhere else in parallel; `None` once cancelled.
    fn fill_entry_metadata(&self, entries: &mut [FileEntry], followed: &[bool]) -> Option<bool> {
        let width = metadata_fill_width().min(entries.len());
        let chunk = entries.len().div_ceil(width);
        std::thread::scope(|scope| {
            for (piece, followed) in entries.chunks_mut(chunk).zip(followed.chunks(chunk)) {
                scope.spawn(move || self.fill_piece(piece, followed));
            }
        });
        if self.cancellable.is_cancelled() {
            return None;
        }
        Some(Instant::now() < self.deadline)
    }

    fn fill_piece(&self, piece: &mut [FileEntry], followed: &[bool]) {
        let unfollowed = piece
            .iter_mut()
            .zip(followed)
            .filter(|(_, followed)| !**followed);
        for (entry, _) in unfollowed {
            if self.out_of_time() {
                break;
            }
            self.progress.answered(&fill_native_entry_metadata(entry));
        }
    }

    /// Whether the up-front metadata pass finished; `None` once cancelled.
    fn metadata_complete(&self, listing: &mut Listing) -> Option<bool> {
        let wants_metadata = self.request.include_metadata && !listing.entries.is_empty();
        if !wants_metadata {
            return Some(true);
        }
        if Instant::now() >= self.deadline {
            return Some(false);
        }
        self.fill_entry_metadata(&mut listing.entries, &listing.followed)
    }

    fn scan(&self, stamped: oneshot::Sender<DirectoryStamp>) -> NativeEnumeration {
        let stamp = match read_stamp(self.path, self.progress) {
            Ok(stamp) => stamp,
            Err(error) => return failed_listing(error),
        };
        let _ = stamped.send(stamp.clone());
        let mounted = self.mounted_children();
        let mut listing = match self.list(&mounted) {
            Ok(listing) => listing,
            Err(stopped) => return stopped,
        };
        let (can_trash, can_delete) = self.probe_capabilities(&listing.entries, &mounted);
        let Some(metadata_complete) = self.metadata_complete(&mut listing) else {
            return NativeEnumeration::Cancelled;
        };
        NativeEnumeration::Complete(NativeListing {
            stamp: Box::new(stamp),
            entries: listing.entries,
            truncated: listing.truncated,
            names_digest: listing.names_digest,
            metadata_complete,
            can_trash,
            can_delete,
        })
    }
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
    let scan = NativeScan {
        path,
        request,
        cancellable,
        deadline,
        progress,
    };
    match scan.scan(stamped) {
        NativeEnumeration::Cancelled if hold.as_ref().is_some_and(file_activity::Hold::stopped) => {
            NativeEnumeration::Failed(file_activity::HELD_FOR_OPERATION.to_owned())
        }
        scanned => scanned,
    }
}

fn native_lane_class(background: bool) -> Class {
    match background {
        true => Class::Background,
        false => Class::Interactive,
    }
}

/// Reports one native directory load as directory events.
struct NativeReport<'a> {
    request_id: RequestId,
    started: Instant,
    emit: &'a Rc<dyn Fn(DirectoryEvent)>,
}

impl NativeReport<'_> {
    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn failed(&self, message: String, refused: bool, denied: bool) {
        (self.emit)(DirectoryEvent::Failed {
            request_id: self.request_id,
            message,
            refused,
            denied,
        });
    }

    /// The scan's own result, or `None` once a lane refusal has been reported.
    fn lane_outcome(
        &self,
        outcome: Result<Arc<NativeEnumeration>, Unanswered>,
    ) -> Option<NativeEnumeration> {
        let request_id = self.request_id;
        match outcome {
            Ok(outcome) => {
                Some(Arc::try_unwrap(outcome).unwrap_or_else(|shared| (*shared).clone()))
            }
            Err(Unanswered::TimedOut | Unanswered::Skipped) => {
                tracing::warn!(
                    request_id = request_id.0,
                    "directory load refused: the mount stopped answering"
                );
                self.failed(
                    crate::services::io_error_message(&ErrorKind::TimedOut.into()),
                    true,
                    false,
                );
                None
            }
            Err(Unanswered::Failed) => {
                self.failed("Native directory worker failed".to_owned(), false, false);
                None
            }
        }
    }

    fn complete(&self, listing: NativeListing, stamp_sent: bool) {
        let request_id = self.request_id;
        let emit = self.emit;
        if !stamp_sent {
            emit(DirectoryEvent::Stamp {
                request_id,
                stamp: listing.stamp,
            });
        }
        let total_entries = listing.entries.len();
        if !listing.entries.is_empty() {
            tracing::info!(
                request_id = request_id.0,
                entries = total_entries,
                elapsed_ms = self.elapsed_ms(),
                "first directory batch ready"
            );
            emit(DirectoryEvent::Batch {
                request_id,
                entries: listing.entries,
            });
        }
        if !listing.metadata_complete {
            tracing::warn!(
                request_id = request_id.0,
                entries = total_entries,
                elapsed_ms = self.elapsed_ms(),
                reason = "metadata budget",
                "initial metadata pass incomplete"
            );
            emit(DirectoryEvent::MetadataIncomplete { request_id });
        }
        if listing.truncated {
            tracing::warn!(
                request_id = request_id.0,
                entries = total_entries,
                elapsed_ms = self.elapsed_ms(),
                reason = "budget",
                "directory load truncated"
            );
        } else {
            tracing::info!(
                request_id = request_id.0,
                entries = total_entries,
                elapsed_ms = self.elapsed_ms(),
                "directory load finished"
            );
            emit(DirectoryEvent::ListingDigest {
                request_id,
                names_digest: listing.names_digest,
            });
        }
        emit(DirectoryEvent::Finished {
            request_id,
            truncated: listing.truncated,
            can_trash: listing.can_trash,
            can_delete: listing.can_delete,
        });
    }

    fn report(&self, outcome: NativeEnumeration, stamp_sent: bool) {
        let request_id = self.request_id;
        match outcome {
            NativeEnumeration::Complete(listing) => self.complete(listing, stamp_sent),
            NativeEnumeration::Failed(message) => {
                tracing::warn!(request_id = request_id.0, "directory load failed");
                self.failed(message, false, false);
            }
            NativeEnumeration::Denied(message) => {
                tracing::info!(request_id = request_id.0, "directory load denied");
                self.failed(message, false, true);
            }
            NativeEnumeration::Cancelled => {}
        }
    }

    /// Runs the scan in its mount lane, forwarding the stamp as soon as it is read.
    async fn scan(
        &self,
        request: DirectoryRequest,
        path: PathBuf,
    ) -> Option<(NativeEnumeration, bool)> {
        let deadline = self.started + request.time_budget;
        let listing = format!(
            "listing with metadata {} up to {}, in the background {}",
            request.include_metadata, request.max_entries, request.background
        );
        let class = native_lane_class(request.background);
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
                    (self.emit)(DirectoryEvent::Stamp {
                        request_id: self.request_id,
                        stamp: Box::new(stamp),
                    });
                    stamp_sent = true;
                }
                work.await
            }
        };
        Some((self.lane_outcome(outcome)?, stamp_sent))
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
        let report = NativeReport {
            request_id,
            started,
            emit: &emit,
        };
        if let Some((outcome, stamp_sent)) = report.scan(request, path).await {
            report.report(outcome, stamp_sent);
        }
    });
    LoadHandle::new(move || {
        tracing::debug!(request_id = request_id.0, "directory load cancelled");
        task.abort();
    })
}

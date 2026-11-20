// SPDX-License-Identifier: MIT
//! Filling metadata and icon details for entries already listed.

use std::ops::ControlFlow;

use futures_channel::mpsc::UnboundedSender;

use super::*;

type MediaProbeResult = Result<crate::sandbox::metadata::MediaMetadata, String>;

pub(super) fn metadata_fill_width() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().min(8))
        .unwrap_or(4)
        .max(1)
}

/// The entries one metadata fill covers and how long it may take.
pub(super) struct FillRequest {
    pub(super) request_id: RequestId,
    pub(super) locations: Vec<Location>,
    pub(super) include_icon_details: bool,
    pub(super) time_budget: Duration,
}

/// What a fill attempted, failed and left unfinished.
#[derive(Clone, Copy, Default)]
pub(super) struct FillTally {
    pub(super) attempted: usize,
    pub(super) failed: usize,
    pub(super) truncated: bool,
}

impl FillTally {
    fn add(&mut self, other: Self) {
        self.attempted += other.attempted;
        self.failed += other.failed;
        self.truncated = self.truncated || other.truncated;
    }

    pub(super) fn outcome(self, total: usize) -> MetadataOutcome {
        let none_attempted = self.attempted == 0 && total > 0;
        let all_failed = self.attempted > 0 && self.failed == self.attempted;
        if self.truncated {
            MetadataOutcome::Truncated
        } else if none_attempted {
            MetadataOutcome::Unsupported
        } else if all_failed {
            MetadataOutcome::Failed
        } else {
            MetadataOutcome::Complete
        }
    }
}

pub(super) fn unknown_update(location: &Location) -> MetadataUpdate {
    MetadataUpdate {
        location: location.clone(),
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
    }
}

/// Sends a worker's first update at once, then batches of eight.
#[derive(Default)]
struct UpdateBatch {
    updates: Vec<MetadataUpdate>,
    sent_first: bool,
}

impl UpdateBatch {
    fn push(&mut self, update: MetadataUpdate, tx: &UnboundedSender<Vec<MetadataUpdate>>) {
        self.updates.push(update);
        if !self.sent_first || self.updates.len() >= 8 {
            let _ = tx.unbounded_send(std::mem::take(&mut self.updates));
            self.sent_first = true;
        }
    }

    fn flush(&mut self, tx: &UnboundedSender<Vec<MetadataUpdate>>) {
        if !self.updates.is_empty() {
            let _ = tx.unbounded_send(std::mem::take(&mut self.updates));
        }
    }
}

/// What every worker of one parallel fill shares.
struct FillJob<'a, F> {
    locations: &'a [Location],
    next: &'a AtomicUsize,
    include_icon_details: bool,
    deadline: Instant,
    fill_details: &'a F,
}

impl<F> FillJob<'_, F>
where
    F: Fn(&mut MetadataUpdate, &gio::FileInfo, &Location, &gio::Cancellable, Instant) -> bool,
{
    fn fill_pending_details(
        &self,
        pending: Vec<(MetadataUpdate, gio::FileInfo, &Location)>,
        cancellable: &gio::Cancellable,
        tx: &UnboundedSender<Vec<MetadataUpdate>>,
    ) -> bool {
        for (mut update, info, location) in pending {
            if !(self.fill_details)(&mut update, &info, location, cancellable, self.deadline) {
                return false;
            }
            let _ = tx.unbounded_send(vec![update]);
        }
        true
    }

    fn run_worker(
        &self,
        cancellable: &gio::Cancellable,
        tx: &UnboundedSender<Vec<MetadataUpdate>>,
    ) -> FillTally {
        let mut tally = FillTally::default();
        let mut batch = UpdateBatch::default();
        let mut pending_details = Vec::new();
        loop {
            let index = self.next.fetch_add(1, Ordering::Relaxed);
            let Some(location) = self.locations.get(index) else {
                break;
            };
            if cancellable.is_cancelled() || Instant::now() >= self.deadline {
                tally.truncated = true;
                break;
            }
            let Some(path) = location.native_path() else {
                continue;
            };
            tally.attempted += 1;
            let (update, ok) = match gio::File::for_path(path).query_info(
                METADATA_ATTRIBUTES,
                gio::FileQueryInfoFlags::NONE,
                Some(cancellable),
            ) {
                Ok(info) => {
                    let (update, ok) = update_from_info(&info, location);
                    if self.include_icon_details {
                        pending_details.push((update.clone(), info, location));
                    }
                    (update, ok)
                }
                Err(_) => (unknown_update(location), false),
            };
            tally.failed += usize::from(!ok);
            batch.push(update, tx);
        }
        batch.flush(tx);
        if !self.fill_pending_details(pending_details, cancellable, tx) {
            tally.truncated = true;
        }
        tally
    }

    fn run(
        &self,
        width: usize,
        cancellable: &gio::Cancellable,
        tx: &UnboundedSender<Vec<MetadataUpdate>>,
    ) -> FillTally
    where
        F: Sync,
    {
        let worker_count = width.max(1).min(self.locations.len());
        let mut tally = FillTally::default();
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..worker_count)
                .map(|_| {
                    let cancellable = cancellable.clone();
                    let tx = tx.clone();
                    scope.spawn(move || self.run_worker(&cancellable, &tx))
                })
                .collect();
            for handle in handles {
                match handle.join() {
                    Ok(piece) => tally.add(piece),
                    Err(_) => tally.truncated = true,
                }
            }
        });
        tally
    }
}

pub(super) fn emit_fill_outcome(
    emit: &Rc<dyn Fn(DirectoryEvent)>,
    request_id: RequestId,
    updates: Vec<MetadataUpdate>,
    outcome: MetadataOutcome,
) {
    if !updates.is_empty() {
        emit(DirectoryEvent::MetadataFilled {
            request_id,
            updates,
        });
    }
    emit(DirectoryEvent::MetadataFinished {
        request_id,
        outcome,
    });
}

fn run_parallel_fill(
    width: usize,
    request: FillRequest,
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
    let FillRequest {
        request_id,
        locations,
        include_icon_details,
        time_budget,
    } = request;
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let cancelled = cancellable.clone();
    let (tx, mut rx) = futures_channel::mpsc::unbounded::<Vec<MetadataUpdate>>();
    let task = glib::MainContext::default().spawn_local(async move {
        let deadline = Instant::now() + time_budget;
        let locations_len = locations.len();
        let blocking_task = gio::spawn_blocking(move || {
            let next = AtomicUsize::new(0);
            let job = FillJob {
                locations: &locations,
                next: &next,
                include_icon_details,
                deadline,
                fill_details: &fill_details,
            };
            let tally = job.run(width, &cancellable, &tx);
            drop(tx);
            tally
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

        let Ok(tally) = blocking_task.await else {
            return;
        };
        let outcome = if cancelled.is_cancelled() {
            MetadataOutcome::Cancelled
        } else {
            tally.outcome(locations_len)
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

pub(super) fn fill_parallel(request: FillRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
    run_parallel_fill(metadata_fill_width(), request, emit, fill_icon_details)
}

#[cfg(test)]
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

#[cfg(test)]
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
    let request = FillRequest {
        request_id,
        locations,
        include_icon_details,
        time_budget,
    };
    run_parallel_fill(width, request, emit, fill_details)
}

#[cfg(any(test, feature = "test-support"))]
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
    #[cfg(not(any(test, feature = "test-support")))]
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
    #[cfg(any(test, feature = "test-support"))]
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

fn mark_details_unavailable(update: &mut MetadataUpdate) {
    update.image_dimensions = MetadataValue::Unavailable;
    update.child_count = MetadataValue::Unavailable;
    update.duration_seconds = MetadataValue::Unavailable;
}

fn mark_media_unavailable(update: &mut MetadataUpdate) {
    update.image_dimensions = MetadataValue::Unavailable;
    update.duration_seconds = MetadataValue::Unavailable;
}

fn needs_media_metadata(info: &gio::FileInfo, path: &Path) -> bool {
    if info.file_type() != gio::FileType::Regular {
        return false;
    }
    let media = is_image_path(path) || is_media_path(path);
    media && bytes_support_named_family(path)
}

fn apply_media_metadata(
    update: &mut MetadataUpdate,
    metadata: crate::sandbox::metadata::MediaMetadata,
) {
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

/// Icon details for one native file that has none cached; `Break` ends the fill with
/// whether it finished in time.
struct DetailProbe<'a> {
    path: &'a Path,
    cancellable: &'a gio::Cancellable,
    deadline: Instant,
    fingerprint: Option<IconDetailsFingerprint>,
}

impl DetailProbe<'_> {
    fn expired(&self) -> bool {
        self.cancellable.is_cancelled() || Instant::now() >= self.deadline
    }

    fn cache(&self, update: &MetadataUpdate) -> bool {
        if self.expired() {
            return false;
        }
        cache_icon_details(self.path, self.fingerprint, update);
        true
    }

    fn child_count(&self, update: &mut MetadataUpdate, info: &gio::FileInfo) -> ControlFlow<bool> {
        if info.file_type() != gio::FileType::Directory {
            update.child_count = MetadataValue::Unavailable;
            return ControlFlow::Continue(());
        }
        match count_children(self.path, self.cancellable, self.deadline) {
            None => ControlFlow::Break(false),
            Some(Children::Counted(count)) => {
                update.child_count = MetadataValue::Known(count);
                ControlFlow::Continue(())
            }
            Some(Children::Withheld) => {
                mark_media_unavailable(update);
                update.child_count = MetadataValue::Unknown;
                ControlFlow::Break(true)
            }
            Some(Children::Unreadable) => {
                mark_details_unavailable(update);
                ControlFlow::Break(self.cache(update))
            }
        }
    }

    fn media(
        &self,
        update: &mut MetadataUpdate,
        probe: impl FnOnce(&Path, bool, &gio::Cancellable) -> MediaProbeResult,
    ) -> ControlFlow<bool> {
        #[cfg(any(test, feature = "test-support"))]
        if let Ok(mut counts) = media_metadata_probe_counts().lock() {
            *counts.entry(self.path.to_path_buf()).or_default() += 1;
        }
        let image = is_image_path(self.path);
        let result = probe(self.path, image, self.cancellable);
        if self.expired() {
            return ControlFlow::Break(false);
        }
        match result {
            Ok(metadata) => apply_media_metadata(update, metadata),
            Err(error) if error == file_activity::HELD_FOR_OPERATION => {
                return ControlFlow::Break(true);
            }
            Err(_) => mark_media_unavailable(update),
        }
        ControlFlow::Continue(())
    }

    fn fill(
        &self,
        update: &mut MetadataUpdate,
        info: &gio::FileInfo,
        probe: impl FnOnce(&Path, bool, &gio::Cancellable) -> MediaProbeResult,
    ) -> bool {
        if let ControlFlow::Break(finished) = self.child_count(update, info) {
            return finished;
        }
        if self.expired() {
            return false;
        }
        if !needs_media_metadata(info, self.path) {
            mark_media_unavailable(update);
        } else if let ControlFlow::Break(finished) = self.media(update, probe) {
            return finished;
        }
        self.cache(update)
    }
}

pub(super) fn fill_icon_details_with_probe(
    update: &mut MetadataUpdate,
    info: &gio::FileInfo,
    location: &Location,
    cancellable: &gio::Cancellable,
    deadline: Instant,
    probe: impl FnOnce(&Path, bool, &gio::Cancellable) -> MediaProbeResult,
) -> bool {
    let Some(path) = location.native_path() else {
        mark_details_unavailable(update);
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
    let detail_probe = DetailProbe {
        path,
        cancellable,
        deadline,
        fingerprint,
    };
    detail_probe.fill(update, info, probe)
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
            created_unix_seconds: info
                .creation_date_time()
                .map(|time| MetadataValue::Known(time.to_unix()))
                .unwrap_or(MetadataValue::Unavailable),
            mode,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
            listable,
        },
        ok,
    )
}

fn location_file(location: &Location) -> Option<gio::File> {
    location
        .native_path()
        .map(gio::File::for_path)
        .or_else(|| location.uri_value().map(gio::File::for_uri))
}

/// Queries one entry's metadata within `remaining`; `None` once that runs out.
async fn query_update(
    file: &gio::File,
    location: &Location,
    include_icon_details: bool,
    remaining: Duration,
) -> Option<(MetadataUpdate, bool)> {
    let queried = glib::future_with_timeout(
        remaining,
        file.query_info_future(
            METADATA_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        ),
    )
    .await;
    match queried {
        Ok(Ok(info)) => {
            let (mut update, ok) = update_from_info(&info, location);
            if include_icon_details {
                mark_details_unavailable(&mut update);
            }
            Some((update, ok))
        }
        Ok(Err(_)) => Some((unknown_update(location), false)),
        Err(_) => None,
    }
}

/// Fills entries one at a time through GIO, for locations that are not all native.
pub(super) fn fill_queried(request: FillRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
    let task = glib::MainContext::default().spawn_local(async move {
        let deadline = Instant::now() + request.time_budget;
        let locations = &request.locations;
        let mut updates = Vec::with_capacity(locations.len());
        let mut tally = FillTally::default();
        for location in locations {
            if Instant::now() >= deadline {
                tally.truncated = true;
                break;
            }
            let Some(file) = location_file(location) else {
                continue;
            };
            tally.attempted += 1;
            // Bound each stat by the remaining budget so one hung mount cannot stall the fill.
            let remaining = deadline.saturating_duration_since(Instant::now());
            let queried = match remaining.is_zero() {
                true => None,
                false => {
                    query_update(&file, location, request.include_icon_details, remaining).await
                }
            };
            let Some((update, ok)) = queried else {
                tally.truncated = true;
                break;
            };
            tally.failed += usize::from(!ok);
            updates.push(update);
        }
        let outcome = tally.outcome(locations.len());
        emit_fill_outcome(&emit, request.request_id, updates, outcome);
    });
    LoadHandle::new(move || task.abort())
}

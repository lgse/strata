// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

mod archive;
mod create_entry;

#[cfg(test)]
pub(crate) use archive::ArchiveListing;
pub(crate) use archive::{
    ARCHIVE_PREVIEW_FAILED_MESSAGE, ARCHIVE_TOO_LARGE_MESSAGE, ARCHIVE_UNSUPPORTED_MESSAGE,
    ArchiveListingStatus, INVALID_ARCHIVE, MAX_ARCHIVE_PASSWORD_BYTES, archive_payload_valid,
    decode_archive_listing, encode_archive_result, list_archive_entries_direct,
};

#[cfg(test)]
pub(crate) use archive::write_compression_fixture;

#[cfg(test)]
use std::cell::RefCell;
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    future::Future,
    io,
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{
        gio_file_for_location, location_for_file,
        trash_restore::{RestoreContext, plan_restore_for_location},
        volume::MountTable,
    },
    model::{FileEntry, Location},
    services::{
        CancelledOperation, CompressRequest, CreateDirectoryRequest, CreateFileRequest,
        DeleteRequest, ExtractRequest, LoadHandle, OperationEvent, OperationProvider,
        OperationRequestId, PasteRequest, RenameBatchRecord, RenameBatchRequest, RenameRequest,
        RestoreRequest, RestoreSource, TransferConflict, TrashedOriginal, UndoCopyRequest,
        UndoMergeRequest, UndoMoveRequest, UndoRenameRequest, validate_basename,
    },
};

#[derive(Clone, Debug)]
pub(crate) struct MountFlushHint {
    pub root: PathBuf,
    pub removable: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum SyncDestination<'a> {
    Local(&'a Path),
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "copy helper tests pass a destination that has no local path"
        )
    )]
    NonLocal,
}

fn blocking_join_message(error: Box<dyn std::any::Any + Send>) -> String {
    error
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            error
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "filesystem sync panicked".to_owned())
}

pub(crate) fn sync_filesystem(root: &Path) -> io::Result<()> {
    let handle = rustix::fs::open(
        root,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(io::Error::from)?;
    rustix::fs::syncfs(&handle).map_err(io::Error::from)?;
    Ok(())
}

pub(crate) fn flush_filesystem(root: &Path) -> io::Result<()> {
    #[cfg(test)]
    if let Some(kind) = sync_probe_before(root) {
        return Err(io::Error::new(kind, "flush failed"));
    }
    sync_filesystem(root)
}

#[cfg(test)]
mod sync_probe {
    use std::{
        io,
        path::{Path, PathBuf},
        sync::{Arc, Condvar, Mutex, atomic::AtomicBool, atomic::Ordering},
        thread,
    };

    struct SyncGate {
        started: AtomicBool,
        released: Mutex<bool>,
        released_cv: Condvar,
    }

    struct SyncProbeState {
        fail: Option<io::ErrorKind>,
        gate: Option<Arc<SyncGate>>,
        calls: Vec<SyncObservation>,
    }

    type SyncObserver = Arc<dyn Fn(&Path) + Send + Sync>;

    static SYNC_PROBE: Mutex<Option<SyncProbeState>> = Mutex::new(None);
    static SYNC_OBSERVER: Mutex<Option<SyncObserver>> = Mutex::new(None);

    #[derive(Clone, Debug)]
    pub(crate) struct SyncObservation {
        pub root: PathBuf,
        pub thread_id: thread::ThreadId,
    }

    pub(crate) fn install_sync_probe(fail: Option<io::ErrorKind>, gate: bool) {
        let gate = gate.then(|| {
            Arc::new(SyncGate {
                started: AtomicBool::new(false),
                released: Mutex::new(false),
                released_cv: Condvar::new(),
            })
        });
        let mut slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        *slot = Some(SyncProbeState {
            fail,
            gate,
            calls: Vec::new(),
        });
    }

    pub(crate) fn clear_sync_probe() {
        release_sync_probe();
        let mut slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        *slot = None;
    }

    pub(crate) fn set_sync_observer(observer: Option<SyncObserver>) {
        let mut slot = SYNC_OBSERVER
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *slot = observer;
    }

    pub(crate) fn release_sync_probe() {
        let gate = {
            let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
            slot.as_ref().and_then(|probe| probe.gate.clone())
        };
        let Some(gate) = gate else {
            return;
        };
        let mut released = gate
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *released = true;
        gate.released_cv.notify_all();
    }

    pub(crate) fn sync_probe_is_blocked() -> bool {
        let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        slot.as_ref()
            .and_then(|probe| probe.gate.as_ref())
            .is_some_and(|gate| gate.started.load(Ordering::SeqCst))
    }

    pub(crate) fn sync_probe_len() -> usize {
        let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        slot.as_ref().map(|probe| probe.calls.len()).unwrap_or(0)
    }

    pub(crate) fn sync_probe_observations() -> Vec<SyncObservation> {
        let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        slot.as_ref()
            .map(|probe| probe.calls.clone())
            .unwrap_or_default()
    }

    pub(super) fn sync_probe_before(root: &Path) -> Option<io::ErrorKind> {
        let (fail, gate) = {
            let mut slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
            let probe = slot.as_mut()?;
            probe.calls.push(SyncObservation {
                root: root.to_path_buf(),
                thread_id: thread::current().id(),
            });
            (probe.fail, probe.gate.clone())
        };
        let observer = SYNC_OBSERVER
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(observer) = observer {
            observer(root);
        }
        if let Some(gate) = gate {
            gate.started.store(true, Ordering::SeqCst);
            let mut released = gate
                .released
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while !*released {
                released = gate
                    .released_cv
                    .wait(released)
                    .unwrap_or_else(|error| error.into_inner());
            }
        }
        fail
    }
}

#[cfg(test)]
use sync_probe::sync_probe_before;
#[cfg(test)]
pub(crate) use sync_probe::{
    clear_sync_probe, install_sync_probe, release_sync_probe, set_sync_observer,
    sync_probe_is_blocked, sync_probe_len, sync_probe_observations,
};

#[cfg(test)]
thread_local! {
    static REMOVABLE_ROOTS_FOR_TEST: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
    static FORCE_CROSS_VOLUME_FOR_TEST: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn set_removable_roots_for_test(roots: Option<Vec<PathBuf>>) {
    REMOVABLE_ROOTS_FOR_TEST.with(|slot| *slot.borrow_mut() = roots);
}

#[cfg(test)]
pub(crate) fn set_force_cross_volume_for_test(force: bool) {
    FORCE_CROSS_VOLUME_FOR_TEST.with(|slot| slot.set(force));
}

#[cfg(test)]
fn force_cross_volume_for_test() -> bool {
    FORCE_CROSS_VOLUME_FOR_TEST.with(|slot| slot.get())
}

#[cfg(test)]
fn removable_hints_for_test() -> Option<Vec<MountFlushHint>> {
    REMOVABLE_ROOTS_FOR_TEST.with(|slot| {
        slot.borrow().as_ref().map(|roots| {
            roots
                .iter()
                .map(|root| MountFlushHint {
                    root: root.clone(),
                    removable: true,
                })
                .collect()
        })
    })
}

pub(crate) fn removable_sync_roots(
    destinations: &[SyncDestination<'_>],
    hints: &[MountFlushHint],
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for destination in destinations {
        let SyncDestination::Local(path) = destination else {
            continue;
        };
        let Some(hint) = hints
            .iter()
            .filter(|hint| path.starts_with(&hint.root))
            .max_by_key(|hint| hint.root.as_os_str().len())
        else {
            continue;
        };
        if hint.removable && !roots.iter().any(|root| root == &hint.root) {
            roots.push(hint.root.clone());
        }
    }
    roots
}

fn removable_roots_for_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        return Vec::new();
    }
    #[cfg(test)]
    if let Some(hints) = removable_hints_for_test() {
        let destinations: Vec<_> = paths
            .iter()
            .map(|path| SyncDestination::Local(path))
            .collect();
        return removable_sync_roots(&destinations, &hints);
    }
    let table = MountTable::current();
    let mounts = gio::VolumeMonitor::get().mounts();
    let mut hints = Vec::new();
    for path in paths {
        let Some(root) = table.mount_point_for(path).map(Path::to_path_buf) else {
            continue;
        };
        if hints.iter().any(|hint: &MountFlushHint| hint.root == root) {
            continue;
        }
        let removable = mounts.iter().any(|mount| {
            mount.root().path().as_deref() == Some(root.as_path())
                && mount_drive_is_removable(mount)
        });
        hints.push(MountFlushHint { root, removable });
    }
    let destinations: Vec<_> = paths
        .iter()
        .map(|path| SyncDestination::Local(path))
        .collect();
    removable_sync_roots(&destinations, &hints)
}

fn mount_drive_is_removable(mount: &gio::Mount) -> bool {
    let drive = mount
        .drive()
        .or_else(|| mount.volume().and_then(|volume| volume.drive()));
    drive.is_some_and(|drive| drive.is_removable() || drive.is_media_removable())
}

async fn flush_removable_writes(
    paths: Vec<PathBuf>,
    cancellable: &gio::Cancellable,
    emit: &Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
    completed: &[Location],
    affected_locations: HashSet<Location>,
) -> bool {
    let roots = removable_roots_for_paths(&paths);
    if roots.is_empty() {
        if cancellable.is_cancelled() {
            emit(cancelled_event(
                request_id,
                completed.to_vec(),
                Vec::new(),
                Vec::new(),
                affected_locations,
            ));
            return false;
        }
        return true;
    }
    emit(OperationEvent::FlushingToDevice { request_id });
    for root in roots {
        let synced = gio::spawn_blocking(move || flush_filesystem(&root)).await;
        let error = match synced {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => error.to_string(),
            Err(error) => blocking_join_message(error),
        };
        emit(OperationEvent::TransferFailed {
            request_id,
            completed_locations: completed.to_vec(),
            message: error,
        });
        return false;
    }
    if cancellable.is_cancelled() {
        emit(cancelled_event(
            request_id,
            completed.to_vec(),
            Vec::new(),
            Vec::new(),
            affected_locations,
        ));
        return false;
    }
    true
}

async fn flush_removable_destination(path: &Path) -> Result<(), glib::Error> {
    let roots = removable_roots_for_paths(std::slice::from_ref(&path.to_path_buf()));
    for root in roots {
        let synced = gio::spawn_blocking(move || flush_filesystem(&root)).await;
        match synced {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(io_error(error)),
            Err(error) => return Err(io_error(blocking_join_message(error))),
        }
    }
    Ok(())
}

async fn flush_written_roots(
    paths: &[PathBuf],
    emit: &Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
) {
    let roots = removable_roots_for_paths(paths);
    if roots.is_empty() {
        return;
    }
    emit(OperationEvent::FlushingToDevice { request_id });
    for root in roots {
        let _synced = gio::spawn_blocking(move || flush_filesystem(&root)).await;
    }
}

struct TransferStop {
    completed: Vec<Location>,
    failed: Vec<Location>,
    not_attempted: Vec<Location>,
    affected_locations: HashSet<Location>,
    failure: Option<String>,
}

async fn stop_transfer(
    paths: &[PathBuf],
    emit: &Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
    stop: TransferStop,
) {
    flush_written_roots(paths, emit, request_id).await;
    match stop.failure {
        Some(message) => emit(OperationEvent::TransferFailed {
            request_id,
            completed_locations: stop.completed,
            message,
        }),
        None => emit(cancelled_event(
            request_id,
            stop.completed,
            stop.failed,
            stop.not_attempted,
            stop.affected_locations,
        )),
    }
}

async fn await_cancellable<O, T>(
    object: &O,
    cancellable: &gio::Cancellable,
    start: impl FnOnce(&O, &gio::Cancellable, gio::GioFutureResult<Result<T, glib::Error>>) + 'static,
) -> Result<T, glib::Error>
where
    O: Clone + 'static,
    T: 'static,
{
    // The backend's callback is authoritative: cancellation can race with a successful result.
    let cancellable = cancellable.clone();
    gio::GioFuture::new(object, move |object, _, result| {
        start(object, &cancellable, result);
    })
    .await
}

struct TransferProgressTracker {
    request_id: OperationRequestId,
    completed_items: Cell<usize>,
    transferred_bytes: Cell<u64>,
    total_bytes: Option<u64>,
    emit: Rc<dyn Fn(OperationEvent)>,
}

impl TransferProgressTracker {
    fn new(
        request_id: OperationRequestId,
        total_bytes: Option<u64>,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> Rc<Self> {
        Rc::new(Self {
            request_id,
            completed_items: Cell::new(0),
            transferred_bytes: Cell::new(0),
            total_bytes,
            emit,
        })
    }

    fn emit(&self) {
        self.emit_progress(None);
    }

    fn emit_progress(&self, created_location: Option<Location>) {
        (self.emit)(OperationEvent::TransferProgress {
            request_id: self.request_id,
            completed_items: self.completed_items.get(),
            transferred_bytes: self.transferred_bytes.get(),
            total_bytes: self.total_bytes,
            created_location,
        });
    }

    fn add_bytes(&self, bytes: u64) {
        self.transferred_bytes
            .set(self.transferred_bytes.get().saturating_add(bytes));
        self.emit();
    }

    fn begin_file(self: &Rc<Self>) -> FileTransferProgress {
        FileTransferProgress {
            tracker: self.clone(),
            reported_bytes: Rc::new(Cell::new(0)),
            reported_total: Rc::new(Cell::new(0)),
        }
    }

    fn finish_item(
        &self,
        started_at: u64,
        expected_bytes: Option<u64>,
        created_location: Option<Location>,
    ) {
        if let Some(expected_bytes) = expected_bytes {
            let expected_end = started_at.saturating_add(expected_bytes);
            if self.transferred_bytes.get() < expected_end {
                self.transferred_bytes.set(expected_end);
            }
        }
        self.completed_items
            .set(self.completed_items.get().saturating_add(1));
        self.emit_progress(created_location);
    }
}

struct FileTransferProgress {
    tracker: Rc<TransferProgressTracker>,
    reported_bytes: Rc<Cell<u64>>,
    reported_total: Rc<Cell<u64>>,
}

impl FileTransferProgress {
    fn callback(&self) -> Box<dyn FnMut(i64, i64)> {
        let tracker = self.tracker.clone();
        let reported_bytes = self.reported_bytes.clone();
        let reported_total = self.reported_total.clone();
        Box::new(move |current, total| {
            let current = current.max(0) as u64;
            let previous = reported_bytes.get();
            if current > previous {
                reported_bytes.set(current);
            }
            if total >= 0 {
                reported_total.set(reported_total.get().max(total as u64));
            }
            tracker.add_bytes(current.saturating_sub(previous));
        })
    }

    fn finish(&self) {
        let final_bytes = self.reported_total.get().max(self.reported_bytes.get());
        let missing = final_bytes.saturating_sub(self.reported_bytes.get());
        if missing > 0 {
            self.tracker.add_bytes(missing);
            self.reported_bytes.set(final_bytes);
        }
    }
}

fn transfer_size(
    file: gio::File,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<Option<u64>, glib::Error>>>> {
    Box::pin(async move {
        let info = await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.query_info_async(
                "standard::type,standard::size",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        if info.file_type() != gio::FileType::Directory {
            return Ok(info
                .has_attribute(gio::FILE_ATTRIBUTE_STANDARD_SIZE)
                .then(|| info.size().max(0) as u64));
        }

        let enumerator = await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.enumerate_children_async(
                "standard::name",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        let mut total = Some(0_u64);
        loop {
            let children = await_cancellable(
                &enumerator,
                &cancellable,
                |enumerator, cancellable, result| {
                    enumerator.next_files_async(
                        64,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                },
            )
            .await?;
            if children.is_empty() {
                return Ok(total);
            }
            for child in children {
                let child_size =
                    transfer_size(file.child(child.name()), cancellable.clone()).await?;
                total = total.and_then(|total| child_size.and_then(|size| total.checked_add(size)));
            }
        }
    })
}

async fn transfer_sizes(
    files: &[gio::File],
    cancellable: &gio::Cancellable,
) -> Result<(Vec<Option<u64>>, Option<u64>), glib::Error> {
    let mut sizes = Vec::with_capacity(files.len());
    let mut total = Some(0_u64);
    for file in files {
        let size = match transfer_size(file.clone(), cancellable.clone()).await {
            Ok(size) => size,
            Err(error) if was_cancelled(&error) => return Err(error),
            Err(_) => None,
        };
        total = total.and_then(|total| size.and_then(|size| total.checked_add(size)));
        sizes.push(size);
    }
    Ok((sizes, total))
}

fn validated_child(parent: &gio::File, name: &str) -> Result<gio::File, &'static str> {
    validate_basename(name)?;
    Ok(parent.child(name))
}

fn transfer_is_noop(source: &gio::File, destination: &gio::File, target: &gio::File) -> bool {
    source.equal(target) || source.equal(destination) || destination.has_prefix(source)
}

fn parse_copy_suffix(stem: &OsStr) -> (&OsStr, Option<u64>) {
    let bytes = stem.as_bytes();
    if let Some(without_closing_parenthesis) = bytes.strip_suffix(b")")
        && let Some(separator) = without_closing_parenthesis
            .windows(b" (".len())
            .rposition(|window| window == b" (")
    {
        let suffix = &without_closing_parenthesis[separator + b" (".len()..];
        if !suffix.is_empty()
            && suffix[0] != b'0'
            && suffix.iter().all(u8::is_ascii_digit)
            && let Ok(suffix) = std::str::from_utf8(suffix)
            && let Ok(number) = suffix.parse::<u64>()
            && number < u64::MAX
        {
            return (OsStr::from_bytes(&bytes[..separator]), Some(number));
        }
    }
    (stem, None)
}

fn duplicate_candidate_name(
    base_stem: &OsStr,
    extension: Option<&OsStr>,
    copy_number: u64,
) -> OsString {
    let mut candidate = base_stem.as_bytes().to_vec();
    candidate.extend_from_slice(b" (");
    candidate.extend_from_slice(copy_number.to_string().as_bytes());
    candidate.push(b')');
    if let Some(extension) = extension {
        candidate.push(b'.');
        candidate.extend_from_slice(extension.as_bytes());
    }
    OsString::from_vec(candidate)
}

fn duplicate_target(
    destination: &gio::File,
    name: &Path,
    is_directory: bool,
    cancellable: &gio::Cancellable,
) -> Result<gio::File, glib::Error> {
    cancellable.set_error_if_cancelled()?;
    let name = name.as_os_str();
    let (stem, extension) = if is_directory {
        (name, None)
    } else {
        let path = Path::new(name);
        let extension = path.extension().filter(|extension| !extension.is_empty());
        (path.file_stem().unwrap_or(name), extension)
    };
    let (base_stem, copy_num) = parse_copy_suffix(stem);
    let start_index = copy_num.map_or(1, |number| number + 1);
    for index in start_index..=u64::MAX {
        cancellable.set_error_if_cancelled()?;
        let candidate_name = duplicate_candidate_name(base_stem, extension, index);
        let candidate = destination.child(&candidate_name);
        if !candidate.query_exists(Some(cancellable)) {
            return Ok(candidate);
        }
    }
    Err(io_error("Could not find an unused duplicate name"))
}

const FAT_INVALID_BYTES: &[u8] = b"\"*/:<>?\\|";

fn fat_sanitized_name(name: &OsStr) -> OsString {
    let mut bytes: Vec<u8> = name
        .as_bytes()
        .iter()
        .map(|&byte| {
            if FAT_INVALID_BYTES.contains(&byte) || byte < 0x20 {
                b'_'
            } else {
                byte
            }
        })
        .collect();
    // FAT drivers strip trailing dots and spaces.
    while matches!(bytes.last(), Some(b'.' | b' ')) {
        bytes.pop();
    }
    if bytes.is_empty() {
        bytes.push(b'_');
    }
    OsString::from_vec(bytes)
}

fn fat_name_key(name: &OsStr) -> OsString {
    match name.to_str() {
        Some(name) => OsString::from(name.to_uppercase()),
        None => OsString::from_vec(name.as_bytes().to_ascii_uppercase()),
    }
}

fn unique_fat_sibling_name(candidate: OsString, used: &mut HashSet<OsString>) -> OsString {
    if used.insert(fat_name_key(&candidate)) {
        return candidate;
    }
    let extension = Path::new(&candidate)
        .extension()
        .filter(|extension| !extension.is_empty())
        .map(OsStr::to_os_string);
    let stem = Path::new(&candidate)
        .file_stem()
        .map(OsStr::to_os_string)
        .unwrap_or(candidate);
    let (base_stem, copy_num) = parse_copy_suffix(&stem);
    let mut index = copy_num.map_or(1, |number| number + 1);
    loop {
        let attempt = duplicate_candidate_name(base_stem, extension.as_deref(), index);
        if used.insert(fat_name_key(&attempt)) {
            return attempt;
        }
        index = index.checked_add(1).unwrap_or(1);
    }
}

fn fat_family_child_name(name: &OsStr, fat_family: bool, used: &mut HashSet<OsString>) -> OsString {
    if fat_family {
        unique_fat_sibling_name(fat_sanitized_name(name), used)
    } else {
        name.to_os_string()
    }
}

#[derive(Clone, Copy)]
struct CopyOptions {
    overwrite_existing: bool,
    fat_family: bool,
}

fn target_is_fat_family(target: &gio::File, mounts: &MountTable) -> bool {
    target
        .path()
        .is_some_and(|path| matches!(mounts.fs_type_for(&path), Some("msdos" | "vfat" | "exfat")))
}

async fn copy_children(
    enumerator: &gio::FileEnumerator,
    cancellable: &gio::Cancellable,
    fat_family: bool,
) -> Result<Vec<gio::FileInfo>, glib::Error> {
    let mut children = Vec::new();
    loop {
        let batch = await_cancellable(
            enumerator,
            cancellable,
            |enumerator, cancellable, result| {
                enumerator.next_files_async(
                    64,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |output| result.resolve(output),
                );
            },
        )
        .await?;
        let finished = batch.is_empty();
        children.extend(batch);
        if !fat_family || finished {
            break;
        }
    }
    if fat_family {
        // Planning and copying must allocate collision suffixes in the same order.
        children.sort_by_key(|child| child.name());
    }
    Ok(children)
}

fn was_cancelled(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled)
}

/// Whether a delete failure is retryable as a permanent delete: it was a
/// trash attempt (never a permanent one, which has no further fallback),
/// and the destination doesn't support Trash at all rather than some other,
/// unrelated failure.
fn is_trash_unsupported_failure(permanent: bool, error: &glib::Error) -> bool {
    !permanent && error.matches(gio::IOErrorEnum::NotSupported)
}

fn cancelled_local_operation() -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Cancelled, "Operation cancelled")
}

async fn run_local_fs_step<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, glib::Error> {
    gio::spawn_blocking(work)
        .await
        .map_err(|_| io_error("Local filesystem task panicked"))?
        .map_err(io_error)
}

fn open_local_child_directory<Fd: AsFd>(parent: &Fd, name: &OsStr) -> Result<OwnedFd, String> {
    // RESOLVE_NO_SYMLINKS (stronger than O_NOFOLLOW) plus RESOLVE_BENEATH and
    // RESOLVE_NO_MAGICLINKS: if `name` changed to a symlink (or a magic
    // link) since it was last inspected, this fails closed instead of
    // opening whatever it now points to.
    retry_local_open(|| {
        rustix::fs::openat2(
            parent,
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH
                | rustix::fs::ResolveFlags::NO_SYMLINKS
                | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
    })
    .map_err(|error| {
        format!(
            "{} changed while it was being read: {error}",
            name.to_string_lossy()
        )
    })
}

fn local_directory_children<Fd: AsFd>(handle: &Fd) -> Result<Vec<OsString>, String> {
    let mut children = Vec::new();
    for entry in rustix::fs::Dir::read_from(handle).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let entry_name = entry.file_name();
        if entry_name == c"." || entry_name == c".." {
            continue;
        }
        children.push(OsString::from_vec(entry_name.to_bytes().to_vec()));
    }
    Ok(children)
}

/// The outcome of resolving one copy source entry relative to its parent
/// directory's file descriptor. The type is re-read from disk here rather
/// than trusted from any earlier listing, so a symlink swapped in for a
/// directory is copied as the symlink it now is instead of being opened as
/// a directory.
enum LocalCopySource {
    /// An open file description for a regular file. Kept alive until the
    /// GIO copy that reads through it has finished, so `/proc/self/fd/<n>`
    /// always resolves to this exact file no matter what happens to its
    /// name afterward.
    File(std::fs::File),
    /// A symlink and the path it points to, copied as a new symlink rather
    /// than by following it.
    Symlink(OsString),
    Directory {
        handle: OwnedFd,
        children: Vec<OsString>,
    },
}

fn open_local_copy_source<Fd: AsFd>(parent: &Fd, name: &OsStr) -> Result<LocalCopySource, String> {
    let stat = rustix::fs::statat(parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("Could not inspect {}: {error}", name.to_string_lossy()))?;
    match rustix::fs::FileType::from_raw_mode(stat.st_mode) {
        rustix::fs::FileType::Symlink => {
            let link = rustix::fs::readlinkat(parent, name, Vec::new()).map_err(|error| {
                format!("Could not read link {}: {error}", name.to_string_lossy())
            })?;
            Ok(LocalCopySource::Symlink(OsString::from_vec(
                link.into_bytes(),
            )))
        }
        rustix::fs::FileType::Directory => {
            let handle = open_local_child_directory(parent, name)?;
            let children = local_directory_children(&handle)?;
            Ok(LocalCopySource::Directory { handle, children })
        }
        rustix::fs::FileType::RegularFile => {
            // NONBLOCK so an entry swapped for a FIFO between the stat above
            // and this open cannot block waiting for a writer; the type is
            // re-checked on the opened descriptor below. GIO copies through
            // `/proc/self/fd`, so the flag never affects the copy itself.
            let file = rustix::fs::openat2(
                parent,
                name,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NONBLOCK,
                rustix::fs::Mode::empty(),
                rustix::fs::ResolveFlags::BENEATH
                    | rustix::fs::ResolveFlags::NO_SYMLINKS
                    | rustix::fs::ResolveFlags::NO_MAGICLINKS,
            )
            .map_err(|error| {
                format!(
                    "{} changed while it was being copied: {error}",
                    name.to_string_lossy()
                )
            })?;
            let opened = rustix::fs::fstat(&file).map_err(|error| {
                format!("Could not inspect {}: {error}", name.to_string_lossy())
            })?;
            if rustix::fs::FileType::from_raw_mode(opened.st_mode)
                != rustix::fs::FileType::RegularFile
            {
                return Err(format!(
                    "{} changed while it was being copied",
                    name.to_string_lossy()
                ));
            }
            Ok(LocalCopySource::File(std::fs::File::from(file)))
        }
        _ => Err(format!(
            "Cannot copy {}: it is not a regular file, directory, or symbolic link",
            name.to_string_lossy()
        )),
    }
}

struct CreatedCopyRoot {
    was_created: Cell<bool>,
    identity: Cell<Option<LocalFileIdentity>>,
}

impl CreatedCopyRoot {
    fn new() -> Self {
        Self {
            was_created: Cell::new(false),
            identity: Cell::new(None),
        }
    }
}

async fn record_created_copy_root(
    created_root: &Option<Rc<CreatedCopyRoot>>,
    target: &gio::File,
) -> Result<(), glib::Error> {
    if let Some(created_root) = created_root {
        created_root.was_created.set(true);
        created_root
            .identity
            .set(local_file_identity(target).await?);
    }
    Ok(())
}

type RemoteFileStageCopy = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

type RemoteFileStageCommit = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

async fn discard_incomplete_copy(stage: gio::File) -> Result<(), glib::Error> {
    match permanently_delete(stage, false, gio::Cancellable::new()).await {
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(()),
        result => result,
    }
}

/// Reserves an unpredictable sibling so a partial remote file is never exposed at the final path.
async fn create_remote_file_stage(
    target: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<gio::File, glib::Error> {
    let parent = target
        .parent()
        .ok_or_else(|| io_error("The destination has no parent directory"))?;
    let stage = parent.child(format!(".strata-copy-{}", glib::uuid_string_random()));
    let stream = match await_cancellable(&stage, cancellable, |stage, cancellable, result| {
        stage.create_async(
            gio::FileCreateFlags::PRIVATE,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
    {
        Ok(stream) => stream,
        Err(error) if was_cancelled(&error) => {
            let cleanup = discard_incomplete_copy(stage).await;
            return Err(copy_failure_after_cleanup(error, cleanup));
        }
        Err(error) => return Err(error),
    };
    if let Err(error) = await_cancellable(&stream, cancellable, |stream, cancellable, result| {
        stream.close_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
            result.resolve(output)
        });
    })
    .await
    {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    Ok(stage)
}

fn copy_failure_after_cleanup(
    copy_error: glib::Error,
    cleanup_result: Result<(), glib::Error>,
) -> glib::Error {
    match cleanup_result {
        Ok(()) => copy_error,
        Err(cleanup_error) => io_error(format!(
            "{copy_error}; the incomplete copy could not be removed: {cleanup_error}"
        )),
    }
}

async fn copy_new_remote_file_with(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    copy_to_stage: RemoteFileStageCopy,
    commit_stage: RemoteFileStageCommit,
) -> Result<(), glib::Error> {
    let stage = create_remote_file_stage(&target, &cancellable).await?;
    if let Err(error) = copy_to_stage(source, stage.clone(), cancellable.clone()).await {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    if let Err(error) = cancellable.set_error_if_cancelled() {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    if let Err(error) = commit_stage(stage.clone(), target, cancellable).await {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    Ok(())
}

/// Recursively copies the entry named `name` inside `parent` to `target`,
/// walking descriptor-relative to each already-open source directory
/// instead of re-resolving paths, so a component swapped out from under an
/// in-progress copy cannot redirect what gets read. Regular files are
/// hand-ed to GIO's own copy (preserving its metadata handling and any
/// reflink optimisation) through a `/proc/self/fd` reference pinned to the
/// exact file just verified, rather than the original, re-resolvable path.
fn copy_recursively_local(
    parent: OwnedFd,
    name: OsString,
    target: gio::File,
    options: CopyOptions,
    cancellable: gio::Cancellable,
    created_root: Option<Rc<CreatedCopyRoot>>,
    progress: Option<Rc<TransferProgressTracker>>,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if cancellable.is_cancelled() {
            return Err(cancelled_local_operation());
        }
        let step_parent = parent.try_clone().map_err(io_error)?;
        let step_name = name.clone();
        let step =
            run_local_fs_step(move || open_local_copy_source(&step_parent, &step_name)).await?;
        match step {
            LocalCopySource::Symlink(link_target) => {
                let target_path = target
                    .path()
                    .ok_or_else(|| io_error("Copy destination must be a local path"))?;
                run_local_fs_step(move || {
                    copy_local_symlink(&link_target, &target_path, options.overwrite_existing)
                })
                .await
            }
            LocalCopySource::File(file) => {
                // Deliberately no NOFOLLOW_SYMLINKS here: `/proc/self/fd/<n>`
                // is itself reported as a symlink by lstat, even though the
                // fd it names was already verified to be a plain file. GIO
                // must follow it to reach that file's actual content rather
                // than copying the magic-link's target text as a new symlink.
                let source_ref = gio::File::for_path(format!("/proc/self/fd/{}", file.as_raw_fd()));
                let flags = gio::FileCopyFlags::ALL_METADATA
                    | if options.overwrite_existing {
                        gio::FileCopyFlags::OVERWRITE
                    } else {
                        gio::FileCopyFlags::NONE
                    };
                let file_progress = progress.as_ref().map(TransferProgressTracker::begin_file);
                let progress_callback = file_progress.as_ref().map(FileTransferProgress::callback);
                let result = await_cancellable(
                    &source_ref,
                    &cancellable,
                    move |source, cancellable, result| {
                        source.copy_async(
                            &target,
                            flags,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            progress_callback,
                            move |output| result.resolve(output),
                        );
                    },
                )
                .await;
                if result.is_ok()
                    && let Some(file_progress) = file_progress
                {
                    file_progress.finish();
                }
                // Keeps `file` open (and its fd number stable) for the
                // duration of the copy above; only drop it once resolved.
                drop(file);
                result
            }
            LocalCopySource::Directory {
                handle,
                mut children,
            } => {
                if options.fat_family {
                    children.sort();
                }
                if !options.overwrite_existing || !target.query_exists(Some(&cancellable)) {
                    await_cancellable(&target, &cancellable, |target, cancellable, result| {
                        target.make_directory_async(
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    })
                    .await?;
                    record_created_copy_root(&created_root, &target).await?;
                }
                let mut used_names = HashSet::with_capacity(children.len());
                for child_name in children {
                    if cancellable.is_cancelled() {
                        return Err(cancelled_local_operation());
                    }
                    let child_parent = handle.try_clone().map_err(io_error)?;
                    let target_name =
                        fat_family_child_name(&child_name, options.fat_family, &mut used_names);
                    let child_target = target.child(&target_name);
                    copy_recursively_local(
                        child_parent,
                        child_name,
                        child_target,
                        options,
                        cancellable.clone(),
                        None,
                        progress.clone(),
                    )
                    .await?;
                }
                Ok(())
            }
        }
    })
}

/// Entry point for locally copying a source path: opens its parent
/// directory once, then hands off to the descriptor-relative walk in
/// [`copy_recursively_local`] for everything below it.
fn copy_recursively_local_path(
    source_path: PathBuf,
    target: gio::File,
    overwrite_existing: bool,
    cancellable: gio::Cancellable,
    created_root: Option<Rc<CreatedCopyRoot>>,
    progress: Option<Rc<TransferProgressTracker>>,
    fat_family: bool,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let Some(parent_path) = source_path.parent().map(Path::to_path_buf) else {
            return Err(io_error("Cannot copy the filesystem root"));
        };
        let Some(name) = source_path.file_name().map(OsStr::to_os_string) else {
            return Err(io_error("Invalid copy source"));
        };
        let parent = run_local_fs_step(move || open_local_parent_directory(&parent_path)).await?;
        copy_recursively_local(
            parent,
            name,
            target,
            CopyOptions {
                overwrite_existing,
                fat_family,
            },
            cancellable,
            created_root,
            progress,
        )
        .await
    })
}

fn copy_recursively_with_progress(
    source: gio::File,
    target: gio::File,
    overwrite_existing: bool,
    cancellable: gio::Cancellable,
    created_root: Option<Rc<CreatedCopyRoot>>,
    progress: Option<Rc<TransferProgressTracker>>,
    fat_family: bool,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    if source.is_native()
        && target.is_native()
        && let Some(source_path) = source.path()
    {
        // Remote (GVfs) locations have no local descriptor to walk against,
        // so anything not fully local keeps the GIO path-based copy below
        // rather than claiming an equivalent guarantee.
        return copy_recursively_local_path(
            source_path,
            target,
            overwrite_existing,
            cancellable,
            created_root,
            progress,
            fat_family,
        );
    }
    Box::pin(async move {
        let info = await_cancellable(&source, &cancellable, |source, cancellable, result| {
            source.query_info_async(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        if info.file_type() == gio::FileType::Directory {
            if !overwrite_existing || !target.query_exists(Some(&cancellable)) {
                await_cancellable(&target, &cancellable, |target, cancellable, result| {
                    target.make_directory_async(
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                })
                .await?;
                record_created_copy_root(&created_root, &target).await?;
            }
            let enumerator =
                await_cancellable(&source, &cancellable, |source, cancellable, result| {
                    source.enumerate_children_async(
                        "standard::name",
                        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                })
                .await?;
            let mut used_names = HashSet::new();
            loop {
                let children = copy_children(&enumerator, &cancellable, fat_family).await?;
                if children.is_empty() {
                    break;
                }
                for child in children {
                    let child_name = child.name();
                    let target_name =
                        fat_family_child_name(child_name.as_os_str(), fat_family, &mut used_names);
                    copy_recursively_with_progress(
                        source.child(&child_name),
                        target.child(&target_name),
                        overwrite_existing,
                        cancellable.clone(),
                        None,
                        progress.clone(),
                        fat_family,
                    )
                    .await?;
                }
                if fat_family {
                    break;
                }
            }
            Ok(())
        } else {
            let flags = gio::FileCopyFlags::ALL_METADATA
                | gio::FileCopyFlags::NOFOLLOW_SYMLINKS
                | if overwrite_existing {
                    gio::FileCopyFlags::OVERWRITE
                } else {
                    gio::FileCopyFlags::NONE
                };
            let file_progress = progress.as_ref().map(TransferProgressTracker::begin_file);
            let progress_callback = file_progress.as_ref().map(FileTransferProgress::callback);
            let result =
                await_cancellable(&source, &cancellable, move |source, cancellable, result| {
                    source.copy_async(
                        &target,
                        flags,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        progress_callback,
                        move |output| result.resolve(output),
                    );
                })
                .await;
            if result.is_ok()
                && let Some(file_progress) = file_progress
            {
                file_progress.finish();
            }
            result
        }
    })
}

#[cfg(test)]
fn copy_recursively(
    source: gio::File,
    target: gio::File,
    overwrite_existing: bool,
    cancellable: gio::Cancellable,
    created_root: Option<Rc<CreatedCopyRoot>>,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    copy_recursively_with_progress(
        source,
        target,
        overwrite_existing,
        cancellable,
        created_root,
        None,
        false,
    )
}

async fn copy_new_recursively_with_progress(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
) -> Result<(), glib::Error> {
    let fat_family = target_is_fat_family(&target, &MountTable::current());
    if !target.is_native() {
        let source_type =
            await_cancellable(&source, &cancellable, |source, cancellable, result| {
                source.query_info_async(
                    "standard::type",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |output| result.resolve(output),
                );
            })
            .await?
            .file_type();
        if source_type != gio::FileType::Directory {
            let copy_progress = progress.clone();
            return copy_new_remote_file_with(
                source,
                target,
                cancellable,
                Rc::new(move |source, stage, cancellable| {
                    let progress = copy_progress.clone();
                    Box::pin(async move {
                        copy_recursively_with_progress(
                            source,
                            stage,
                            true,
                            cancellable,
                            None,
                            progress,
                            fat_family,
                        )
                        .await
                    })
                }),
                Rc::new(|stage, target, cancellable| {
                    Box::pin(async move {
                        await_cancellable(
                            &stage,
                            &cancellable,
                            move |stage, cancellable, result| {
                                stage.move_async(
                                    &target,
                                    gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE,
                                    glib::Priority::DEFAULT,
                                    Some(cancellable),
                                    None,
                                    move |output| result.resolve(output),
                                );
                            },
                        )
                        .await
                    })
                }),
            )
            .await;
        }
    }

    if source.is_native()
        && target.is_native()
        && let Some(target_path) = target.path()
    {
        let source_type =
            await_cancellable(&source, &cancellable, |source, cancellable, result| {
                source.query_info_async(
                    "standard::type",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |output| result.resolve(output),
                );
            })
            .await?
            .file_type();
        if source_type == gio::FileType::Directory {
            let parent = target_path
                .parent()
                .ok_or_else(|| io_error("The destination has no parent directory"))?;
            let staged = StagedSibling::create(parent, true).map_err(io_error)?;
            if let Err(error) = copy_recursively_with_progress(
                source,
                gio::File::for_path(staged.path()),
                true,
                cancellable.clone(),
                None,
                progress.clone(),
                fat_family,
            )
            .await
            {
                discard_staged(staged).await;
                return Err(error);
            }
            if let Err(error) = cancellable.set_error_if_cancelled() {
                discard_staged(staged).await;
                return Err(error);
            }

            let staged_path = staged.path().to_owned();
            let committed = gio::spawn_blocking(move || {
                rustix::fs::renameat_with(
                    rustix::fs::CWD,
                    &staged_path,
                    rustix::fs::CWD,
                    &target_path,
                    rustix::fs::RenameFlags::NOREPLACE,
                )
            })
            .await
            .map_err(|_| io_error("The copy worker stopped unexpectedly"));
            let committed = match committed {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(io_error(format!(
                    "Could not finish copying the item: {error}"
                ))),
                Err(error) => Err(error),
            };
            if let Err(error) = committed {
                discard_staged(staged).await;
                return Err(error);
            }
            return Ok(());
        }
    }

    let created_root = Rc::new(CreatedCopyRoot::new());
    let result = copy_recursively_with_progress(
        source,
        target.clone(),
        false,
        cancellable.clone(),
        Some(created_root.clone()),
        progress,
        fat_family,
    )
    .await;
    if result.as_ref().is_err_and(was_cancelled) && created_root.was_created.get() {
        let cleanup_result = permanently_delete_maybe_local_if_unchanged(
            target,
            true,
            created_root.identity.get(),
            gio::Cancellable::new(),
        )
        .await;
        return result.map_err(|error| copy_failure_after_cleanup(error, cleanup_result));
    }
    result
}

#[cfg(test)]
async fn copy_new_recursively(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
) -> Result<(), glib::Error> {
    copy_new_recursively_with_progress(source, target, cancellable, None).await
}

type MoveAttempt = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

async fn move_local_with_progress(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
    attempt_move: MoveAttempt,
) -> Result<(), glib::Error> {
    let source_identity = local_file_identity(&source).await?;
    let target_path = target.path();
    let result = attempt_move(source.clone(), target.clone(), cancellable.clone()).await;
    match result {
        Err(error) if error.matches(gio::IOErrorEnum::WouldRecurse) => {
            copy_new_recursively_with_progress(
                source.clone(),
                target,
                cancellable.clone(),
                progress,
            )
            .await?;
            if let Some(path) = target_path.as_deref() {
                flush_removable_destination(path).await?;
            }
            permanently_delete_maybe_local_if_unchanged(source, true, source_identity, cancellable)
                .await
        }
        other => other,
    }
}

#[cfg(test)]
async fn move_local_with(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    attempt_move: MoveAttempt,
) -> Result<(), glib::Error> {
    move_local_with_progress(source, target, cancellable, None, attempt_move).await
}

/// Pins both resolved parents, then atomically renames without following either
/// entry or replacing a destination created by a concurrent process.
fn move_local_path(
    source_path: PathBuf,
    target_path: PathBuf,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if cancellable.is_cancelled() {
            return Err(cancelled_local_operation());
        }
        #[cfg(test)]
        if force_cross_volume_for_test() {
            return Err(glib::Error::new(
                gio::IOErrorEnum::WouldRecurse,
                "Cannot move directly",
            ));
        }
        let Some(source_parent_path) = source_path.parent().map(Path::to_path_buf) else {
            return Err(io_error("Cannot move the filesystem root"));
        };
        let Some(source_name) = source_path.file_name().map(OsStr::to_os_string) else {
            return Err(io_error("Invalid move source"));
        };
        let Some(target_parent_path) = target_path.parent().map(Path::to_path_buf) else {
            return Err(io_error("The move destination has no parent directory"));
        };
        let Some(target_name) = target_path.file_name().map(OsStr::to_os_string) else {
            return Err(io_error("Invalid move destination"));
        };

        let source_parent =
            run_local_fs_step(move || open_local_parent_directory(&source_parent_path)).await?;
        let target_parent =
            run_local_fs_step(move || open_local_parent_directory(&target_parent_path)).await?;

        let display_name = source_name.to_string_lossy().into_owned();
        gio::spawn_blocking(move || {
            rustix::fs::renameat_with(
                &source_parent,
                &source_name,
                &target_parent,
                &target_name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
        })
        .await
        .map_err(|_| io_error("Move task panicked"))?
        .map_err(|error| match error {
            rustix::io::Errno::XDEV | rustix::io::Errno::INVAL => {
                glib::Error::new(gio::IOErrorEnum::WouldRecurse, "Cannot move directly")
            }
            error => io_error(format!("Could not move {display_name}: {error}")),
        })
    })
}

async fn move_restore_path(
    source_path: PathBuf,
    target_path: PathBuf,
    allowed_root: PathBuf,
    cancellable: gio::Cancellable,
) -> Result<(), glib::Error> {
    move_restore_path_with(
        source_path,
        target_path,
        allowed_root,
        cancellable,
        |source_parent, source_name, target_parent, target_name, flags| {
            rustix::fs::renameat_with(
                source_parent,
                source_name,
                target_parent,
                target_name,
                flags,
            )
        },
    )
    .await
}

async fn move_restore_path_with(
    source_path: PathBuf,
    target_path: PathBuf,
    allowed_root: PathBuf,
    cancellable: gio::Cancellable,
    rename: impl FnOnce(
        &OwnedFd,
        &OsStr,
        &OwnedFd,
        &OsStr,
        rustix::fs::RenameFlags,
    ) -> rustix::io::Result<()>
    + Send
    + 'static,
) -> Result<(), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    let Some(source_parent_path) = source_path.parent().map(Path::to_path_buf) else {
        return Err(io_error("Cannot restore the filesystem root"));
    };
    let Some(source_name) = source_path.file_name().map(OsStr::to_os_string) else {
        return Err(io_error("Invalid restore source"));
    };
    let Some(target_parent_path) = target_path.parent().map(Path::to_path_buf) else {
        return Err(io_error("The restore destination has no parent directory"));
    };
    let Some(target_name) = target_path.file_name().map(OsStr::to_os_string) else {
        return Err(io_error("Invalid restore destination"));
    };

    let source_parent =
        run_local_fs_step(move || open_local_parent_directory(&source_parent_path)).await?;
    let target_parent =
        run_local_fs_step(move || open_local_parent_beneath(&target_parent_path, &allowed_root))
            .await?;

    let display_name = source_name.to_string_lossy().into_owned();
    gio::spawn_blocking(move || {
            if cancellable.is_cancelled() {
                return Err(rustix::io::Errno::CANCELED);
            }
            // Never fall back to an unflagged rename: the no-clobber check must be atomic.
            rename(
                &source_parent,
                &source_name,
                &target_parent,
                &target_name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
        })
        .await
        .map_err(|_| io_error("Restore task panicked"))?
        .map_err(|error| match error {
            rustix::io::Errno::XDEV => {
                io_error(format!("Could not restore {display_name} across volumes"))
            }
            rustix::io::Errno::EXIST => io_error(format!(
                "Could not restore {display_name}: something already exists at the destination"
            )),
            rustix::io::Errno::INVAL | rustix::io::Errno::NOSYS | rustix::io::Errno::OPNOTSUPP => io_error(format!(
                "Could not restore {display_name}: this filesystem does not support atomic no-replace renames. The item remains in Trash. Copy it to a destination you choose instead."
            )),
            rustix::io::Errno::CANCELED => cancelled_local_operation(),
            error => io_error(format!("Could not restore {display_name}: {error}")),
        })
}

async fn move_restore(
    source: gio::File,
    target: gio::File,
    allowed_root: PathBuf,
    cancellable: gio::Cancellable,
) -> Result<(), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    if source.is_native()
        && target.is_native()
        && let (Some(source_path), Some(target_path)) = (source.path(), target.path())
    {
        return move_restore_path(source_path, target_path, allowed_root, cancellable).await;
    }
    Err(io_error(
        "Trash restore requires a local source and destination",
    ))
}

async fn move_local(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
) -> Result<(), glib::Error> {
    let fallback_progress = progress.clone();
    move_local_with_progress(
        source,
        target,
        cancellable,
        fallback_progress,
        Rc::new(move |source, target, cancellable| {
            if source.is_native()
                && target.is_native()
                && let (Some(source_path), Some(target_path)) = (source.path(), target.path())
            {
                return move_local_path(source_path, target_path, cancellable);
            }
            // Remote (GVfs) locations have no local descriptor to walk against, so
            // anything not fully local keeps the GIO path-based move below rather
            // than claiming an equivalent guarantee.
            let move_progress = progress.as_ref().map(TransferProgressTracker::begin_file);
            let progress_callback = move_progress.as_ref().map(FileTransferProgress::callback);
            Box::pin(async move {
                let flags =
                    gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
                let result =
                    await_cancellable(&source, &cancellable, move |source, cancellable, result| {
                        source.move_async(
                            &target,
                            flags,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            progress_callback,
                            move |output| result.resolve(output),
                        );
                    })
                    .await;
                if result.is_ok()
                    && let Some(move_progress) = move_progress
                {
                    move_progress.finish();
                }
                result
            })
        }),
    )
    .await
}

enum StagedSibling {
    File(tempfile::TempPath),
    Directory(tempfile::TempDir),
}

impl StagedSibling {
    fn create(parent: &Path, directory: bool) -> io::Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(".strata-replacement-");
        if directory {
            builder.tempdir_in(parent).map(Self::Directory)
        } else {
            builder
                .tempfile_in(parent)
                .map(tempfile::NamedTempFile::into_temp_path)
                .map(Self::File)
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Directory(directory) => directory.path(),
        }
    }

    fn keep(self) -> io::Result<PathBuf> {
        match self {
            Self::File(path) => path.keep().map_err(|error| error.error),
            Self::Directory(directory) => Ok(directory.keep()),
        }
    }
}

async fn discard_staged(staged: StagedSibling) {
    let _discarded = gio::spawn_blocking(move || drop(staged)).await;
}

fn io_error(error: impl std::fmt::Display) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string())
}

type StageCopy = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        bool,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

async fn replace_local_with(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    copy_to_stage: StageCopy,
    on_replaced: &dyn Fn(),
) -> Result<(), glib::Error> {
    if let Some(locations) = affected_locations {
        locations.extend([&source, &target].into_iter().filter_map(location_for_file));
    }
    if source.path().is_none() {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "Safe replacement is unavailable for this source",
        ));
    }
    let target_path = target.path().ok_or_else(|| {
        glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "Safe replacement is unavailable at this destination",
        )
    })?;
    let parent = target_path
        .parent()
        .ok_or_else(|| io_error("The destination has no parent directory"))?;
    let source_type = await_cancellable(&source, &cancellable, |source, cancellable, result| {
        source.query_info_async(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?
    .file_type();
    let target_type = await_cancellable(&target, &cancellable, |target, cancellable, result| {
        target.query_info_async(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?
    .file_type();
    let source_is_directory = source_type == gio::FileType::Directory;
    let target_is_directory = target_type == gio::FileType::Directory;
    if source_is_directory != target_is_directory {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "A file and a folder cannot safely replace one another",
        ));
    }

    let source_identity = local_file_identity(&source).await?;
    let target_identity = local_file_identity(&target).await?;
    let staged = StagedSibling::create(parent, source_is_directory).map_err(io_error)?;
    let staged_file = gio::File::for_path(staged.path());
    if let Err(error) = copy_to_stage(
        source.clone(),
        staged_file.clone(),
        source_is_directory,
        cancellable.clone(),
    )
    .await
    {
        discard_staged(staged).await;
        return Err(error);
    }
    if let Err(error) = cancellable.set_error_if_cancelled() {
        discard_staged(staged).await;
        return Err(error);
    }
    if let Err(error) = ensure_local_file_identity(&target, target_identity).await {
        discard_staged(staged).await;
        return Err(error);
    }

    let staged_path = staged.path().to_owned();
    // Trash the original first so undo can restore it; keep the atomic
    // exchange with permanent delete where Trash is unsupported.
    let trashed = match await_cancellable(&target, &cancellable, |target, cancellable, result| {
        target.trash_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
            result.resolve(output)
        });
    })
    .await
    {
        Ok(()) => true,
        Err(error) if is_trash_unsupported_failure(false, &error) => false,
        Err(error) => {
            discard_staged(staged).await;
            return Err(error);
        }
    };
    if trashed {
        publish_staged_replacement(staged, target_path.clone()).await?;
        // A failed publication must not register an undo that would delete
        // a concurrent arrival. The original remains recoverable in Trash.
        on_replaced();
    } else {
        let exchange_target = target_path.clone();
        let exchanged = gio::spawn_blocking(move || {
            rustix::fs::renameat_with(
                rustix::fs::CWD,
                &staged_path,
                rustix::fs::CWD,
                &exchange_target,
                rustix::fs::RenameFlags::EXCHANGE,
            )
        })
        .await
        .map_err(|_| io_error("The replacement worker stopped unexpectedly"));
        let exchanged = match exchanged {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(io_error(format!(
                "Could not safely replace the item: {error}"
            ))),
            Err(error) => Err(error),
        };
        if let Err(error) = exchanged {
            discard_staged(staged).await;
            return Err(error);
        }

        let staged_file = gio::File::for_path(staged.keep().map_err(io_error)?);
        permanently_delete_maybe_local_if_unchanged(
            staged_file,
            target_is_directory,
            target_identity,
            gio::Cancellable::new(),
        )
        .await?;
    }
    if move_source {
        flush_removable_destination(&target_path).await?;
        permanently_delete_maybe_local_if_unchanged(
            source,
            source_is_directory,
            source_identity,
            cancellable,
        )
        .await?;
    }
    Ok(())
}

async fn publish_staged_replacement(
    staged: StagedSibling,
    target_path: PathBuf,
) -> Result<(), glib::Error> {
    let staged_path = staged.path().to_owned();
    let renamed = gio::spawn_blocking(move || {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            &staged_path,
            rustix::fs::CWD,
            &target_path,
            rustix::fs::RenameFlags::NOREPLACE,
        )
    })
    .await
    .map_err(|_| io_error("The replacement worker stopped unexpectedly"))
    .and_then(|result| result.map_err(io_error));
    match renamed {
        Ok(()) => {
            let _kept = staged.keep();
            Ok(())
        }
        Err(error) => {
            discard_staged(staged).await;
            Err(io_error(format!(
                "Could not place the replacement item; the original is in Trash: {error}"
            )))
        }
    }
}

async fn replace_local_with_progress(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    progress: Option<Rc<TransferProgressTracker>>,
    on_replaced: &dyn Fn(),
) -> Result<(), glib::Error> {
    let fat_family = target_is_fat_family(&target, &MountTable::current());
    replace_local_with(
        source,
        target,
        move_source,
        cancellable,
        affected_locations,
        Rc::new(move |source, staged, _directory, cancellable| {
            copy_recursively_with_progress(
                source,
                staged,
                true,
                cancellable,
                None,
                progress.clone(),
                fat_family,
            )
        }),
        on_replaced,
    )
    .await
}

#[cfg(test)]
async fn replace_local(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
) -> Result<(), glib::Error> {
    replace_local_with_progress(
        source,
        target,
        move_source,
        cancellable,
        affected_locations,
        None,
        &|| {},
    )
    .await
}

/// What a merge will write: `created` are the topmost destination paths with
/// no existing counterpart, `overwritten` are leaf collisions whose originals
/// get staged in Trash so undo can restore them.
#[derive(Default)]
struct MergePlan {
    created: Vec<Location>,
    overwritten: Vec<Location>,
}

/// Backs up an overwritten original before the incoming copy lands. Production
/// stages through Trash; tests substitute a rename so fixtures on filesystems
/// without Trash support still exercise the flow.
type StageOverwrite = Rc<
    dyn Fn(Location, gio::Cancellable) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

fn trash_stage_overwrite(
    location: Location,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let file = gio_file_for_location(&location);
        await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.trash_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
                result.resolve(output)
            });
        })
        .await
    })
}

/// The injectable steps of a merge, bundled to keep the call sites small.
struct MergeHooks<'a> {
    fat_family: bool,
    copy_into_target: StageCopy,
    stage_overwrite: StageOverwrite,
    on_merged: &'a dyn Fn(MergePlan),
}

fn classify_merge<'a>(
    source: &'a gio::File,
    target: &'a gio::File,
    plan: &'a mut MergePlan,
    cancellable: &'a gio::Cancellable,
    fat_family: bool,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>> + 'a>> {
    Box::pin(async move {
        let enumerator = await_cancellable(source, cancellable, |source, cancellable, result| {
            source.enumerate_children_async(
                "standard::name,standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        let mut used_names = HashSet::new();
        loop {
            let children = copy_children(&enumerator, cancellable, fat_family).await?;
            if children.is_empty() {
                return Ok(());
            }
            for child in children {
                if cancellable.is_cancelled() {
                    return Err(cancelled_local_operation());
                }
                let child_name = child.name();
                let target_name =
                    fat_family_child_name(child_name.as_os_str(), fat_family, &mut used_names);
                let child_source = source.child(&child_name);
                let child_target = target.child(&target_name);
                let target_type = match await_cancellable(
                    &child_target,
                    cancellable,
                    |target, cancellable, result| {
                        target.query_info_async(
                            "standard::type",
                            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    },
                )
                .await
                {
                    Ok(info) => Some(info.file_type()),
                    Err(error) if error.matches(gio::IOErrorEnum::NotFound) => None,
                    Err(error) => return Err(error),
                };
                let source_is_directory = child.file_type() == gio::FileType::Directory;
                match target_type {
                    None => {
                        if let Some(location) = location_for_file(&child_target) {
                            plan.created.push(location);
                        }
                    }
                    Some(gio::FileType::Directory) if source_is_directory => {
                        classify_merge(&child_source, &child_target, plan, cancellable, fat_family)
                            .await?;
                    }
                    Some(gio::FileType::Directory) => {
                        return Err(glib::Error::new(
                            gio::IOErrorEnum::NotSupported,
                            "A file and a folder cannot safely replace one another",
                        ));
                    }
                    Some(_) if source_is_directory => {
                        return Err(glib::Error::new(
                            gio::IOErrorEnum::NotSupported,
                            "A file and a folder cannot safely replace one another",
                        ));
                    }
                    Some(_) => {
                        if let Some(location) = location_for_file(&child_target) {
                            plan.overwritten.push(location);
                        }
                    }
                }
            }
            if fat_family {
                return Ok(());
            }
        }
    })
}

/// Combines the source folder into the existing destination folder: every
/// child lands inside `target`, overwriting same-named entries, while
/// destination-only contents stay. Unlike [`replace_local_with`] this writes
/// into the live destination, so a failed or cancelled merge leaves a
/// partial result behind and never cleans what was already there.
async fn merge_local_with(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    hooks: MergeHooks<'_>,
) -> Result<(), glib::Error> {
    if let Some(locations) = affected_locations {
        locations.extend([&source, &target].into_iter().filter_map(location_for_file));
    }
    for file in [&source, &target] {
        let file_type = await_cancellable(file, &cancellable, |file, cancellable, result| {
            file.query_info_async(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?
        .file_type();
        if file_type != gio::FileType::Directory {
            return Err(glib::Error::new(
                gio::IOErrorEnum::NotSupported,
                "Only folders can be merged",
            ));
        }
    }
    let mut plan = MergePlan::default();
    classify_merge(&source, &target, &mut plan, &cancellable, hooks.fat_family).await?;
    // Stage every overwritten original in Trash before the copy so undo can
    // restore it; on failure, report what was staged so it stays recoverable.
    let mut staged = Vec::new();
    for location in &plan.overwritten {
        let result = (hooks.stage_overwrite)(location.clone(), cancellable.clone()).await;
        if let Err(error) = result {
            (hooks.on_merged)(MergePlan {
                created: Vec::new(),
                overwritten: staged,
            });
            return Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                &format!(
                    "Could not move {} to Trash before merging: {error}",
                    location.display_name()
                ),
            ));
        }
        staged.push(location.clone());
    }
    (hooks.on_merged)(plan);
    let source_identity = local_file_identity(&source).await?;
    let target_path = target.path();
    (hooks.copy_into_target)(source.clone(), target, true, cancellable.clone()).await?;
    if move_source {
        if let Some(path) = target_path.as_deref() {
            flush_removable_destination(path).await?;
        }
        permanently_delete_maybe_local_if_unchanged(source, true, source_identity, cancellable)
            .await?;
    }
    Ok(())
}

async fn merge_local_with_progress(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    progress: Option<Rc<TransferProgressTracker>>,
    on_merged: &dyn Fn(MergePlan),
) -> Result<(), glib::Error> {
    let fat_family = target_is_fat_family(&target, &MountTable::current());
    merge_local_with(
        source,
        target,
        move_source,
        cancellable,
        affected_locations,
        MergeHooks {
            fat_family,
            copy_into_target: Rc::new(move |source, target, _directory, cancellable| {
                copy_recursively_with_progress(
                    source,
                    target,
                    true,
                    cancellable,
                    None,
                    progress.clone(),
                    fat_family,
                )
            }),
            stage_overwrite: Rc::new(trash_stage_overwrite),
            on_merged,
        },
    )
    .await
}

#[cfg(test)]
async fn merge_local(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    on_merged: &dyn Fn(MergePlan),
) -> Result<(), glib::Error> {
    merge_local_with_progress(
        source,
        target,
        move_source,
        cancellable,
        None,
        None,
        on_merged,
    )
    .await
}

fn permanently_delete(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if directory {
            let enumerator = await_cancellable(&file, &cancellable, |file, cancellable, result| {
                file.enumerate_children_async(
                    "standard::name,standard::type",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |output| result.resolve(output),
                );
            })
            .await?;
            loop {
                let children = await_cancellable(
                    &enumerator,
                    &cancellable,
                    |enumerator, cancellable, result| {
                        enumerator.next_files_async(
                            64,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    },
                )
                .await?;
                if children.is_empty() {
                    break;
                }
                for child in children {
                    permanently_delete(
                        file.child(child.name()),
                        child.file_type() == gio::FileType::Directory,
                        cancellable.clone(),
                    )
                    .await?;
                }
            }
        }
        await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.delete_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
                result.resolve(output)
            });
        })
        .await
    })
}

/// The outcome of resolving one delete target relative to its parent
/// directory's file descriptor.
enum LocalDeleteStep {
    /// A non-directory entry (file, symlink, or other special file) that has
    /// already been unlinked.
    Removed,
    /// A directory that was opened (not yet removed) along with its
    /// immediate children, still to be deleted before the directory itself.
    Directory {
        handle: OwnedFd,
        children: Vec<OsString>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LocalFileIdentity {
    device: u64,
    inode: u64,
}

impl LocalFileIdentity {
    fn from_stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        }
    }
}

fn ensure_expected_local_identity(
    name: &OsStr,
    stat: &rustix::fs::Stat,
    expected: Option<LocalFileIdentity>,
) -> Result<(), String> {
    if expected.is_some_and(|expected| expected != LocalFileIdentity::from_stat(stat)) {
        return Err(format!(
            "{} changed while the operation was in progress",
            name.to_string_lossy()
        ));
    }
    Ok(())
}

/// Fails closed if an opened directory is no longer at its original name.
fn ensure_local_delete_target_unchanged<ParentFd: AsFd, TargetFd: AsFd>(
    parent: &ParentFd,
    name: &OsStr,
    target: &TargetFd,
) -> Result<(), String> {
    let named = rustix::fs::statat(parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW).map_err(
        |error| {
            format!(
                "{} changed while it was being deleted: {error}",
                name.to_string_lossy()
            )
        },
    )?;
    let opened = rustix::fs::fstat(target)
        .map_err(|error| format!("Could not recheck {}: {error}", name.to_string_lossy()))?;
    if named.st_dev != opened.st_dev || named.st_ino != opened.st_ino {
        return Err(format!(
            "{} changed while it was being deleted",
            name.to_string_lossy()
        ));
    }
    Ok(())
}

/// Inspects and, for non-directories, immediately deletes the entry named
/// `name` inside `parent`. The type is re-read from disk here rather than
/// trusted from any earlier listing.
fn open_local_delete_target<Fd: AsFd>(
    parent: &Fd,
    name: &OsStr,
    expected: Option<LocalFileIdentity>,
) -> Result<LocalDeleteStep, String> {
    let stat = rustix::fs::statat(parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("Could not inspect {}: {error}", name.to_string_lossy()))?;
    ensure_expected_local_identity(name, &stat, expected)?;
    if !matches!(
        rustix::fs::FileType::from_raw_mode(stat.st_mode),
        rustix::fs::FileType::Directory
    ) {
        rustix::fs::unlinkat(parent, name, rustix::fs::AtFlags::empty())
            .map_err(|error| format!("Could not delete {}: {error}", name.to_string_lossy()))?;
        return Ok(LocalDeleteStep::Removed);
    }
    // RESOLVE_NO_SYMLINKS (stronger than O_NOFOLLOW) plus RESOLVE_BENEATH and
    // RESOLVE_NO_MAGICLINKS: if `name` changed to a symlink (or a magic link)
    // in the moment since the statat above, this fails closed instead of
    // opening whatever it now points to.
    let handle = retry_local_open(|| {
        rustix::fs::openat2(
            parent,
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH
                | rustix::fs::ResolveFlags::NO_SYMLINKS
                | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
    })
    .map_err(|error| {
        format!(
            "{} changed while it was being deleted: {error}",
            name.to_string_lossy()
        )
    })?;
    let opened = rustix::fs::fstat(&handle)
        .map_err(|error| format!("Could not recheck {}: {error}", name.to_string_lossy()))?;
    ensure_expected_local_identity(name, &opened, expected)?;
    let mut children = Vec::new();
    for entry in rustix::fs::Dir::read_from(&handle).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let entry_name = entry.file_name();
        if entry_name == c"." || entry_name == c".." {
            continue;
        }
        children.push(OsString::from_vec(entry_name.to_bytes().to_vec()));
    }
    Ok(LocalDeleteStep::Directory { handle, children })
}

fn retry_local_open(
    mut open: impl FnMut() -> rustix::io::Result<OwnedFd>,
) -> rustix::io::Result<OwnedFd> {
    for _ in 0..15 {
        match open() {
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => {}
            result => return result,
        }
    }
    open()
}

fn cancelled_local_delete() -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Cancelled, "Delete cancelled")
}

async fn run_local_delete_step<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, glib::Error> {
    gio::spawn_blocking(work)
        .await
        .map_err(|_| io_error("Delete task panicked"))?
        .map_err(io_error)
}

#[derive(Clone)]
struct LocalDeleteRoot {
    parent: Arc<OwnedFd>,
    name: OsString,
    expected: Option<LocalFileIdentity>,
}

struct LocalDeleteGuard {
    parent: Arc<OwnedFd>,
    name: OsString,
    handle: Arc<OwnedFd>,
}

enum LocalDeleteJob {
    Entry {
        parent: Arc<OwnedFd>,
        name: OsString,
        expected: Option<LocalFileIdentity>,
        guard: Option<Arc<LocalDeleteGuard>>,
        completion: Option<Arc<LocalDeleteGroup>>,
    },
    RemoveDirectory {
        parent: Arc<OwnedFd>,
        name: OsString,
        handle: Arc<OwnedFd>,
        completion: Option<Arc<LocalDeleteGroup>>,
    },
}

struct LocalDeleteGroup {
    remaining: AtomicUsize,
    final_job: Mutex<Option<LocalDeleteJob>>,
}

struct LocalDeleteQueueState {
    jobs: Vec<LocalDeleteJob>,
    active: usize,
}

struct LocalDeleteQueue {
    state: Mutex<LocalDeleteQueueState>,
    wake: Condvar,
    cancelled: Arc<AtomicBool>,
    failed: AtomicBool,
    error: Mutex<Option<String>>,
}

impl LocalDeleteQueue {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            state: Mutex::new(LocalDeleteQueueState {
                jobs: Vec::new(),
                active: 0,
            }),
            wake: Condvar::new(),
            cancelled,
            failed: AtomicBool::new(false),
            error: Mutex::new(None),
        }
    }

    fn is_stopped(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || self.failed.load(Ordering::Acquire)
    }

    fn enqueue(&self, job: LocalDeleteJob) {
        if self.is_stopped() {
            return;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if self.is_stopped() {
            return;
        }
        state.jobs.push(job);
        self.wake.notify_one();
    }

    fn fail(&self, error: String) {
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        let mut first_error = self
            .error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if first_error.is_none() {
            *first_error = Some(error);
        }
        self.failed.store(true, Ordering::Release);
        self.wake.notify_all();
    }

    fn next_job(&self) -> Option<LocalDeleteJob> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        loop {
            if self.is_stopped() {
                return None;
            }
            if let Some(job) = state.jobs.pop() {
                state.active += 1;
                return Some(job);
            }
            if state.active == 0 {
                return None;
            }
            state = self
                .wake
                .wait_timeout(state, Duration::from_millis(10))
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        }
    }

    fn result(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) {
            Err("Delete cancelled".to_owned())
        } else if let Some(error) = self
            .error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
        {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn finish_job(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.active = state.active.saturating_sub(1);
        if state.active == 0 && state.jobs.is_empty() {
            self.wake.notify_all();
        }
    }
}

fn complete_local_delete_job(
    queue: &Arc<LocalDeleteQueue>,
    completion: Option<Arc<LocalDeleteGroup>>,
) {
    let Some(group) = completion else {
        return;
    };
    if group.remaining.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    let final_job = group
        .final_job
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    if let Some(final_job) = final_job {
        queue.enqueue(final_job);
    }
}

fn process_local_delete_job(queue: &Arc<LocalDeleteQueue>, job: LocalDeleteJob) {
    match job {
        LocalDeleteJob::Entry {
            parent,
            name,
            expected,
            guard,
            completion,
        } => {
            if queue.is_stopped() {
                return;
            }
            if let Some(guard) = guard
                && let Err(error) = ensure_local_delete_target_unchanged(
                    guard.parent.as_ref(),
                    &guard.name,
                    guard.handle.as_ref(),
                )
            {
                queue.fail(error);
                return;
            }
            let step = match open_local_delete_target(parent.as_ref(), &name, expected) {
                Ok(step) => step,
                Err(error) => {
                    queue.fail(error);
                    return;
                }
            };
            match step {
                LocalDeleteStep::Removed => complete_local_delete_job(queue, completion),
                LocalDeleteStep::Directory { handle, children } => {
                    let handle = Arc::new(handle);
                    if children.is_empty() {
                        remove_local_delete_directory(queue, parent, name, handle, completion);
                        return;
                    }
                    let guard = Arc::new(LocalDeleteGuard {
                        parent: parent.clone(),
                        name: name.clone(),
                        handle: handle.clone(),
                    });
                    let group = Arc::new(LocalDeleteGroup {
                        remaining: AtomicUsize::new(children.len()),
                        final_job: Mutex::new(Some(LocalDeleteJob::RemoveDirectory {
                            parent: parent.clone(),
                            name,
                            handle: handle.clone(),
                            completion,
                        })),
                    });
                    for child in children {
                        queue.enqueue(LocalDeleteJob::Entry {
                            parent: handle.clone(),
                            name: child,
                            expected: None,
                            guard: Some(guard.clone()),
                            completion: Some(group.clone()),
                        });
                    }
                }
            }
        }
        LocalDeleteJob::RemoveDirectory {
            parent,
            name,
            handle,
            completion,
        } => remove_local_delete_directory(queue, parent, name, handle, completion),
    }
}

fn remove_local_delete_directory(
    queue: &Arc<LocalDeleteQueue>,
    parent: Arc<OwnedFd>,
    name: OsString,
    handle: Arc<OwnedFd>,
    completion: Option<Arc<LocalDeleteGroup>>,
) {
    if queue.is_stopped() {
        return;
    }
    if let Err(error) =
        ensure_local_delete_target_unchanged(parent.as_ref(), &name, handle.as_ref())
    {
        queue.fail(error);
        return;
    }
    if let Err(error) = rustix::fs::unlinkat(parent.as_ref(), &name, rustix::fs::AtFlags::REMOVEDIR)
    {
        queue.fail(format!(
            "Could not delete {}: {error}",
            name.to_string_lossy()
        ));
        return;
    }
    complete_local_delete_job(queue, completion);
}

fn local_device_is_rotational(fd: &OwnedFd) -> Option<bool> {
    let stat = rustix::fs::fstat(fd).ok()?;
    let device = PathBuf::from("/sys/dev/block").join(format!(
        "{}:{}",
        rustix::fs::major(stat.st_dev),
        rustix::fs::minor(stat.st_dev)
    ));
    let device = std::fs::canonicalize(device).ok()?;
    rotational_sysfs_node(&device, &mut HashSet::new())
}

fn rotational_sysfs_node(path: &Path, visited: &mut HashSet<PathBuf>) -> Option<bool> {
    let path = std::fs::canonicalize(path).ok()?;
    if !visited.insert(path.clone()) {
        return None;
    }

    let mut has_slave = false;
    let mut unknown_slave = false;
    if let Ok(slaves) = std::fs::read_dir(path.join("slaves")) {
        for slave in slaves.flatten() {
            has_slave = true;
            match rotational_sysfs_node(&slave.path(), visited) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => unknown_slave = true,
            }
        }
    }
    if has_slave {
        return (!unknown_slave).then_some(false);
    }

    let mut current = Some(path.as_path());
    while let Some(node) = current {
        if let Ok(value) = std::fs::read_to_string(node.join("queue/rotational")) {
            return match value.trim() {
                "0" => Some(false),
                "1" => Some(true),
                _ => None,
            };
        }
        current = node.parent();
    }
    None
}

fn bounded_local_delete_worker_count(available: usize, rotational: bool) -> usize {
    let available = available.max(1);
    if rotational { 1 } else { available.min(2) }
}

fn local_delete_worker_count(roots: &[LocalDeleteRoot]) -> usize {
    let available = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let rotational = roots
        .iter()
        .any(|root| local_device_is_rotational(root.parent.as_ref()) == Some(true));
    bounded_local_delete_worker_count(available, rotational)
}

fn parallel_delete_local_blocking_with_workers(
    roots: Vec<LocalDeleteRoot>,
    cancelled: Arc<AtomicBool>,
    worker_count: usize,
) -> Result<(), String> {
    let worker_count = worker_count.max(1);
    let queue = Arc::new(LocalDeleteQueue::new(cancelled.clone()));
    for root in roots {
        queue.enqueue(LocalDeleteJob::Entry {
            parent: root.parent,
            name: root.name,
            expected: root.expected,
            guard: None,
            completion: None,
        });
    }

    run_local_delete_workers(&queue, worker_count, |work| {
        thread::Builder::new().spawn(work)
    });

    queue.result()
}

fn run_local_delete_workers(
    queue: &Arc<LocalDeleteQueue>,
    worker_count: usize,
    mut spawn: impl FnMut(Box<dyn FnOnce() + Send>) -> std::io::Result<thread::JoinHandle<()>>,
) {
    let mut workers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let worker_queue = queue.clone();
        match spawn(Box::new(move || {
            let queue = worker_queue;
            let _priority = rustix::process::setpriority_process(None, 10);
            while let Some(job) = queue.next_job() {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    process_local_delete_job(&queue, job);
                }));
                if result.is_err() {
                    queue.fail("Delete worker panicked".to_owned());
                }
                queue.finish_job();
            }
        })) {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                queue.fail(format!("Could not start delete worker: {error}"));
                break;
            }
        }
    }
    for worker in workers {
        if worker.join().is_err() {
            queue.fail("Delete worker panicked".to_owned());
        }
    }
}

fn parallel_delete_local_blocking(
    roots: Vec<LocalDeleteRoot>,
    cancelled: Arc<AtomicBool>,
) -> Result<(), String> {
    let worker_count = local_delete_worker_count(&roots);
    parallel_delete_local_blocking_with_workers(roots, cancelled, worker_count)
}

fn parallel_delete_local(
    roots: Vec<LocalDeleteRoot>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    parallel_delete_local_with(roots, cancellable, parallel_delete_local_blocking)
}

fn parallel_delete_local_with(
    roots: Vec<LocalDeleteRoot>,
    cancellable: gio::Cancellable,
    delete: impl FnOnce(Vec<LocalDeleteRoot>, Arc<AtomicBool>) -> Result<(), String> + Send + 'static,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if roots.is_empty() {
            return Ok(());
        }
        if cancellable.is_cancelled() {
            return Err(cancelled_local_delete());
        }
        let cancelled = Arc::new(AtomicBool::new(cancellable.is_cancelled()));
        let cancellation_flag = cancelled.clone();
        let cancellation_handler = cancellable.connect_cancelled(move |_| {
            cancellation_flag.store(true, Ordering::Release);
        });
        let result = gio::spawn_blocking(move || delete(roots, cancelled)).await;
        if let Some(id) = cancellation_handler {
            cancellable.disconnect_cancelled(id);
        }
        match result.map_err(|_| io_error("Delete task panicked"))? {
            Ok(()) => Ok(()),
            Err(error) if error == "Delete cancelled" => Err(cancelled_local_delete()),
            Err(error) => Err(io_error(error)),
        }
    })
}

/// Recursively and permanently deletes the entry named `name` inside
/// `parent`, walking descriptor-relative to each already-open directory
/// rather than re-resolving paths, so a component swapped out from under an
/// in-progress delete cannot redirect it outside the tree it started in.
fn permanently_delete_local(
    parent: OwnedFd,
    name: OsString,
    expected: Option<LocalFileIdentity>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    parallel_delete_local(
        vec![LocalDeleteRoot {
            parent: Arc::new(parent),
            name,
            expected,
        }],
        cancellable,
    )
}

/// Uses a staged link so overwriting remains atomic without following the target.
fn copy_local_symlink(
    link_target: &OsStr,
    target_path: &Path,
    overwrite_existing: bool,
) -> Result<(), String> {
    let parent_path = target_path
        .parent()
        .ok_or_else(|| "The symlink destination has no parent directory".to_owned())?;
    let target_name = target_path
        .file_name()
        .ok_or_else(|| "Invalid symlink destination".to_owned())?;
    let parent = open_local_parent_directory(parent_path)?;

    if !overwrite_existing {
        return rustix::fs::symlinkat(link_target, &parent, target_name)
            .map_err(|error| format!("Could not recreate {}: {error}", target_path.display()));
    }

    let staged_name = format!(".strata-symlink-{}", glib::uuid_string_random());
    rustix::fs::symlinkat(link_target, &parent, &staged_name)
        .map_err(|error| format!("Could not stage {}: {error}", target_path.display()))?;
    let result = rustix::fs::renameat_with(
        &parent,
        &staged_name,
        &parent,
        target_name,
        rustix::fs::RenameFlags::empty(),
    );
    if result.is_err() {
        let _ = rustix::fs::unlinkat(&parent, &staged_name, rustix::fs::AtFlags::empty());
    }
    result.map_err(|error| format!("Could not recreate {}: {error}", target_path.display()))
}

/// Resolves ordinary parent aliases in one kernel lookup and pins the directory.
/// This is the operation's starting point, not a recursive traversal: selected
/// entries and their children retain their separate no-follow policy. Rooting at
/// `/` permits absolute symlink targets without allowing procfs magic links.
fn open_local_parent_directory(parent_path: &Path) -> Result<OwnedFd, String> {
    if !parent_path.is_absolute() {
        return Err("A local operation target must use an absolute path".to_owned());
    }
    let root = rustix::fs::open(
        c"/",
        rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("Could not open the filesystem root: {error}"))?;
    let relative = parent_path
        .strip_prefix(Path::new("/"))
        .map_err(|_| "A local operation target must use an absolute path".to_owned())?;
    if relative.as_os_str().is_empty() {
        return Ok(root);
    }
    retry_local_open(|| {
        rustix::fs::openat2(
            &root,
            relative,
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::IN_ROOT | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
    })
    .map_err(|error| format!("Could not safely open {}: {error}", parent_path.display()))
}

fn open_local_parent_beneath(parent_path: &Path, allowed_root: &Path) -> Result<OwnedFd, String> {
    if !parent_path.is_absolute() || !allowed_root.is_absolute() {
        return Err("A restore destination must use an absolute path".to_owned());
    }
    let root = rustix::fs::open(
        allowed_root,
        rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("Could not open the restore area: {error}"))?;
    if parent_path == allowed_root {
        return Ok(root);
    }
    let relative = parent_path
        .strip_prefix(allowed_root)
        .map_err(|_| "The restore destination is outside the trash volume".to_owned())?;
    retry_local_open(|| {
        rustix::fs::openat2(
            &root,
            relative,
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH
                | rustix::fs::ResolveFlags::NO_MAGICLINKS
                | rustix::fs::ResolveFlags::NO_XDEV,
        )
    })
    .map_err(|error| {
        format!(
            "Could not safely open restore destination {}: {error}",
            parent_path.display()
        )
    })
}

/// Entry point for permanently deleting a local path: opens the target's
/// parent directory once, then hands off to the descriptor-relative walk in
/// [`permanently_delete_local`] for everything below it.
fn permanently_delete_local_path_if_unchanged(
    path: PathBuf,
    expected: Option<LocalFileIdentity>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let Some(parent_path) = path.parent().map(Path::to_path_buf) else {
            return Err(io_error("Cannot permanently delete the filesystem root"));
        };
        let Some(name) = path.file_name().map(OsStr::to_os_string) else {
            return Err(io_error("Invalid delete target"));
        };
        let parent =
            run_local_delete_step(move || open_local_parent_directory(&parent_path)).await?;
        permanently_delete_local(parent, name, expected, cancellable).await
    })
}

async fn local_file_identity(file: &gio::File) -> Result<Option<LocalFileIdentity>, glib::Error> {
    if !file.is_native() {
        return Ok(None);
    }
    let Some(path) = file.path() else {
        return Ok(None);
    };
    run_local_delete_step(move || {
        let parent_path = path
            .parent()
            .ok_or_else(|| "Cannot inspect the filesystem root".to_owned())?;
        let name = path
            .file_name()
            .ok_or_else(|| "Invalid local filesystem target".to_owned())?;
        let parent = open_local_parent_directory(parent_path)?;
        let stat = rustix::fs::statat(&parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
        Ok(LocalFileIdentity::from_stat(&stat))
    })
    .await
    .map(Some)
}

async fn ensure_local_file_identity(
    file: &gio::File,
    expected: Option<LocalFileIdentity>,
) -> Result<(), glib::Error> {
    let current = local_file_identity(file).await?;
    if current != expected {
        return Err(io_error(
            "The target changed while the operation was in progress",
        ));
    }
    Ok(())
}

/// Deletes `file` through the descriptor-relative local walk when it names a
/// local path, or through the path-based GIO delete otherwise. Used for
/// every permanent delete this module performs on the caller's behalf --
/// not just the user-requested ones -- so that cleaning up a staged
/// replacement's old target, or a move's now-copied source, gets the same
/// race safety and doesn't act on whatever type that entry was earlier in
/// the operation rather than what it actually is right before deletion.
fn permanently_delete_maybe_local(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    permanently_delete_maybe_local_if_unchanged(file, directory, None, cancellable)
}

fn permanently_delete_maybe_local_if_unchanged(
    file: gio::File,
    directory: bool,
    expected: Option<LocalFileIdentity>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    if file.is_native()
        && let Some(path) = file.path()
    {
        return permanently_delete_local_path_if_unchanged(path, expected, cancellable);
    }
    permanently_delete(file, directory, cancellable)
}

fn operation_error_summary(errors: &[String], action: &str) -> String {
    let mut summary = format!(
        "{} could not be {action}. The remaining items were processed.",
        if errors.len() == 1 {
            "1 item".to_owned()
        } else {
            format!("{} items", errors.len())
        }
    );
    for error in errors.iter().take(8) {
        summary.push_str("\n\n• ");
        summary.push_str(error);
    }
    if errors.len() > 8 {
        summary.push_str(&format!("\n\n…and {} more", errors.len() - 8));
    }
    summary
}

fn deletion_error_summary(errors: &[String]) -> String {
    operation_error_summary(errors, "deleted")
}

/// Backends without Trash support (most remote filesystems, including SMB)
/// fail a move-to-trash with `NOT_SUPPORTED`. Give an actionable message for
/// that specific case instead of the raw GIO error text.
fn deletion_error_message(name: &str, permanent: bool, error: &glib::Error) -> String {
    if !permanent && error.matches(gio::IOErrorEnum::NotSupported) {
        format!("{name}: This location doesn't support Trash. Delete permanently instead.")
    } else {
        format!("{name}: {error}")
    }
}

impl TrashedOriginal {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    fn matches_path(self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|metadata| self == Self::from_metadata(&metadata))
    }
}

#[derive(Clone)]
struct RestoreEntry {
    source: Location,
    display_name: String,
    original_target: Option<Location>,
    trash_info: Option<PathBuf>,
    confirmed_destination: Option<PathBuf>,
    physical_path: Option<PathBuf>,
}

async fn trashed_entries_for_originals(
    original_locations: &[Location],
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, TrashedOriginal>,
) -> Result<Vec<RestoreEntry>, glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    let requested = original_locations
        .iter()
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect::<HashSet<_>>();
    // GVfs can miss an item re-trashed under the same basename after a restore, so prefer the
    // authoritative freedesktop.org metadata for the home trash before consulting trash:///.
    let fallback_requested = requested.clone();
    let fallback_cancellable = cancellable.clone();
    let fallback_originals = originals.clone();
    let mut fallback = gio::spawn_blocking(move || {
        home_trash_entries(
            &fallback_requested,
            &fallback_cancellable,
            &fallback_originals,
        )
    })
    .await
    .map_err(|_| glib::Error::new(gio::IOErrorEnum::Failed, "Trash lookup task failed"))?;
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    if fallback.len() == requested.len() && requested.len() == original_locations.len() {
        return Ok(original_locations
            .iter()
            .filter_map(|location| location.native_path())
            .filter_map(|path| fallback.remove(path))
            .collect());
    }

    let trash = gio::File::for_uri("trash:///");
    let enumerator = await_cancellable(&trash, cancellable, |trash, cancellable, result| {
        trash.enumerate_children_async(
            "standard::name,standard::display-name,trash::orig-path,trash::deletion-date",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |value| result.resolve(value),
        );
    })
    .await?;
    let mut newest = HashMap::<PathBuf, (String, Location, String)>::new();
    loop {
        if cancellable.is_cancelled() {
            return Err(cancelled_local_operation());
        }
        let infos = await_cancellable(
            &enumerator,
            cancellable,
            |enumerator, cancellable, result| {
                enumerator.next_files_async(
                    64,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |value| result.resolve(value),
                );
            },
        )
        .await?;
        if infos.is_empty() {
            break;
        }
        for info in infos {
            let Some(original_path) = info.attribute_byte_string("trash::orig-path") else {
                continue;
            };
            let original_path = PathBuf::from(original_path.as_str());
            if !requested.contains(&original_path) {
                continue;
            }
            let deletion_date = info
                .attribute_string("trash::deletion-date")
                .map(|value| value.to_string())
                .unwrap_or_default();
            let Some(location) = location_for_file(&trash.child(info.name())) else {
                continue;
            };
            let original = Location::local(&original_path);
            if let Some(expected) = originals.get(&original) {
                let plan = plan_restore_for_location(
                    &location,
                    Some(&original),
                    None,
                    None,
                    &RestoreContext::current(),
                )
                .await;
                if !plan.is_ok_and(|plan| expected.matches_path(&plan.source_path)) {
                    continue;
                }
            }
            let candidate = (deletion_date, location, info.display_name().to_string());
            match newest.get(&original_path) {
                Some(current) if current.0 >= candidate.0 => {}
                _ => {
                    newest.insert(original_path, candidate);
                }
            }
        }
    }

    if requested
        .iter()
        .any(|path| !fallback.contains_key(path) && !newest.contains_key(path))
        || requested.len() != original_locations.len()
    {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotFound,
            "One or more recently trashed items are no longer available",
        ));
    }
    Ok(original_locations
        .iter()
        .filter_map(|location| location.native_path())
        .filter_map(|path| {
            fallback.remove(path).or_else(|| {
                newest
                    .remove(path)
                    .map(|(_, source, display_name)| RestoreEntry {
                        source,
                        display_name,
                        original_target: None,
                        trash_info: None,
                        confirmed_destination: None,
                        physical_path: None,
                    })
            })
        })
        .collect())
}

/// Where a successful restore lands: the confirmed destination for a Trash
/// dialog restore, the trashinfo original path for an undo restore.
fn restored_destination(entry: &RestoreEntry) -> Location {
    entry
        .confirmed_destination
        .as_ref()
        .map(|path| Location::local(path.clone()))
        .or_else(|| entry.original_target.clone())
        .unwrap_or_else(|| entry.source.clone())
}

async fn restore_trash_entry(
    entry: &RestoreEntry,
    context: &RestoreContext,
    affected_locations: &mut HashSet<Location>,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let plan = match plan_restore_for_location(
        &entry.source,
        entry.original_target.as_ref(),
        entry.trash_info.as_deref(),
        entry.physical_path.as_deref(),
        context,
    )
    .await
    {
        Ok(plan)
            if entry
                .confirmed_destination
                .as_ref()
                .is_some_and(|confirmed| plan.destination != *confirmed) =>
        {
            return Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                "The original location changed and no longer matches the confirmed destination",
            ));
        }
        Ok(plan) => plan,
        Err(error) => {
            return Err(glib::Error::new(gio::IOErrorEnum::Failed, error.message()));
        }
    };
    if let Some(parent) = plan.destination.parent() {
        affected_locations.insert(Location::local(parent));
    }
    let source = gio::File::for_path(&plan.source_path);
    let target = gio::File::for_path(&plan.destination);
    move_restore(source, target, plan.allowed_root, cancellable.clone()).await?;
    if let Some(info_path) = plan.trash_info.as_ref().or(entry.trash_info.as_ref())
        && let Err(error) = std::fs::remove_file(info_path)
    {
        tracing::warn!(%error, "unable to remove restored trash metadata");
    }
    Ok(())
}

/// Finds the Trash entry a merge staged for an overwritten original.
fn trashed_merge_original(
    location: Location,
    cancellable: gio::Cancellable,
    originals: HashMap<Location, TrashedOriginal>,
) -> Pin<Box<dyn Future<Output = Result<RestoreEntry, glib::Error>>>> {
    Box::pin(async move {
        let entries = trashed_entries_for_originals(
            std::slice::from_ref(&location),
            &cancellable,
            &originals,
        )
        .await?;
        entries.into_iter().next().ok_or_else(|| {
            glib::Error::new(
                gio::IOErrorEnum::NotFound,
                "The original is no longer in Trash",
            )
        })
    })
}

/// Reverts one overwritten merge path: removes the incoming copy so the
/// staged original can move back out of Trash.
async fn undo_merged_overwrite(
    location: &Location,
    staged: &RestoreEntry,
    context: &RestoreContext,
    affected_locations: &mut HashSet<Location>,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let file = gio_file_for_location(location);
    let existing_type = await_cancellable(&file, cancellable, |file, cancellable, result| {
        file.query_info_async(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
    .map(|info| info.file_type())
    .ok();
    if let Some(file_type) = existing_type {
        permanently_delete_maybe_local(
            file,
            file_type == gio::FileType::Directory,
            cancellable.clone(),
        )
        .await?;
    }
    restore_trash_entry(staged, context, affected_locations, cancellable).await
}

/// Resolves where an overwritten original was staged. Production reads Trash;
/// tests hand back fixture entries pointing at a fake trash root.
type StagedOriginalLookup = Rc<
    dyn Fn(
        Location,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<RestoreEntry, glib::Error>>>>,
>;

async fn run_merge_undo(
    request_id: OperationRequestId,
    created: Vec<Location>,
    overwritten: Vec<Location>,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
    staged_original: StagedOriginalLookup,
) {
    let total = overwritten.len() + created.len();
    let pending: Vec<Location> = overwritten.iter().chain(&created).cloned().collect();
    let mut affected_locations = HashSet::from([Location::uri("trash:///")]);
    for location in &pending {
        if let Some(parent) = location.parent() {
            affected_locations.insert(parent);
        }
    }
    let context = RestoreContext::current();
    let mut completed_locations = Vec::new();
    let mut restored = Vec::new();
    let mut failed_locations = Vec::new();
    let mut errors = Vec::new();
    let mut completed = 0usize;
    let remaining = |index: usize| pending[index..].to_vec();
    // Undo applies to copies only, so an incoming item at an overwritten path
    // is a duplicate of the surviving source: delete it permanently to free
    // the path, then move the staged original back out of Trash.
    for (index, location) in overwritten.iter().enumerate() {
        if cancellable.is_cancelled() {
            emit(cancelled_event(
                request_id,
                completed_locations,
                failed_locations,
                remaining(index),
                affected_locations,
            ));
            return;
        }
        let result = match staged_original(location.clone(), cancellable.clone()).await {
            Ok(staged) => {
                undo_merged_overwrite(
                    location,
                    &staged,
                    &context,
                    &mut affected_locations,
                    &cancellable,
                )
                .await
            }
            Err(error) => Err(error),
        };
        let restored_location = match result {
            Ok(()) => {
                completed_locations.push(location.clone());
                restored.push(location.clone());
                Some(location.clone())
            }
            Err(error) if was_cancelled(&error) => {
                failed_locations.push(location.clone());
                emit(cancelled_event(
                    request_id,
                    completed_locations,
                    failed_locations,
                    remaining(index + 1),
                    affected_locations,
                ));
                return;
            }
            Err(error) => {
                errors.push(format!("{}: {error}", location.display_name()));
                failed_locations.push(location.clone());
                None
            }
        };
        completed += 1;
        emit(OperationEvent::RestoreProgress {
            request_id,
            completed,
            total,
            restored_location,
        });
    }
    for (index, location) in created.iter().enumerate() {
        if cancellable.is_cancelled() {
            emit(cancelled_event(
                request_id,
                completed_locations,
                failed_locations,
                remaining(overwritten.len() + index),
                affected_locations,
            ));
            return;
        }
        let file = gio_file_for_location(location);
        let existing_type = await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.query_info_async(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await
        .map(|info| info.file_type())
        .ok();
        let result = match existing_type {
            None => Ok(()),
            Some(_) => {
                let trashed =
                    await_cancellable(&file, &cancellable, |file, cancellable, result| {
                        file.trash_async(
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    })
                    .await;
                match trashed {
                    Err(error) if error.matches(gio::IOErrorEnum::NotSupported) => {
                        permanently_delete_maybe_local(
                            file,
                            existing_type == Some(gio::FileType::Directory),
                            cancellable.clone(),
                        )
                        .await
                    }
                    result => result.map(|_| ()),
                }
            }
        };
        let deleted_location = match result {
            Ok(()) => {
                completed_locations.push(location.clone());
                Some(location.clone())
            }
            Err(error) if was_cancelled(&error) => {
                failed_locations.push(location.clone());
                emit(cancelled_event(
                    request_id,
                    completed_locations,
                    failed_locations,
                    remaining(overwritten.len() + index + 1),
                    affected_locations,
                ));
                return;
            }
            Err(error) => {
                errors.push(format!("{}: {error}", location.display_name()));
                failed_locations.push(location.clone());
                None
            }
        };
        completed += 1;
        emit(OperationEvent::DeleteProgress {
            request_id,
            completed,
            total,
            deleted_location,
        });
    }
    if errors.is_empty() {
        emit(OperationEvent::Restored {
            request_id,
            locations: Vec::new(),
            restored,
        });
    } else {
        emit(OperationEvent::CompletedWithErrors {
            request_id,
            deleted_locations: completed_locations,
            retryable_locations: Vec::new(),
            has_non_retryable_failures: true,
            message: deletion_error_summary(&errors),
        });
    }
}

fn home_trash_entries(
    requested: &HashSet<PathBuf>,
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, TrashedOriginal>,
) -> HashMap<PathBuf, RestoreEntry> {
    home_trash_entries_at(
        &glib::user_data_dir().join("Trash"),
        requested,
        cancellable,
        originals,
    )
}

fn home_trash_entries_at(
    trash_root: &Path,
    requested: &HashSet<PathBuf>,
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, TrashedOriginal>,
) -> HashMap<PathBuf, RestoreEntry> {
    let info_root = trash_root.join("info");
    let files_root = trash_root.join("files");
    let mut newest = HashMap::<PathBuf, (String, RestoreEntry)>::new();
    let Ok(infos) = std::fs::read_dir(info_root) else {
        return HashMap::new();
    };
    for info in infos.flatten() {
        if cancellable.is_cancelled() {
            break;
        }
        let info_path = info.path();
        let Some(name) = info_path.file_name() else {
            continue;
        };
        let bytes = name.as_bytes();
        let Some(file_name) = bytes.strip_suffix(b".trashinfo") else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(&info_path) else {
            continue;
        };
        let encoded_path = contents.lines().find_map(|line| line.strip_prefix("Path="));
        let deletion_date = contents
            .lines()
            .find_map(|line| line.strip_prefix("DeletionDate="))
            .unwrap_or_default();
        let Some(original_path) =
            encoded_path.and_then(|path| gio::File::for_uri(&format!("file://{path}")).path())
        else {
            continue;
        };
        if !requested.contains(&original_path) {
            continue;
        }
        let source_path = files_root.join(OsString::from_vec(file_name.to_vec()));
        let Ok(metadata) = std::fs::symlink_metadata(&source_path) else {
            continue;
        };
        if originals
            .get(&Location::local(&original_path))
            .is_some_and(|expected| *expected != TrashedOriginal::from_metadata(&metadata))
        {
            continue;
        }
        let entry = RestoreEntry {
            source: Location::local(&source_path),
            display_name: source_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Trashed item".to_owned()),
            original_target: Some(Location::local(&original_path)),
            trash_info: Some(info_path),
            confirmed_destination: None,
            physical_path: Some(source_path.clone()),
        };
        match newest.get(&original_path) {
            Some((current_date, _)) if current_date.as_str() >= deletion_date => {}
            _ => {
                newest.insert(original_path, (deletion_date.to_owned(), entry));
            }
        }
    }
    newest
        .into_iter()
        .map(|(path, (_, entry))| (path, entry))
        .collect()
}

fn cancellation_handle(cancellable: gio::Cancellable) -> LoadHandle {
    LoadHandle::new(move || cancellable.cancel())
}

struct DeletionTarget {
    location: Location,
    display_name: String,
    is_directory: bool,
}

impl DeletionTarget {
    fn from_entry(entry: &FileEntry) -> Self {
        Self {
            location: entry.location.clone(),
            display_name: entry.display_name.clone(),
            is_directory: entry.is_directory(),
        }
    }

    async fn probed(location: &Location, cancellable: &gio::Cancellable) -> Self {
        let file = gio_file_for_location(location);
        let is_directory = await_cancellable(&file, cancellable, |file, cancellable, result| {
            file.query_info_async(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await
        .is_ok_and(|info| info.file_type() == gio::FileType::Directory);
        Self {
            location: location.clone(),
            display_name: location.display_name(),
            is_directory,
        }
    }
}

async fn run_deletion(
    request_id: OperationRequestId,
    targets: Vec<DeletionTarget>,
    permanent: bool,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
) {
    let mut errors = Vec::new();
    let mut deleted_locations = Vec::new();
    let mut failed_locations = Vec::new();
    let mut retryable_locations = Vec::new();
    let mut affected_locations = HashSet::new();
    for target in &targets {
        if let Some(parent) = target.location.parent() {
            affected_locations.insert(parent);
        }
        if target.is_directory {
            affected_locations.insert(target.location.clone());
        }
    }
    if !permanent {
        affected_locations.insert(Location::uri("trash:///"));
    }
    let total = targets.len();
    for (index, target) in targets.iter().enumerate() {
        if cancellable.is_cancelled() {
            emit(cancelled_event(
                request_id,
                deleted_locations,
                failed_locations,
                targets[index..]
                    .iter()
                    .map(|target| target.location.clone())
                    .collect(),
                affected_locations,
            ));
            return;
        }
        let file = gio_file_for_location(&target.location);
        let result = if permanent {
            if target
                .location
                .uri_value()
                .is_some_and(|uri| uri.starts_with("trash:"))
            {
                await_cancellable(&file, &cancellable, |file, cancellable, result| {
                    file.delete_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
                        result.resolve(output)
                    });
                })
                .await
            } else {
                permanently_delete_maybe_local(file, target.is_directory, cancellable.clone()).await
            }
        } else {
            await_cancellable(&file, &cancellable, |file, cancellable, result| {
                file.trash_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
                    result.resolve(output)
                });
            })
            .await
        };
        let deleted_location = if let Err(error) = result {
            if was_cancelled(&error) {
                failed_locations.push(target.location.clone());
                emit(cancelled_event(
                    request_id,
                    deleted_locations,
                    failed_locations,
                    targets[index + 1..]
                        .iter()
                        .map(|target| target.location.clone())
                        .collect(),
                    affected_locations,
                ));
                return;
            }
            if is_trash_unsupported_failure(permanent, &error) {
                retryable_locations.push(target.location.clone());
            }
            errors.push(deletion_error_message(
                &target.display_name,
                permanent,
                &error,
            ));
            failed_locations.push(target.location.clone());
            None
        } else {
            deleted_locations.push(target.location.clone());
            Some(target.location.clone())
        };
        emit(OperationEvent::DeleteProgress {
            request_id,
            completed: index + 1,
            total,
            deleted_location,
        });
    }
    if errors.is_empty() {
        emit(OperationEvent::Deleted {
            request_id,
            locations: deleted_locations,
        });
    } else {
        let has_non_retryable_failures = errors.len() > retryable_locations.len();
        emit(OperationEvent::CompletedWithErrors {
            request_id,
            deleted_locations,
            retryable_locations,
            has_non_retryable_failures,
            message: deletion_error_summary(&errors),
        });
    }
}

fn cancelled_event(
    request_id: crate::services::OperationRequestId,
    completed: Vec<Location>,
    failed: Vec<Location>,
    not_attempted: Vec<Location>,
    affected_locations: HashSet<Location>,
) -> OperationEvent {
    OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed,
            failed,
            not_attempted,
            affected_locations,
        },
    }
}

/// A batch rename item after target resolution and collision checks.
struct PlannedBatchRename {
    source: Location,
    new_name: String,
    original_name: String,
    target: Location,
    /// Temporary name the source was moved to so a batch-mate can take it.
    staged: Option<Location>,
    applied: bool,
    failed: bool,
}

async fn set_display_name(
    file: &gio::File,
    name: &str,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let name = name.to_owned();
    await_cancellable(file, cancellable, move |file, cancellable, result| {
        file.set_display_name_async(
            &name,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
    .map(|_| ())
}

/// Moves a staged source back to its original name after the batch was
/// interrupted. Uses its own cancellable: the operation's token is already
/// cancelled on this path. Returns false when the item stays at the
/// temporary name.
async fn restore_staged(planned: &mut PlannedBatchRename) -> bool {
    let Some(staged) = planned.staged.take() else {
        return true;
    };
    let file = gio_file_for_location(&staged);
    if set_display_name(&file, &planned.original_name, &gio::Cancellable::new())
        .await
        .is_ok()
    {
        return true;
    }
    planned.staged = Some(staged);
    false
}

async fn emit_batch_cancelled(
    request_id: OperationRequestId,
    pending: &mut [PlannedBatchRename],
    renamed: &[RenameBatchRecord],
    failed: &mut Vec<Location>,
    affected_locations: HashSet<Location>,
    emit: &Rc<dyn Fn(OperationEvent)>,
) {
    let mut not_attempted = Vec::new();
    for planned in pending.iter_mut() {
        if planned.applied || planned.failed {
            continue;
        }
        if restore_staged(planned).await {
            not_attempted.push(planned.source.clone());
        } else {
            planned.failed = true;
            failed.push(planned.source.clone());
        }
    }
    emit(cancelled_event(
        request_id,
        renamed
            .iter()
            .map(|record: &RenameBatchRecord| record.current.clone())
            .collect(),
        std::mem::take(failed),
        not_attempted,
        affected_locations,
    ));
}

/// Best-effort basename for error messages and rename records.
fn location_basename(location: &Location) -> String {
    if let Some(name) = location
        .native_path()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
    {
        return name;
    }
    gio_file_for_location(location)
        .basename()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "item".to_owned())
}

#[derive(Default)]
pub struct LocalOperationProvider;

impl OperationProvider for LocalOperationProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            if let Err(message) = validate_basename(&request.new_name) {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: message.to_owned(),
                });
                return;
            }
            let file = request
                .entry
                .location
                .native_path()
                .map(gio::File::for_path)
                .unwrap_or_else(|| {
                    gio::File::for_uri(request.entry.location.uri_value().unwrap_or_default())
                });
            let item = request.entry.location.clone();
            let affected_locations = item.parent().into_iter().collect();
            if operation_cancellable.is_cancelled() {
                emit(cancelled_event(
                    request.id,
                    Vec::new(),
                    Vec::new(),
                    vec![item],
                    affected_locations,
                ));
                return;
            }
            match await_cancellable(
                &file,
                &operation_cancellable,
                move |file, cancellable, result| {
                    file.set_display_name_async(
                        &request.new_name,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                },
            )
            .await
            {
                Ok(_) => emit(OperationEvent::Renamed {
                    request_id: request.id,
                }),
                Err(error) if was_cancelled(&error) => {
                    emit(cancelled_event(
                        request.id,
                        Vec::new(),
                        vec![item],
                        Vec::new(),
                        affected_locations,
                    ));
                }
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error.to_string(),
                }),
            }
        });
        cancellation_handle(cancellable)
    }

    fn rename_batch(
        &self,
        request: RenameBatchRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let mut renamed = Vec::new();
            let mut errors = Vec::new();
            let mut failed = Vec::new();
            let mut affected_locations = HashSet::new();

            // Resolve every target before touching the filesystem so
            // collisions inside the batch are decided up front.
            let mut pending: Vec<PlannedBatchRename> = Vec::new();
            let mut target_counts: HashMap<Location, usize> = HashMap::new();
            for item in &request.items {
                if let Some(parent) = item.location.parent() {
                    affected_locations.insert(parent);
                }
                let original_name = location_basename(&item.location);
                if let Err(message) = validate_basename(&item.new_name) {
                    errors.push(format!("{original_name}: {message}"));
                    failed.push(item.location.clone());
                    continue;
                }
                let Some(target) = item
                    .location
                    .parent()
                    .and_then(|parent| parent.child(std::ffi::OsStr::new(&item.new_name)))
                else {
                    errors.push(format!(
                        "{original_name}: could not resolve the new location"
                    ));
                    failed.push(item.location.clone());
                    continue;
                };
                if target == item.location {
                    continue;
                }
                *target_counts.entry(target.clone()).or_default() += 1;
                pending.push(PlannedBatchRename {
                    source: item.location.clone(),
                    new_name: item.new_name.clone(),
                    original_name,
                    target,
                    staged: None,
                    applied: false,
                    failed: false,
                });
            }
            // Two items claiming one target must not rename a winner and drop
            // the loser silently.
            pending.retain(|planned| {
                if target_counts.get(&planned.target).copied().unwrap_or(0) > 1 {
                    errors.push(format!(
                        "{}: \"{}\" is planned for more than one item",
                        planned.original_name, planned.new_name
                    ));
                    failed.push(planned.source.clone());
                    return false;
                }
                true
            });

            // A target still occupied by another item's source would fail on
            // the not-yet-renamed entry; stage those sources aside first so
            // the planned permutation can apply.
            let targets: HashSet<Location> = pending
                .iter()
                .map(|planned| planned.target.clone())
                .collect();
            let mut reserved: HashSet<Location> = targets.iter().cloned().collect();
            reserved.extend(pending.iter().map(|planned| planned.source.clone()));
            for index in 0..pending.len() {
                if operation_cancellable.is_cancelled() {
                    emit_batch_cancelled(
                        request.id,
                        &mut pending,
                        &renamed,
                        &mut failed,
                        affected_locations,
                        &emit,
                    )
                    .await;
                    return;
                }
                if !targets.contains(&pending[index].source) {
                    continue;
                }
                let Some(parent) = pending[index].source.parent() else {
                    continue;
                };
                let mut attempt = 0_u32;
                let staged = loop {
                    let name = format!(".strata-batch-{}-{attempt}", request.id.0);
                    if let Some(candidate) = parent.child(std::ffi::OsStr::new(&name))
                        && !reserved.contains(&candidate)
                        && !gio_file_for_location(&candidate)
                            .query_exists(None::<&gio::Cancellable>)
                    {
                        break candidate;
                    }
                    attempt += 1;
                };
                reserved.insert(staged.clone());
                let file = gio_file_for_location(&pending[index].source);
                let staged_name = location_basename(&staged);
                match set_display_name(&file, &staged_name, &operation_cancellable).await {
                    Ok(()) => pending[index].staged = Some(staged),
                    Err(error) if was_cancelled(&error) => {
                        emit_batch_cancelled(
                            request.id,
                            &mut pending,
                            &renamed,
                            &mut failed,
                            affected_locations,
                            &emit,
                        )
                        .await;
                        return;
                    }
                    // Source stays put; the blocked item reports its own error.
                    Err(_) => {}
                }
            }

            for index in 0..pending.len() {
                if operation_cancellable.is_cancelled() {
                    emit_batch_cancelled(
                        request.id,
                        &mut pending,
                        &renamed,
                        &mut failed,
                        affected_locations,
                        &emit,
                    )
                    .await;
                    return;
                }
                let (source, new_name) = {
                    let planned = &pending[index];
                    (
                        planned
                            .staged
                            .clone()
                            .unwrap_or_else(|| planned.source.clone()),
                        planned.new_name.clone(),
                    )
                };
                let file = gio_file_for_location(&source);
                match set_display_name(&file, &new_name, &operation_cancellable).await {
                    Ok(()) => {
                        let planned = &mut pending[index];
                        planned.applied = true;
                        renamed.push(RenameBatchRecord {
                            original: planned.source.clone(),
                            current: planned.target.clone(),
                            original_name: planned.original_name.clone(),
                        });
                    }
                    Err(error) if was_cancelled(&error) => {
                        emit_batch_cancelled(
                            request.id,
                            &mut pending,
                            &renamed,
                            &mut failed,
                            affected_locations,
                            &emit,
                        )
                        .await;
                        return;
                    }
                    Err(error) => {
                        let planned = &mut pending[index];
                        let stranded = planned.staged.as_ref().map(location_basename);
                        let restored = restore_staged(planned).await;
                        errors.push(match stranded.filter(|_| !restored) {
                            Some(staged_name) => format!(
                                "{}: {error} (item left as \"{staged_name}\")",
                                planned.original_name
                            ),
                            None => format!("{}: {error}", planned.original_name),
                        });
                        planned.failed = true;
                        failed.push(planned.source.clone());
                    }
                }
            }
            emit(OperationEvent::RenamedBatch {
                request_id: request.id,
                renamed,
                errors,
            });
        });
        cancellation_handle(cancellable)
    }

    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        create_entry::start(
            request.id,
            request.parent,
            request.name,
            request.unique_name,
            true,
            emit,
        )
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        create_entry::start(
            request.id,
            request.parent,
            request.name,
            request.unique_name,
            false,
            emit,
        )
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let destination = gio_file_for_location(&request.destination);
            let fat_family = target_is_fat_family(&destination, &MountTable::current());
            let mut used_names = HashSet::new();
            let mut affected_locations = HashSet::from([request.destination.clone()]);
            for parent in request.items.iter().filter_map(|item| item.source.parent()) {
                affected_locations.insert(parent);
            }
            let sources = request
                .items
                .iter()
                .map(|item| gio_file_for_location(&item.source))
                .collect::<Vec<_>>();
            let (item_sizes, total_bytes) =
                match transfer_sizes(&sources, &operation_cancellable).await {
                    Ok(sizes) => sizes,
                    Err(error) if was_cancelled(&error) => {
                        emit(cancelled_event(
                            request.id,
                            Vec::new(),
                            Vec::new(),
                            request
                                .items
                                .iter()
                                .map(|item| item.source.clone())
                                .collect(),
                            affected_locations,
                        ));
                        return;
                    }
                    Err(error) => {
                        emit(OperationEvent::TransferFailed {
                            request_id: request.id,
                            completed_locations: Vec::new(),
                            message: error.to_string(),
                        });
                        return;
                    }
                };
            let progress = TransferProgressTracker::new(request.id, total_bytes, emit.clone());
            progress.emit();
            let mut completed = Vec::new();
            let mut written_paths = Vec::new();
            for (index, item) in request.items.iter().enumerate() {
                if operation_cancellable.is_cancelled() {
                    stop_transfer(
                        &written_paths,
                        &emit,
                        request.id,
                        TransferStop {
                            completed,
                            failed: Vec::new(),
                            not_attempted: request.items[index..]
                                .iter()
                                .map(|item| item.source.clone())
                                .collect(),
                            affected_locations,
                            failure: None,
                        },
                    )
                    .await;
                    return;
                }
                let source = sources[index].clone();
                let item_started_at = progress.transferred_bytes.get();
                let Some(name) = source.basename() else {
                    flush_written_roots(&written_paths, &emit, request.id).await;
                    emit(OperationEvent::Failed {
                        request_id: request.id,
                        message: "A clipboard item has no file name".to_owned(),
                    });
                    return;
                };
                let name = PathBuf::from(fat_family_child_name(
                    name.as_os_str(),
                    fat_family,
                    &mut used_names,
                ));
                let default_target = destination.child(&name);
                let is_duplicate = !request.move_sources && source.equal(&default_target);
                let needs_unique_target =
                    is_duplicate || item.conflict == TransferConflict::KeepBoth;
                if (!is_duplicate && transfer_is_noop(&source, &destination, &default_target))
                    || (is_duplicate && item.conflict == TransferConflict::ReplaceExisting)
                {
                    completed.push(item.source.clone());
                    progress.finish_item(item_started_at, item_sizes[index], None);
                    continue;
                }
                let target = if needs_unique_target {
                    let is_directory = match await_cancellable(
                        &source,
                        &operation_cancellable,
                        |source, cancellable, result| {
                            source.query_info_async(
                                "standard::type",
                                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                                glib::Priority::DEFAULT,
                                Some(cancellable),
                                move |output| result.resolve(output),
                            );
                        },
                    )
                    .await
                    {
                        Ok(info) => info.file_type() == gio::FileType::Directory,
                        Err(error) => {
                            let failure = (!was_cancelled(&error)).then(|| error.to_string());
                            stop_transfer(
                                &written_paths,
                                &emit,
                                request.id,
                                TransferStop {
                                    completed,
                                    failed: vec![item.source.clone()],
                                    not_attempted: request.items[index + 1..]
                                        .iter()
                                        .map(|item| item.source.clone())
                                        .collect(),
                                    affected_locations,
                                    failure,
                                },
                            )
                            .await;
                            return;
                        }
                    };
                    match duplicate_target(
                        &destination,
                        &name,
                        is_directory,
                        &operation_cancellable,
                    ) {
                        Ok(target) => target,
                        Err(error) => {
                            let failure = (!was_cancelled(&error)).then(|| error.to_string());
                            stop_transfer(
                                &written_paths,
                                &emit,
                                request.id,
                                TransferStop {
                                    completed,
                                    failed: vec![item.source.clone()],
                                    not_attempted: request.items[index + 1..]
                                        .iter()
                                        .map(|item| item.source.clone())
                                        .collect(),
                                    affected_locations,
                                    failure,
                                },
                            )
                            .await;
                            return;
                        }
                    }
                } else {
                    default_target
                };
                affected_locations.insert(item.source.clone());
                let target_location = location_for_file(&target);
                if let Some(target) = target_location.clone() {
                    affected_locations.insert(target);
                }
                if let Some(path) = target.path() {
                    written_paths.push(path);
                }
                let result = if is_duplicate {
                    copy_new_recursively_with_progress(
                        source,
                        target,
                        operation_cancellable.clone(),
                        Some(progress.clone()),
                    )
                    .await
                } else if item.conflict == TransferConflict::Merge {
                    merge_local_with_progress(
                        source,
                        target,
                        request.move_sources,
                        operation_cancellable.clone(),
                        Some(&mut affected_locations),
                        Some(progress.clone()),
                        &|plan| {
                            emit(OperationEvent::Merged {
                                request_id: request.id,
                                source: item.source.clone(),
                                created: plan.created,
                                overwritten: plan.overwritten,
                            });
                        },
                    )
                    .await
                } else if item.conflict == TransferConflict::ReplaceExisting {
                    let replaced_target = target_location.clone();
                    let emit = emit.clone();
                    replace_local_with_progress(
                        source,
                        target,
                        request.move_sources,
                        operation_cancellable.clone(),
                        Some(&mut affected_locations),
                        Some(progress.clone()),
                        &move || {
                            // Only copies get the restore-the-original undo
                            // entry; a replaced move keeps the move-back
                            // record and leaves the original in Trash.
                            if request.move_sources {
                                return;
                            }
                            if let Some(target) = &replaced_target {
                                emit(OperationEvent::Merged {
                                    request_id: request.id,
                                    source: item.source.clone(),
                                    created: Vec::new(),
                                    overwritten: vec![target.clone()],
                                });
                            }
                        },
                    )
                    .await
                } else if request.move_sources {
                    move_local(
                        source,
                        target,
                        operation_cancellable.clone(),
                        Some(progress.clone()),
                    )
                    .await
                } else {
                    copy_new_recursively_with_progress(
                        source,
                        target,
                        operation_cancellable.clone(),
                        Some(progress.clone()),
                    )
                    .await
                };
                if let Err(error) = result {
                    let failure = (!was_cancelled(&error)).then(|| error.to_string());
                    stop_transfer(
                        &written_paths,
                        &emit,
                        request.id,
                        TransferStop {
                            completed,
                            failed: vec![item.source.clone()],
                            not_attempted: request.items[index + 1..]
                                .iter()
                                .map(|item| item.source.clone())
                                .collect(),
                            affected_locations,
                            failure,
                        },
                    )
                    .await;
                    return;
                }
                completed.push(item.source.clone());
                // A merge reports its written paths through Merged instead:
                // copy undo trashes recorded locations, and the merged folder
                // held pre-existing contents that must not be trashed.
                let created_location = target_location
                    .filter(|_| !request.move_sources && item.conflict != TransferConflict::Merge);
                progress.finish_item(item_started_at, item_sizes[index], created_location);
            }
            if !flush_removable_writes(
                written_paths,
                &operation_cancellable,
                &emit,
                request.id,
                &completed,
                affected_locations,
            )
            .await
            {
                return;
            }
            emit(OperationEvent::Pasted {
                request_id: request.id,
                locations: completed,
            });
        });
        cancellation_handle(cancellable)
    }

    fn undo_move(&self, request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let mut affected_locations = HashSet::new();
            for item in &request.items {
                for location in [&item.record.current, &item.record.original] {
                    affected_locations.insert(location.clone());
                    if let Some(parent) = location.parent() {
                        affected_locations.insert(parent);
                    }
                }
            }
            let sources = request
                .items
                .iter()
                .map(|item| gio_file_for_location(&item.record.current))
                .collect::<Vec<_>>();
            let (item_sizes, total_bytes) =
                match transfer_sizes(&sources, &operation_cancellable).await {
                    Ok(sizes) => sizes,
                    Err(error) if was_cancelled(&error) => {
                        emit(cancelled_event(
                            request.id,
                            Vec::new(),
                            Vec::new(),
                            request
                                .items
                                .iter()
                                .map(|item| item.record.current.clone())
                                .collect(),
                            affected_locations,
                        ));
                        return;
                    }
                    Err(error) => {
                        emit(OperationEvent::TransferFailed {
                            request_id: request.id,
                            completed_locations: Vec::new(),
                            message: error.to_string(),
                        });
                        return;
                    }
                };
            let progress = TransferProgressTracker::new(request.id, total_bytes, emit.clone());
            progress.emit();
            let mut completed = Vec::new();
            let mut restored_paths = Vec::new();
            for (index, item) in request.items.iter().enumerate() {
                let remaining = || {
                    request.items[index..]
                        .iter()
                        .map(|item| item.record.current.clone())
                        .collect::<Vec<_>>()
                };
                if operation_cancellable.is_cancelled() {
                    stop_transfer(
                        &restored_paths,
                        &emit,
                        request.id,
                        TransferStop {
                            completed,
                            failed: Vec::new(),
                            not_attempted: remaining(),
                            affected_locations,
                            failure: None,
                        },
                    )
                    .await;
                    return;
                }
                let source = sources[index].clone();
                let item_started_at = progress.transferred_bytes.get();
                let restored_path = item.record.original.native_path().map(Path::to_path_buf);
                if let Some(path) = restored_path {
                    restored_paths.push(path);
                }
                let target = gio_file_for_location(&item.record.original);
                let result = if item.conflict == TransferConflict::ReplaceExisting {
                    replace_local_with_progress(
                        source,
                        target,
                        true,
                        operation_cancellable.clone(),
                        Some(&mut affected_locations),
                        Some(progress.clone()),
                        &|| {},
                    )
                    .await
                } else {
                    move_local(
                        source,
                        target,
                        operation_cancellable.clone(),
                        Some(progress.clone()),
                    )
                    .await
                };
                if let Err(error) = result {
                    let failure = (!was_cancelled(&error)).then(|| error.to_string());
                    stop_transfer(
                        &restored_paths,
                        &emit,
                        request.id,
                        TransferStop {
                            completed,
                            failed: vec![item.record.current.clone()],
                            not_attempted: request.items[index + 1..]
                                .iter()
                                .map(|item| item.record.current.clone())
                                .collect(),
                            affected_locations,
                            failure,
                        },
                    )
                    .await;
                    return;
                }
                completed.push(item.record.current.clone());
                progress.finish_item(item_started_at, item_sizes[index], None);
            }
            if !flush_removable_writes(
                restored_paths,
                &operation_cancellable,
                &emit,
                request.id,
                &completed,
                affected_locations,
            )
            .await
            {
                return;
            }
            emit(OperationEvent::Pasted {
                request_id: request.id,
                locations: completed,
            });
        });
        cancellation_handle(cancellable)
    }

    fn undo_rename(
        &self,
        request: UndoRenameRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let current = request.current.clone();
            let original = request.original.clone();
            let mut affected_locations = HashSet::new();
            for location in [&current, &original] {
                affected_locations.insert(location.clone());
                if let Some(parent) = location.parent() {
                    affected_locations.insert(parent);
                }
            }
            let result = move_local(
                gio_file_for_location(&current),
                gio_file_for_location(&original),
                operation_cancellable,
                None,
            )
            .await;
            match result {
                Ok(()) => emit(OperationEvent::Renamed {
                    request_id: request.id,
                }),
                Err(error) if was_cancelled(&error) => emit(cancelled_event(
                    request.id,
                    Vec::new(),
                    Vec::new(),
                    vec![current],
                    affected_locations,
                )),
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error.to_string(),
                }),
            }
        });
        cancellation_handle(cancellable)
    }

    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let targets = request
                .entries
                .iter()
                .map(DeletionTarget::from_entry)
                .collect();
            run_deletion(
                request.id,
                targets,
                request.permanent,
                emit,
                operation_cancellable,
            )
            .await;
        });
        cancellation_handle(cancellable)
    }

    fn undo_copy(&self, request: UndoCopyRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let mut targets = Vec::with_capacity(request.locations.len());
            for location in &request.locations {
                targets.push(DeletionTarget::probed(location, &operation_cancellable).await);
            }
            run_deletion(request.id, targets, false, emit, operation_cancellable).await;
        });
        cancellation_handle(cancellable)
    }

    fn undo_merge(
        &self,
        request: UndoMergeRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            run_merge_undo(
                request.id,
                request.created,
                request.overwritten,
                emit,
                operation_cancellable,
                Rc::new(move |location, cancellable| {
                    trashed_merge_original(location, cancellable, request.originals.clone())
                }),
            )
            .await;
        });
        cancellation_handle(cancellable)
    }

    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let cancellable = gio::Cancellable::new();
        let operation_cancellable = cancellable.clone();
        let _task = glib::MainContext::default().spawn_local(async move {
            let entries = match request.source {
                RestoreSource::TrashEntries(items) => items
                    .into_iter()
                    .map(|item| RestoreEntry {
                        source: item.entry.location,
                        display_name: item.entry.display_name,
                        original_target: None,
                        trash_info: None,
                        confirmed_destination: Some(item.destination),
                        physical_path: item.entry.thumbnail_path,
                    })
                    .collect(),
                RestoreSource::OriginalLocations(locations) => match trashed_entries_for_originals(
                    &locations,
                    &operation_cancellable,
                    &HashMap::new(),
                )
                .await
                {
                    Ok(entries) => entries,
                    Err(error) if was_cancelled(&error) => {
                        emit(cancelled_event(
                            request.id,
                            Vec::new(),
                            Vec::new(),
                            locations,
                            HashSet::from([Location::uri("trash:///")]),
                        ));
                        return;
                    }
                    Err(error) => {
                        emit(OperationEvent::Failed {
                            request_id: request.id,
                            message: format!("Unable to find items in Trash: {error}"),
                        });
                        return;
                    }
                },
            };
            let total = entries.len();
            let mut errors = Vec::new();
            let mut restored_locations = Vec::new();
            let mut restored = Vec::new();
            let mut failed_locations = Vec::new();
            let mut affected_locations = HashSet::from([Location::uri("trash:///")]);
            let context = RestoreContext::current();
            for (index, entry) in entries.iter().enumerate() {
                if operation_cancellable.is_cancelled() {
                    emit(cancelled_event(
                        request.id,
                        restored,
                        failed_locations,
                        entries[index..]
                            .iter()
                            .map(|entry| entry.source.clone())
                            .collect(),
                        affected_locations,
                    ));
                    return;
                }
                let result = restore_trash_entry(
                    entry,
                    &context,
                    &mut affected_locations,
                    &operation_cancellable,
                )
                .await;
                let restored_location = if let Err(error) = result {
                    if was_cancelled(&error) {
                        failed_locations.push(entry.source.clone());
                        emit(cancelled_event(
                            request.id,
                            restored,
                            failed_locations,
                            entries[index + 1..]
                                .iter()
                                .map(|entry| entry.source.clone())
                                .collect(),
                            affected_locations,
                        ));
                        return;
                    }
                    errors.push(format!("{}: {error}", entry.display_name));
                    failed_locations.push(entry.source.clone());
                    None
                } else {
                    restored_locations.push(entry.source.clone());
                    restored.push(restored_destination(entry));
                    Some(restored_destination(entry))
                };
                emit(OperationEvent::RestoreProgress {
                    request_id: request.id,
                    completed: index + 1,
                    total,
                    restored_location,
                });
            }
            if errors.is_empty() {
                emit(OperationEvent::Restored {
                    request_id: request.id,
                    locations: restored_locations,
                    restored,
                });
            } else {
                emit(OperationEvent::RestoreCompletedWithErrors {
                    request_id: request.id,
                    restored_locations,
                    restored,
                    message: operation_error_summary(&errors, "restored"),
                });
            }
        });
        cancellation_handle(cancellable)
    }

    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        archive::compress(request, emit)
    }

    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        archive::extract(request, emit)
    }
}

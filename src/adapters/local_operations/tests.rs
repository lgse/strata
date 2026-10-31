// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    error::Error,
    ffi::{OsStr, OsString},
    fs,
    future::Future,
    io,
    os::{
        fd::{AsFd, OwnedFd},
        unix::{ffi::OsStringExt, fs::PermissionsExt},
    },
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::{Duration, Instant, SystemTime},
};

use gtk::{gio, glib, prelude::*};

use crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT;
use fs_quirks::{FsQuirks, Preset, Quirks};

fn serial() -> Result<crate::test_support::MainContextGuard<'static>, String> {
    ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())
}

mod aliases;
mod closes;
mod conflicts;
mod copy;
mod create_entry;
mod deletion;
mod file_copy;
mod folder_copy;
pub(super) mod fs_quirks;
mod leftovers;
mod merge;
mod moves;
mod naming;
mod paste_results;
mod paths;
mod placement;
mod progress;
mod release;
mod replacement;
mod restore_safety;
mod sync;
mod trash_capabilities;
mod undo;

/// File copies running at once, and the most seen, across copy workers.
static COPY_ACTIVITY: std::sync::Mutex<(usize, usize)> = std::sync::Mutex::new((0, 0));

fn copy_activity() -> std::sync::MutexGuard<'static, (usize, usize)> {
    COPY_ACTIVITY
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub(super) struct CopyActivity;

impl CopyActivity {
    pub(super) fn start() -> Self {
        let mut activity = copy_activity();
        activity.0 += 1;
        activity.1 = activity.1.max(activity.0);
        Self
    }
}

impl Drop for CopyActivity {
    fn drop(&mut self) {
        copy_activity().0 -= 1;
    }
}

fn fixing() -> NamePolicy {
    NamePolicy::new(NameMode::Fix)
}

fn copy_recursively_fixing(
    source: gio::File,
    target: gio::File,
    merge: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let names = fixing();
        if merge {
            return super::merge_copy(None, names)(source, target, Default::default(), cancellable)
                .await
                .map(drop);
        }
        copy_new_recursively_on_filesystem(
            source,
            target,
            Taken::Fail,
            cancellable,
            None,
            names,
            None,
        )
        .await
        .map(drop)
    })
}

fn recording_progress() -> (
    Rc<TransferProgressTracker>,
    Rc<RefCell<Vec<OperationEvent>>>,
) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1590),
        None,
        None,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    (progress, events)
}

fn followed_report(events: &[OperationEvent]) -> crate::services::TransferReport {
    let mut reported = crate::services::TransferReport::default();
    for event in events {
        if let OperationEvent::ItemsReported { report, .. } = event {
            reported.extend(report.clone());
        }
    }
    reported.follow_renames();
    reported
}

pub(super) fn renamed_beneath(
    target: &Path,
    report: &crate::services::TransferReport,
) -> Vec<(String, String, crate::services::RenameCause)> {
    let relative = |location: &Location| {
        location
            .native_path()
            .and_then(|path| path.strip_prefix(target).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut renamed = report
        .renamed
        .iter()
        .map(|item| {
            (
                relative(&item.requested),
                relative(&item.saved_as),
                item.cause,
            )
        })
        .collect::<Vec<_>>();
    renamed.sort_by(|left, right| left.0.cmp(&right.0));
    renamed
}

fn file_stage(root: &Path) -> Result<super::StagedSibling, Box<dyn Error>> {
    let parent = Arc::new(open_local_parent_directory(root)?);
    let (name, (_, created)) =
        super::copy_engine::stage::create_file(super::fs_ops::fs(), parent.as_fd())
            .map_err(|error| format!("{error:?}"))?;
    Ok(super::StagedSibling {
        ownership: None,
        parent,
        name,
        identity: created.identity,
        restrictive: Vec::new(),
    })
}

fn staged(
    root: &Path,
    content: super::StageContent,
) -> Result<super::StagedSibling, Box<dyn Error>> {
    let parent = Arc::new(open_local_parent_directory(root)?);
    Ok(super::StagedSibling::create(parent, content).map_err(|error| format!("{error:?}"))?)
}

use super::{
    CopyStep, DeletionTarget, FailedAction, LocalFileIdentity, LocalOperationProvider, MergeCopied,
    MergeHooks, MergePlan, NamePolicy, PinnedEntry, RestoreEntry, StageOverwrite,
    StagedOriginalLookup, TRANSFER_PROGRESS_INTERVAL, TransferProgressTracker, TransferStop,
    TrashError, await_cancellable, copy_failure_after_cleanup, copy_failure_on_fat32,
    copy_new_recursively_on_filesystem, copy_new_remote_file_with, copy_recursively,
    deletion_error_message, deletion_error_summary, io_error, join_local_tasks,
    local_file_identity, merge_local, merge_local_with, move_local, move_local_with,
    names::{Taken, numbered_name, numbered_names, parse_copy_suffix},
    open_local_parent_directory, operation_error_summary, parallel_delete_local,
    permanently_delete_local, permanently_delete_local_path_if_unchanged, replace_local,
    replace_local_with, run_deletion_in, run_merge_undo, set_removable_roots_for_test,
    set_sync_observer, stop_transfer, sync_probe_observations, transfer_is_noop, transfer_sizes,
    trash_entries_at, trash_stage_overwrite, validated_child, was_cancelled,
};
use crate::{
    adapters::volume::FAT32_MAX_FILE_SIZE,
    model::{EntryKind, FileEntry, Location, MetadataValue, hidden_names::HiddenName},
    services::{
        DeleteRequest, ItemIdentity, LoadHandle, MoveRecord, NameMode, OperationEvent,
        OperationProvider, OperationRequestId, PasteItem, PasteRequest, PlacedSnapshot,
        RestoreRequest, RestoreSource, RestoreTrashItem, TransferConflict, UndoMoveItem,
        UndoMoveRequest, UndoRenameRequest,
    },
};

struct ClockGuard;

impl ClockGuard {
    fn set(now: SystemTime) -> Self {
        super::copy_engine::set_clock_for_test(Some(now));
        Self
    }
}

impl Drop for ClockGuard {
    fn drop(&mut self) {
        super::copy_engine::set_clock_for_test(None);
    }
}

struct KernelGuard;

impl KernelGuard {
    fn set(release: Option<(u32, u32, u32)>) -> Self {
        super::copy_engine::set_kernel_for_test(release);
        Self
    }
}

impl Drop for KernelGuard {
    fn drop(&mut self) {
        super::copy_engine::set_kernel_for_test(None);
    }
}

struct PlacedEntriesGuard;

impl PlacedEntriesGuard {
    fn set(limit: usize) -> Self {
        super::copy_engine::set_placed_entries_for_test(Some(limit));
        Self
    }
}

impl Drop for PlacedEntriesGuard {
    fn drop(&mut self) {
        super::copy_engine::set_placed_entries_for_test(None);
    }
}

struct RemovableFlushGuard;

impl RemovableFlushGuard {
    fn install(root: &Path, fail: Option<io::ErrorKind>) -> Self {
        set_removable_roots_for_test(Some(vec![root.to_path_buf()]));
        super::install_sync_probe(fail, false);
        Self
    }
}

fn separate_volumes(source: &Path, destination: &Path) -> (FsQuirks, FsQuirks) {
    (
        FsQuirks::install(source, Quirks::default()),
        FsQuirks::install(destination, Quirks::default()),
    )
}

impl Drop for RemovableFlushGuard {
    fn drop(&mut self) {
        super::release_sync_probe();
        super::clear_sync_probe();
        set_sync_observer(None);
        set_removable_roots_for_test(None);
    }
}

pub(super) fn names(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

/// Where inode numbers are not trusted, a file born within one birth-time tick
/// of `than` passes for it.
fn wait_for_a_later_birth_time(probe: &Path, than: &fs::Metadata) {
    let Ok(born) = than.created() else {
        return;
    };
    loop {
        fs::write(probe, b"probe").expect("write the probe");
        let stamped = fs::metadata(probe).expect("stat the probe");
        fs::remove_file(probe).expect("remove the probe");
        if stamped.created().is_ok_and(|later| later > born) {
            break;
        }
    }
}

fn set_times(path: &Path, seconds: u64, nanoseconds: u32) -> io::Result<()> {
    let times = rustix::fs::Timespec {
        tv_sec: i64::try_from(seconds).map_err(io::Error::other)?,
        tv_nsec: nanoseconds.into(),
    };
    rustix::fs::utimensat(
        rustix::fs::CWD,
        path,
        &rustix::fs::Timestamps {
            last_access: times,
            last_modification: times,
        },
        rustix::fs::AtFlags::empty(),
    )?;
    Ok(())
}

fn base_takes_noreplace(folder: &Path) -> io::Result<bool> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    let (one, other) = (folder.join("rename-probe"), folder.join("rename-probe-2"));
    fs::write(&one, b"one")?;
    let renamed = renameat_with(CWD, &one, CWD, &other, RenameFlags::NOREPLACE).is_ok();
    for probe in [&one, &other] {
        let _ = fs::remove_file(probe);
    }
    Ok(renamed)
}

fn base_takes_rename_flags(folder: &Path) -> io::Result<bool> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    if !base_takes_noreplace(folder)? {
        return Ok(false);
    }
    let (one, other) = (folder.join("rename-probe"), folder.join("rename-probe-2"));
    fs::write(&one, b"one")?;
    fs::write(&other, b"other")?;
    let exchanged = renameat_with(CWD, &one, CWD, &other, RenameFlags::EXCHANGE).is_ok();
    // exfat-fuse accepts RENAME_EXCHANGE and overwrites instead of swapping.
    let overwrote = exchanged && fs::symlink_metadata(&one).is_err();
    for probe in [&one, &other] {
        let _ = fs::remove_file(probe);
    }
    Ok(!overwrote)
}

fn hard_linked(original: &Path, link: &Path) -> io::Result<bool> {
    match fs::hard_link(original, link) {
        Ok(()) => Ok(true),
        Err(error) if error.raw_os_error() == Some(rustix::io::Errno::PERM.raw_os_error()) => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn base_links_keep_their_number(folder: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let (one, other) = (folder.join("link-probe"), folder.join("link-probe-2"));
    fs::write(&one, b"one")?;
    let kept = hard_linked(&one, &other)?
        && fs::symlink_metadata(&one)?.ino() == fs::symlink_metadata(&other)?.ino();
    for probe in [&one, &other] {
        let _ = fs::remove_file(probe);
    }
    Ok(kept)
}

pub(super) fn base_places_files_by_linking(folder: &Path) -> io::Result<bool> {
    Ok(!base_takes_noreplace(folder)? && base_links_keep_their_number(folder)?)
}

fn forced_mode(path: &Path) -> io::Result<u32> {
    use rustix::fs::{AtFlags, CWD, StatxFlags};
    let stat = rustix::fs::statx(
        CWD,
        path,
        AtFlags::SYMLINK_NOFOLLOW | AtFlags::STATX_FORCE_SYNC,
        StatxFlags::TYPE | StatxFlags::MODE,
    )?;
    Ok(u32::from(stat.stx_mode))
}

fn a_later_change_time(path: &Path, than: &Path, seconds: u64) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let changed = |metadata: &fs::Metadata| (metadata.ctime(), metadata.ctime_nsec());
    let earlier = changed(&fs::symlink_metadata(than)?);
    loop {
        set_times(path, seconds, 0)?;
        let stamped = fs::symlink_metadata(path)?;
        if changed(&stamped) == (stamped.mtime(), stamped.mtime_nsec()) {
            return Ok(false);
        }
        if changed(&stamped) > earlier {
            return Ok(true);
        }
    }
}

/// Sets the times of each file beneath `root` again from outside the seam,
/// as Samba does once the SMB client's deferred close of a file it wrote
/// reaches it: only the change time moves.
fn land_deferred_closes(root: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let before = fs::symlink_metadata(&path)?;
        if before.is_dir() {
            land_deferred_closes(&path)?;
            continue;
        }
        if !before.is_file() {
            continue;
        }
        let time = |tv_sec, tv_nsec| rustix::fs::Timespec { tv_sec, tv_nsec };
        let times = rustix::fs::Timestamps {
            last_access: time(before.atime(), before.atime_nsec()),
            last_modification: time(before.mtime(), before.mtime_nsec()),
        };
        // A kernel without fine-grained times stamps within its tick.
        let deadline = Instant::now() + CHANGE_TIME_WAIT;
        loop {
            rustix::fs::utimensat(rustix::fs::CWD, &path, &times, rustix::fs::AtFlags::empty())?;
            let after = fs::symlink_metadata(&path)?;
            if (after.ctime(), after.ctime_nsec()) != (before.ctime(), before.ctime_nsec()) {
                break;
            }
            if Instant::now() >= deadline {
                return Err(io::Error::other(format!(
                    "the change time of {} did not move within {CHANGE_TIME_WAIT:?}; this mount \
                     may report the modification time as the change time",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

const CHANGE_TIME_WAIT: Duration = Duration::from_secs(2);

fn has_colon(name: &OsStr) -> Option<rustix::io::Errno> {
    name.as_encoded_bytes()
        .contains(&b':')
        .then_some(rustix::io::Errno::INVAL)
}

async fn copy_new_recursively_with_progress(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
) -> Result<(), glib::Error> {
    copy_new_recursively_on_filesystem(
        source,
        target,
        Taken::Fail,
        cancellable,
        progress,
        NamePolicy::default(),
        None,
    )
    .await
    .map(drop)
}

async fn copy_new_recursively(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
) -> Result<(), glib::Error> {
    copy_new_recursively_with_progress(source, target, cancellable, None).await
}

fn reporting(
    source: &Path,
    destination: super::copy_engine::Destination,
    options: super::copy_engine::Options,
) -> (Result<(), glib::Error>, crate::services::TransferReport) {
    let reported = Rc::new(RefCell::new(crate::services::TransferReport::default()));
    let events = reported.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| {
            if let OperationEvent::ItemsReported { report, .. } = event {
                events.borrow_mut().extend(report);
            }
        }),
    );
    let copied = glib::MainContext::default().block_on(super::copy_engine::copy_local(
        source.to_path_buf(),
        destination,
        options,
        Some(progress),
        &gio::Cancellable::new(),
    ));
    (copied.map(drop), reported.take())
}

fn copy_reporting(
    source: &Path,
    target: &Path,
    workers: Option<usize>,
) -> Result<crate::services::TransferReport, Box<dyn Error>> {
    let destination = super::copy_engine::Destination::New(target.to_path_buf(), Taken::Fail);
    let (copied, reported) = reporting(source, destination, workers_only(workers));
    copied?;
    Ok(reported)
}

fn merge_reporting(
    source: &Path,
    target: &Path,
    workers: Option<usize>,
) -> (Result<(), glib::Error>, crate::services::TransferReport) {
    let destination = super::copy_engine::Destination::Merge(target.to_path_buf(), None);
    reporting(source, destination, workers_only(workers))
}

fn placed_now(paths: &[&Path]) -> HashMap<Location, PlacedSnapshot> {
    paths
        .iter()
        .filter_map(|path| {
            let placed = glib::MainContext::default().block_on(super::copy_engine::placed(
                path.to_path_buf(),
                super::copy_engine::placed_budget(),
                &gio::Cancellable::new(),
            ))?;
            Some((Location::local(path), placed))
        })
        .collect()
}

fn workers_only(workers: Option<usize>) -> super::copy_engine::Options {
    super::copy_engine::Options {
        workers,
        ..super::copy_engine::Options::default()
    }
}

fn terminal_transfer(events: &[OperationEvent]) -> Option<&OperationEvent> {
    events.iter().rev().find(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. }
                | OperationEvent::TransferFailed { .. }
                | OperationEvent::Cancelled { .. }
        )
    })
}

fn item_moves(events: &[OperationEvent]) -> Vec<(Location, Location)> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemMoved { from, to, .. } => Some((from.clone(), to.clone())),
            _ => None,
        })
        .collect()
}

fn trashed_items(events: &[OperationEvent]) -> Vec<(Location, (u64, u64))> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemTrashed {
                location, identity, ..
            } => Some((location.clone(), numbers(*identity))),
            _ => None,
        })
        .collect()
}

fn numbers(identity: ItemIdentity) -> (u64, u64) {
    (identity.evidence.device, identity.evidence.inode)
}

fn numbers_at(path: &Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).expect("identity");
    (metadata.dev(), metadata.ino())
}

fn identity_off_main(path: &Path) -> Option<ItemIdentity> {
    let path = path.to_owned();
    std::thread::spawn(move || super::native_fs::identity_at_path(&path))
        .join()
        .ok()
        .flatten()
}

fn pump_until_transfer(events: &RefCell<Vec<OperationEvent>>) {
    let start = Instant::now();
    while terminal_transfer(&events.borrow()).is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timed out waiting for the transfer to finish: {:?}",
            events.borrow()
        );
        // Own the default context for the whole wait. Releasing it between
        // polls lets a GIO copy worker run a progress callback inline.
        glib::MainContext::default().iteration(true);
    }
}

fn watch_source_during_sync(source: &Path) -> Arc<std::sync::Mutex<Vec<bool>>> {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let source = source.to_path_buf();
    set_sync_observer(Some(Arc::new(move |_| {
        record
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(source.exists());
    })));
    seen
}

fn file_entry(path: &std::path::Path) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: path.file_name().unwrap_or_default().to_owned(),
        display_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}

fn directory_entry(path: &std::path::Path) -> FileEntry {
    FileEntry {
        kind: EntryKind::Directory,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        ..file_entry(path)
    }
}

fn settle_cancelled_io(context: &glib::MainContext) {
    context.block_on(glib::timeout_future(Duration::from_millis(25)));
    while context.pending() {
        context.iteration(false);
    }
}

fn always_copy() -> super::MoveAttempt {
    Rc::new(|_, _, _, _| Box::pin(async { Ok(super::Moved::Copy) }))
}

fn wait_for_operation(
    events: &RefCell<Vec<OperationEvent>>,
    is_terminal: impl Fn(&OperationEvent) -> bool,
) {
    while !events.borrow().iter().any(&is_terminal) {
        glib::MainContext::default().iteration(true);
    }
}

fn drive_until_rename_settles(events: &Rc<RefCell<Vec<OperationEvent>>>) {
    wait_for_operation(events, |event| {
        matches!(
            event,
            OperationEvent::Renamed { .. }
                | OperationEvent::Failed { .. }
                | OperationEvent::Cancelled { .. }
        )
    });
}

fn settle(start: impl FnOnce(Rc<dyn Fn(OperationEvent)>) -> LoadHandle) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = start(Rc::new(move |event| emitted.borrow_mut().push(event)));
    drive_until_rename_settles(&events);
    events.take()
}

fn rename(path: &Path, new_name: &str) -> Vec<OperationEvent> {
    let entry = file_entry(path);
    settle(|emit| {
        LocalOperationProvider.rename(
            crate::services::RenameRequest {
                id: OperationRequestId(46),
                entry,
                new_name: new_name.to_owned(),
            },
            emit,
        )
    })
}

/// Cancels from the main thread the first time a racer runs, holding that racer's worker
/// until then, as a user's cancel lands while the worker is inside an item.
#[derive(Clone, Default)]
struct CancelFromMain(Arc<(std::sync::Mutex<Option<bool>>, std::sync::Condvar)>);

impl CancelFromMain {
    fn racer(&self) -> fs_quirks::Racer {
        let this = self.clone();
        Box::new(move || {
            let (state, changed) = &*this.0;
            let mut state = state.lock().expect("cancel state");
            if state.is_none() {
                *state = Some(false);
                while *state == Some(false) {
                    state = changed.wait(state).expect("cancel state");
                }
            }
        })
    }

    fn waiter(&self) -> fs_quirks::Racer {
        let this = self.clone();
        Box::new(move || {
            let (state, changed) = &*this.0;
            let mut state = state.lock().expect("cancel state");
            while *state != Some(true) {
                state = changed.wait(state).expect("cancel state");
            }
        })
    }

    fn serve(&self, cancel: impl FnOnce()) {
        let (state, changed) = &*self.0;
        let mut state = state.lock().expect("cancel state");
        if *state == Some(false) {
            cancel();
            *state = Some(true);
            changed.notify_all();
        }
    }

    fn served(&self) -> bool {
        *self.0.0.lock().expect("cancel state") == Some(true)
    }
}

fn wait_for_restore(events: &Rc<RefCell<Vec<OperationEvent>>>) {
    wait_for_operation(events, |event| {
        matches!(
            event,
            OperationEvent::Restored { .. }
                | OperationEvent::RestoreCompletedWithErrors { .. }
                | OperationEvent::Failed { .. }
                | OperationEvent::Cancelled { .. }
        )
    });
}

fn volume_trash_entry(
    root: &Path,
    name: &str,
    orig_path: &str,
    contents: &[u8],
) -> Result<(PathBuf, FileEntry), Box<dyn Error>> {
    let uid = rustix::process::getuid().as_raw();
    let trash = root.join(format!(".Trash-{uid}"));
    fs::create_dir_all(trash.join("files"))?;
    fs::create_dir_all(trash.join("info"))?;
    let source = trash.join("files").join(name);
    fs::write(&source, contents)?;
    fs::write(
        trash.join("info").join(format!("{name}.trashinfo")),
        format!("[Trash Info]\nPath={orig_path}\nDeletionDate=2026-01-01T00:00:00\n"),
    )?;
    Ok((source.clone(), file_entry(&source)))
}

fn drive_until_transfer_settles(events: &Rc<RefCell<Vec<OperationEvent>>>) {
    wait_for_operation(events, |event| {
        matches!(
            event,
            OperationEvent::Pasted { .. }
                | OperationEvent::TransferFailed { .. }
                | OperationEvent::Cancelled { .. }
        )
    });
}

fn run_paste_collecting_created(
    request: PasteRequest,
) -> Result<Vec<Option<Location>>, Box<dyn Error>> {
    let created = run_paste(request)
        .iter()
        .filter_map(|event| match event {
            OperationEvent::TransferProgress {
                created_location, ..
            } => Some(created_location.clone()),
            _ => None,
        })
        .collect();
    Ok(created)
}

fn run_paste(request: PasteRequest) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        request,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    wait_for_operation(&events, |event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    });

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    events.take()
}

fn sequential_delete_local(
    parent: OwnedFd,
    name: OsString,
) -> Pin<Box<dyn Future<Output = Result<(), String>>>> {
    Box::pin(async move {
        let step_parent = parent.try_clone().map_err(|error| error.to_string())?;
        let step_name = name.clone();
        let step = super::run_local_delete_step(move || {
            super::open_local_delete_target(&step_parent, &step_name, None, false)
        })
        .await
        .map_err(|error| error.to_string())?;
        let super::LocalDeleteStep::Directory { handle, children } = step else {
            return Ok(());
        };
        for child in children {
            let checked_parent = parent.try_clone().map_err(|error| error.to_string())?;
            let checked_handle = handle.try_clone().map_err(|error| error.to_string())?;
            let checked_name = name.clone();
            super::run_local_delete_step(move || {
                super::ensure_local_delete_target_unchanged(
                    &checked_parent,
                    &checked_name,
                    &checked_handle,
                )
            })
            .await
            .map_err(|error| error.to_string())?;
            sequential_delete_local(
                handle.try_clone().map_err(|error| error.to_string())?,
                child,
            )
            .await?;
        }
        super::run_local_delete_step(move || {
            super::ensure_local_delete_target_unchanged(&parent, &name, &handle)?;
            rustix::fs::unlinkat(&parent, &name, rustix::fs::AtFlags::REMOVEDIR)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())
    })
}

fn sync_delete_benchmark_filesystem(root: &Path) -> Result<(), Box<dyn Error>> {
    let handle = rustix::fs::open(
        root,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    rustix::fs::syncfs(handle)?;
    Ok(())
}

fn create_delete_benchmark_tree(root: &Path, count: usize) -> io::Result<()> {
    const DIRECTORIES: usize = 256;
    fs::create_dir(root)?;
    for directory in 0..DIRECTORIES.min(count.max(1)) {
        fs::create_dir(root.join(format!("dir-{directory}")))?;
    }
    for index in 0..count {
        fs::File::create(
            root.join(format!("dir-{}", index % DIRECTORIES))
                .join(format!("item-{index}")),
        )?;
    }
    Ok(())
}

#[test]
#[ignore = "manual performance benchmark; run scripts/benchmark-delete.sh"]
fn benchmark_delete_large_directory() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let count = std::env::var("STRATA_DELETE_BENCH_FILES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100_000);
    let benchmark_root = std::env::var_os("STRATA_DELETE_BENCH_ROOT")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?.join("target/delete-benchmark"));
    fs::create_dir_all(&benchmark_root)?;
    let serial_root = benchmark_root.join(format!("serial-{unique}"));
    let available_workers = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1);
    let mut worker_counts = vec![1, 2, 4, available_workers];
    worker_counts.sort_unstable();
    worker_counts.dedup();
    worker_counts.retain(|workers| *workers <= available_workers);
    let worker_roots = worker_counts
        .iter()
        .map(|workers| {
            (
                *workers,
                benchmark_root.join(format!("workers-{workers}-{unique}")),
            )
        })
        .collect::<Vec<_>>();
    create_delete_benchmark_tree(&serial_root, count)?;
    for (_, root) in &worker_roots {
        create_delete_benchmark_tree(root, count)?;
    }
    sync_delete_benchmark_filesystem(&benchmark_root)?;
    let context = glib::MainContext::default();

    let serial_parent = super::open_local_parent_directory(
        serial_root.parent().ok_or("benchmark root has no parent")?,
    )?;
    let serial_name = serial_root
        .file_name()
        .ok_or("benchmark root has no name")?
        .to_owned();
    let serial_started = Instant::now();
    context
        .block_on(sequential_delete_local(serial_parent, serial_name))
        .map_err(io::Error::other)?;
    let serial_elapsed = serial_started.elapsed();
    sync_delete_benchmark_filesystem(&benchmark_root)?;

    let mut measurements = Vec::new();
    let mut detected_workers = None;
    for (workers, root) in &worker_roots {
        let parent = super::open_local_parent_directory(
            root.parent().ok_or("benchmark root has no parent")?,
        )?;
        let name = root
            .file_name()
            .ok_or("benchmark root has no name")?
            .to_owned();
        let target_root = super::PinnedEntry {
            parent: Arc::new(parent),
            name,
            identity: None,
        };
        detected_workers = Some(super::local_delete_worker_count(std::slice::from_ref(
            &target_root,
        )));
        let parallel_started = Instant::now();
        super::parallel_delete_local_blocking_with_workers(
            vec![target_root],
            Arc::new(AtomicBool::new(false)),
            *workers,
            &Default::default(),
        )
        .map_err(io::Error::other)?;
        let parallel_elapsed = parallel_started.elapsed();
        measurements.push((*workers, parallel_elapsed));
        sync_delete_benchmark_filesystem(&benchmark_root)?;
    }
    let production_workers = detected_workers.unwrap_or(1);
    let (_, production_elapsed) = measurements
        .iter()
        .find(|measurement| measurement.0 == production_workers)
        .copied()
        .ok_or("missing production worker measurement")?;
    let single_elapsed = measurements
        .iter()
        .find(|measurement| measurement.0 == 1)
        .map(|measurement| measurement.1)
        .ok_or("missing single-worker measurement")?;

    println!("files: {count}; production workers: {production_workers}");
    println!(
        "legacy sequential: {:.3}s ({:.0} files/s)",
        serial_elapsed.as_secs_f64(),
        count as f64 / serial_elapsed.as_secs_f64().max(f64::EPSILON)
    );
    for (workers, elapsed) in &measurements {
        println!(
            "workers={workers}: elapsed={:.3}s ({:.0} files/s)",
            elapsed.as_secs_f64(),
            count as f64 / elapsed.as_secs_f64().max(f64::EPSILON)
        );
    }
    println!(
        "production parallel speedup: {:.2}x; end-to-end speedup: {:.2}x",
        single_elapsed.as_secs_f64() / production_elapsed.as_secs_f64().max(f64::EPSILON),
        serial_elapsed.as_secs_f64() / production_elapsed.as_secs_f64().max(f64::EPSILON)
    );
    assert!(!serial_root.exists());
    assert!(worker_roots.iter().all(|(_, root)| !root.exists()));
    Ok(())
}

#[test]
#[ignore = "manual scale benchmark; run STRATA_DELETE_BENCH_SCALE=1 scripts/benchmark-delete.sh"]
fn benchmark_parallel_delete_scale() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let count = std::env::var("STRATA_DELETE_BENCH_FILES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1_000_000);
    let benchmark_root = std::env::var_os("STRATA_DELETE_BENCH_ROOT")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?.join("target/delete-benchmark"));
    fs::create_dir_all(&benchmark_root)?;
    let root = benchmark_root.join(format!("scale-{unique}"));
    create_delete_benchmark_tree(&root, count)?;
    sync_delete_benchmark_filesystem(&benchmark_root)?;

    let parent =
        super::open_local_parent_directory(root.parent().ok_or("benchmark root has no parent")?)?;
    let name = root
        .file_name()
        .ok_or("benchmark root has no name")?
        .to_owned();
    let target_root = super::PinnedEntry {
        parent: Arc::new(parent),
        name,
        identity: None,
    };
    let workers = super::local_delete_worker_count(std::slice::from_ref(&target_root));
    let cleanup_started = Instant::now();
    super::parallel_delete_local_blocking_with_workers(
        vec![target_root],
        Arc::new(AtomicBool::new(false)),
        workers,
        &Default::default(),
    )
    .map_err(io::Error::other)?;
    let cleanup_elapsed = cleanup_started.elapsed();

    println!(
        "files={count} workers={workers} elapsed={:.3}s ({:.0} files/s)",
        cleanup_elapsed.as_secs_f64(),
        count as f64 / cleanup_elapsed.as_secs_f64().max(f64::EPSILON)
    );
    assert!(!root.exists());
    Ok(())
}

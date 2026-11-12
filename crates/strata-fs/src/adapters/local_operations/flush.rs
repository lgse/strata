// SPDX-License-Identifier: MIT

use super::*;

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

pub(super) fn blocking_join_message(error: Box<dyn std::any::Any + Send>) -> String {
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

pub(super) fn device_flush_failure(error: &str) -> String {
    rust_i18n::t!(
        "The device could not finish writing: %{error}. Earlier writes may still be pending; wait for safe eject before unplugging.",
        error = error
    )
    .into_owned()
}

pub(crate) fn sync_filesystem(root: &Path) -> io::Result<()> {
    let handle = fs()
        .open_path(
            root.as_os_str(),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(io::Error::from)?;
    fs().sync_fs(std::os::fd::AsFd::as_fd(&handle))
        .map_err(io::Error::from)?;
    Ok(())
}

pub fn flush_filesystem(root: &Path) -> io::Result<()> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(kind) = sync_probe_before(root) {
        return Err(io::Error::new(kind, "flush failed"));
    }
    sync_filesystem(root)
}

#[cfg(any(test, feature = "test-support"))]
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
    pub struct SyncObservation {
        pub root: PathBuf,
        pub thread_id: thread::ThreadId,
    }

    pub fn install_sync_probe(fail: Option<io::ErrorKind>, gate: bool) {
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

    pub fn clear_sync_probe() {
        release_sync_probe();
        let mut slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        *slot = None;
    }

    #[cfg(test)]
    pub fn set_sync_observer(observer: Option<SyncObserver>) {
        let mut slot = SYNC_OBSERVER
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *slot = observer;
    }

    pub fn release_sync_probe() {
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

    pub fn sync_probe_is_blocked() -> bool {
        let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        slot.as_ref()
            .and_then(|probe| probe.gate.as_ref())
            .is_some_and(|gate| gate.started.load(Ordering::SeqCst))
    }

    pub fn sync_probe_len() -> usize {
        let slot = SYNC_PROBE.lock().unwrap_or_else(|error| error.into_inner());
        slot.as_ref().map(|probe| probe.calls.len()).unwrap_or(0)
    }

    pub fn sync_probe_observations() -> Vec<SyncObservation> {
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
pub(crate) use sync_probe::set_sync_observer;
#[cfg(any(test, feature = "test-support"))]
use sync_probe::sync_probe_before;
#[cfg(any(test, feature = "test-support"))]
pub use sync_probe::{
    clear_sync_probe, install_sync_probe, release_sync_probe, sync_probe_is_blocked,
    sync_probe_len, sync_probe_observations,
};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static REMOVABLE_ROOTS_FOR_TEST: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_removable_roots_for_test(roots: Option<Vec<PathBuf>>) {
    REMOVABLE_ROOTS_FOR_TEST.with(|slot| *slot.borrow_mut() = roots);
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn removable_hints_for_test() -> Option<Vec<MountFlushHint>> {
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

/// Reads the mount table off the main thread; the volume monitor is
/// in-memory GIO state and is matched here.
pub(super) async fn removable_roots_for_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        return Vec::new();
    }
    let destinations: Vec<_> = paths
        .iter()
        .map(|path| SyncDestination::Local(path))
        .collect();
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hints) = removable_hints_for_test() {
        return removable_sync_roots(&destinations, &hints);
    }
    let table = mount_watch::fresh().await;
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
    removable_sync_roots(&destinations, &hints)
}

pub(super) fn mount_drive_is_removable(mount: &gio::Mount) -> bool {
    let drive = mount
        .drive()
        .or_else(|| mount.volume().and_then(|volume| volume.drive()));
    drive.is_some_and(|drive| drive.is_removable() || drive.is_media_removable())
}

pub(super) async fn flush_removable_writes(
    paths: Vec<PathBuf>,
    cancellable: &gio::Cancellable,
    emit: &Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
    completed: &[Location],
    affected_locations: HashSet<Location>,
) -> bool {
    let roots = removable_roots_for_paths(&paths).await;
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
            Ok(Err(error)) => os_error_text(error),
            Err(error) => blocking_join_message(error),
        };
        emit(OperationEvent::TransferFailed {
            request_id,
            completed_locations: completed.to_vec(),
            message: device_flush_failure(&error),
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

pub(super) async fn flush_written_roots(
    paths: &[PathBuf],
    emit: &Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
) -> Result<(), String> {
    let roots = removable_roots_for_paths(paths).await;
    if roots.is_empty() {
        return Ok(());
    }
    emit(OperationEvent::FlushingToDevice { request_id });
    for root in roots {
        let result = gio::spawn_blocking(move || flush_filesystem(&root)).await;
        let error = match result {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => os_error_text(error),
            Err(error) => blocking_join_message(error),
        };
        return Err(device_flush_failure(&error));
    }
    Ok(())
}

pub(super) struct TransferStop {
    pub(super) completed: Vec<Location>,
    pub(super) failed: Vec<Location>,
    pub(super) not_attempted: Vec<Location>,
    pub(super) affected_locations: HashSet<Location>,
    pub(super) failure: Option<String>,
    pub(super) skip_flush: bool,
}

pub(super) fn joined_failures(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(crate::i18n::join_sentences(&first, &second)),
        (first, second) => first.or(second),
    }
}

pub(super) async fn stop_transfer(
    paths: &[PathBuf],
    emit: &Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
    stop: TransferStop,
) {
    let flush_error = if stop.skip_flush {
        None
    } else {
        flush_written_roots(paths, emit, request_id).await.err()
    };
    match (stop.failure, flush_error) {
        (failure, Some(flush_error)) => emit(OperationEvent::TransferFailed {
            request_id,
            completed_locations: stop.completed,
            message: failure.map_or(flush_error.clone(), |message| {
                crate::i18n::join_sentences(&message, &flush_error)
            }),
        }),
        (Some(message), None) => emit(OperationEvent::TransferFailed {
            request_id,
            completed_locations: stop.completed,
            message,
        }),
        (None, None) => emit(cancelled_event(
            request_id,
            stop.completed,
            stop.failed,
            stop.not_attempted,
            stop.affected_locations,
        )),
    }
}

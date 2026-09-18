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
        fd::OwnedFd,
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

mod conflicts;
mod copy;
mod create_entry;
mod deletion;
mod merge;
mod moves;
mod naming;
mod paste_results;
mod paths;
mod progress;
mod replacement;
mod restore_safety;
mod trash_capabilities;
mod undo;

use super::{
    LocalDeleteRoot, LocalFileIdentity, LocalOperationProvider, MergeHooks, MergePlan, MountTable,
    RestoreEntry, StageCopy, StageOverwrite, StagedOriginalLookup, TransferProgressTracker,
    await_cancellable, bounded_local_delete_worker_count, copy_failure_after_cleanup,
    copy_new_recursively, copy_new_remote_file_with, copy_recursively, copy_recursively_fat_family,
    deletion_error_message, deletion_error_summary, duplicate_candidate_name, fat_sanitized_name,
    home_trash_entries_at, io_error, is_trash_unsupported_failure, local_file_identity,
    merge_local, merge_local_with, move_local, move_local_with, open_local_parent_directory,
    operation_error_summary, parallel_delete_local, parse_copy_suffix, permanently_delete_local,
    permanently_delete_local_path_if_unchanged, replace_local, replace_local_with, run_merge_undo,
    target_is_fat_family, transfer_is_noop, trash_stage_overwrite, unique_fat_sibling_name,
    validated_child, was_cancelled,
};
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        DeleteRequest, LoadHandle, MoveRecord, OperationEvent, OperationProvider,
        OperationRequestId, PasteItem, PasteRequest, RestoreRequest, RestoreSource,
        RestoreTrashItem, TransferConflict, TrashedOriginal, UndoMoveItem, UndoMoveRequest,
        UndoRenameRequest,
    },
};

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
    }
}

fn directory_entry(path: &std::path::Path) -> FileEntry {
    FileEntry {
        kind: EntryKind::Directory,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        ..file_entry(path)
    }
}

fn settle_cancelled_io(context: &glib::MainContext) {
    context.block_on(glib::timeout_future(Duration::from_millis(25)));
    while context.pending() {
        context.iteration(false);
    }
}

fn always_would_recurse() -> super::MoveAttempt {
    Rc::new(|_, _, _| {
        Box::pin(async {
            Err(glib::Error::new(
                gio::IOErrorEnum::WouldRecurse,
                "injected cross-filesystem move failure",
            ))
        })
    })
}

fn wait_for_operation(
    events: &RefCell<Vec<OperationEvent>>,
    is_terminal: impl Fn(&OperationEvent) -> bool,
) {
    while !events.borrow().iter().any(&is_terminal) {
        glib::MainContext::default().iteration(true);
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

    let created = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            OperationEvent::TransferProgress {
                created_location, ..
            } => Some(created_location.clone()),
            _ => None,
        })
        .collect();
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    Ok(created)
}

fn sequential_delete_local(
    parent: OwnedFd,
    name: OsString,
) -> Pin<Box<dyn Future<Output = Result<(), String>>>> {
    Box::pin(async move {
        let step_parent = parent.try_clone().map_err(|error| error.to_string())?;
        let step_name = name.clone();
        let step = super::run_local_delete_step(move || {
            super::open_local_delete_target(&step_parent, &step_name, None)
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
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
        let target_root = super::LocalDeleteRoot {
            parent: Arc::new(parent),
            name,
            expected: None,
        };
        detected_workers = Some(super::local_delete_worker_count(std::slice::from_ref(
            &target_root,
        )));
        let parallel_started = Instant::now();
        super::parallel_delete_local_blocking_with_workers(
            vec![target_root],
            Arc::new(AtomicBool::new(false)),
            *workers,
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
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
    let target_root = super::LocalDeleteRoot {
        parent: Arc::new(parent),
        name,
        expected: None,
    };
    let workers = super::local_delete_worker_count(std::slice::from_ref(&target_root));
    let cleanup_started = Instant::now();
    super::parallel_delete_local_blocking_with_workers(
        vec![target_root],
        Arc::new(AtomicBool::new(false)),
        workers,
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

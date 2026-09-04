// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    error::Error,
    ffi::{OsStr, OsString},
    fs,
    io::{Cursor, Write},
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use gtk::{gio, glib, prelude::*};

use crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT;

use super::{
    LocalOperationProvider, await_cancellable, copy_new_recursively, copy_recursively,
    deletion_error_message, deletion_error_summary, duplicate_candidate_name,
    extract_7z_from_reader, extract_tar, extract_zip_from_archive, home_trash_entries_at,
    is_trash_unsupported_failure, move_local_with, operation_error_summary, parse_copy_suffix,
    replace_local, replace_local_with, transfer_is_noop, validated_archive_path, validated_child,
    write_staged_archive,
};
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        ArchiveFormat, CompressRequest, DeleteRequest, LoadHandle, OperationEvent,
        OperationProvider, OperationRequestId, PasteItem, PasteRequest, RestoreRequest,
        RestoreSource, TransferConflict,
    },
};

fn file_entry(path: &std::path::Path) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        native_name: path.file_name().unwrap_or_default().to_owned(),
        display_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn directory_entry(path: &std::path::Path) -> FileEntry {
    FileEntry {
        kind: EntryKind::Directory,
        ..file_entry(path)
    }
}

fn settle_cancelled_io(context: &glib::MainContext) {
    context.block_on(glib::timeout_future(Duration::from_millis(25)));
    while context.pending() {
        context.iteration(false);
    }
}

#[test]
fn deletion_error_summaries_are_bounded_and_report_the_failure_count() {
    let errors = (1..=10)
        .map(|index| format!("item-{index}: denied"))
        .collect::<Vec<_>>();

    let summary = deletion_error_summary(&errors);

    assert!(summary.starts_with("10 items could not be deleted"));
    assert!(summary.contains("• item-1: denied"));
    assert!(summary.contains("• item-8: denied"));
    assert!(!summary.contains("• item-9: denied"));
    assert!(summary.ends_with("…and 2 more"));
    assert!(
        operation_error_summary(&errors[..1], "restored")
            .starts_with("1 item could not be restored")
    );
}

#[test]
fn a_backend_without_trash_support_gets_an_actionable_message() {
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "trash not supported");

    let trash_message = deletion_error_message("share-folder", false, &error);
    assert!(trash_message.contains("doesn't support Trash"));
    assert!(trash_message.contains("Delete permanently instead"));

    let permanent_message = deletion_error_message("share-folder", true, &error);
    assert!(!permanent_message.contains("Trash"));
    assert!(permanent_message.contains("trash not supported"));
}

#[test]
fn a_trash_attempt_that_fails_as_unsupported_is_retryable() {
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "trash not supported");
    assert!(is_trash_unsupported_failure(false, &error));
}

#[test]
fn an_already_permanent_delete_failure_is_never_retryable() {
    // Nothing left to fall back to if a *permanent* delete itself failed
    // with `NotSupported` -- retrying it the same way would just fail again.
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "trash not supported");
    assert!(!is_trash_unsupported_failure(true, &error));
}

#[test]
fn an_unrelated_trash_failure_is_not_retryable() {
    let error = glib::Error::new(gio::IOErrorEnum::PermissionDenied, "access denied");
    assert!(!is_trash_unsupported_failure(false, &error));
}

#[test]
fn other_deletion_failures_keep_the_raw_error() {
    let error = glib::Error::new(gio::IOErrorEnum::PermissionDenied, "access denied");

    let message = deletion_error_message("secret.txt", false, &error);

    assert_eq!(message, "secret.txt: access denied");
}

#[test]
fn validated_children_are_confined_to_native_and_uri_parents() {
    let native = gio::File::for_path("/fixture/parent");
    let remote = gio::File::for_uri("sftp://host.example/home/user/");

    assert!(
        validated_child(&native, "folder")
            .is_ok_and(|child| child.equal(&gio::File::for_path("/fixture/parent/folder")))
    );
    assert!(validated_child(&remote, "folder").is_ok_and(|child| {
        child.equal(&gio::File::for_uri("sftp://host.example/home/user/folder"))
    }));

    for name in ["../escaped", "nested/child", "/tmp/absolute", ".", ".."] {
        assert!(validated_child(&native, name).is_err());
        assert!(validated_child(&remote, name).is_err());
    }
}

#[test]
fn transfers_into_the_same_location_or_a_descendant_are_noops() {
    let source = gio::File::for_path("/fixture/source");
    let parent = gio::File::for_path("/fixture");
    let same_target = parent.child("source");
    let descendant = gio::File::for_path("/fixture/source/nested");
    let descendant_target = descendant.child("source");
    let unrelated = gio::File::for_path("/elsewhere");
    let unrelated_target = unrelated.child("source");

    assert!(transfer_is_noop(&source, &parent, &same_target));
    assert!(transfer_is_noop(&source, &source, &source.child("source")));
    assert!(transfer_is_noop(&source, &descendant, &descendant_target));
    assert!(!transfer_is_noop(&source, &unrelated, &unrelated_target));
}

#[test]
fn completed_gio_result_wins_a_cancellation_race() {
    let context = glib::MainContext::new();
    let cancellable = gio::Cancellable::new();
    let cancel_after_result = cancellable.clone();
    let file = gio::File::for_path("/fixture");

    let result = context.block_on(await_cancellable(
        &file,
        &cancellable,
        move |_, _, result| {
            result.resolve(Ok::<_, glib::Error>(()));
            cancel_after_result.cancel();
        },
    ));

    assert!(result.is_ok());
}

#[test]
fn recursive_copy_preserves_nested_directory_contents() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-transfer-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("top.txt"), b"top")?;
    fs::write(source.join("nested/child.txt"), b"child")?;

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    assert_eq!(fs::read(target.join("nested/child.txt"))?, b"child");

    fs::write(source.join("top.txt"), b"replacement")?;
    let overwrite = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        gio::Cancellable::new(),
        None,
    ));
    assert!(overwrite.is_ok());
    assert_eq!(fs::read(target.join("top.txt"))?, b"replacement");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn staged_file_replacement_preserves_the_destination_on_disk_full() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-replacement-failure-test-{unique}"));
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root)?;
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original")?;

    let result = glib::MainContext::default().block_on(replace_local_with(
        gio::File::for_path(source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
        Rc::new(|_, staged, _, _| {
            Box::pin(async move {
                fs::write(
                    staged
                        .path()
                        .ok_or_else(|| super::io_error("missing stage"))?,
                    b"partial",
                )
                .map_err(super::io_error)?;
                Err(glib::Error::new(
                    gio::IOErrorEnum::NoSpace,
                    "injected disk-full failure",
                ))
            })
        }),
    ));

    assert!(result.is_err());
    assert_eq!(fs::read(&target)?, b"original");
    assert_eq!(fs::read_dir(&root)?.count(), 2);
    fs::remove_dir_all(root)?;
    Ok(())
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

#[test]
fn moving_a_directory_falls_back_to_a_safe_copy_when_the_move_would_recurse()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-fallback-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("top.txt"), b"top")?;
    fs::write(source.join("nested/child.txt"), b"child")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        always_would_recurse(),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    assert_eq!(fs::read(target.join("nested/child.txt"))?, b"child");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_non_would_recurse_move_failure_is_returned_without_falling_back() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-real-failure-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("top.txt"), b"top")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|_, _, _| {
            Box::pin(async {
                Err(glib::Error::new(
                    gio::IOErrorEnum::PermissionDenied,
                    "injected permission failure",
                ))
            })
        }),
    ));

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::PermissionDenied)));
    assert_eq!(fs::read(source.join("top.txt"))?, b"top");
    assert!(!target.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_successful_move_attempt_is_used_without_falling_back_to_copy() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-success-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("top.txt"), b"top")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|source, target, _| {
            Box::pin(async move {
                fs::rename(
                    source.path().expect("native source"),
                    target.path().expect("native target"),
                )
                .map_err(super::io_error)
            })
        }),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cancelling_staging_preserves_the_destination_and_cleans_the_partial_copy()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-replacement-cancel-test-{unique}"));
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root)?;
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original")?;
    let staging = Rc::new(Cell::new(false));
    let staging_for_copy = staging.clone();
    let cancellable = gio::Cancellable::new();

    let task = glib::MainContext::default().spawn_local(replace_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        cancellable.clone(),
        None,
        Rc::new(move |_, staged, _, cancellable| {
            let staging = staging_for_copy.clone();
            Box::pin(async move {
                fs::write(
                    staged
                        .path()
                        .ok_or_else(|| super::io_error("missing stage"))?,
                    b"partial",
                )
                .map_err(super::io_error)?;
                staging.set(true);
                cancellable.future().await;
                Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "injected cancellation",
                ))
            })
        }),
    ));
    let context = glib::MainContext::default();
    while !staging.get() {
        context.iteration(true);
    }
    cancellable.cancel();
    let result = context.block_on(task)?;
    settle_cancelled_io(&context);

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert_eq!(fs::read(&target)?, b"original");
    assert_eq!(fs::read(&source)?, b"replacement");
    assert_eq!(fs::read_dir(&root)?.count(), 2);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn staged_file_replacement_commits_then_removes_a_moved_source() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-replacement-success-test-{unique}"));
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root)?;
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original")?;

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(&target)?, b"replacement");
    assert!(!source.exists());
    assert_eq!(fs::read_dir(&root)?.count(), 1);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cancelled_replacement_move_tracks_the_modified_source_and_target_roots()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-replacement-move-cancel-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("new"))?;
    fs::create_dir_all(target.join("old"))?;
    fs::write(source.join("new/item.txt"), b"replacement")?;
    for index in 0..16 {
        fs::write(target.join(format!("old/item-{index}.txt")), b"old")?;
    }

    let cancellable = gio::Cancellable::new();
    let cancel_after_commit = cancellable.clone();
    let committed_marker = target.join("new/item.txt");
    let context = glib::MainContext::default();
    let watcher = context.spawn_local(async move {
        while !committed_marker.exists() {
            glib::timeout_future(Duration::ZERO).await;
        }
        cancel_after_commit.cancel();
    });
    let mut affected_locations = HashSet::new();
    let result = context.block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        cancellable,
        Some(&mut affected_locations),
    ));
    context.block_on(watcher)?;
    settle_cancelled_io(&context);

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert!(affected_locations.contains(&Location::local(&source)));
    assert!(affected_locations.contains(&Location::local(&target)));
    assert_eq!(fs::read(target.join("new/item.txt"))?, b"replacement");
    assert!(source.exists());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn each_transfer_item_keeps_its_own_conflict_decision() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-conflict-decisions-test-{unique}"));
    let sources = root.join("sources");
    let destination = root.join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    fs::write(sources.join("replace.txt"), b"new replacement")?;
    fs::write(sources.join("late.txt"), b"new late item")?;
    fs::write(destination.join("replace.txt"), b"old replacement")?;
    fs::write(destination.join("late.txt"), b"late arrival")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(1),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(sources.join("replace.txt")),
                    conflict: TransferConflict::ReplaceExisting,
                },
                PasteItem {
                    source: Location::local(sources.join("late.txt")),
                    conflict: TransferConflict::FailIfExists,
                },
            ],
            move_sources: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. }
                | OperationEvent::Cancelled { .. }
                | OperationEvent::TransferFailed { .. }
                | OperationEvent::Failed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::TransferFailed {
            completed_locations,
            ..
        }) if completed_locations == &[Location::local(sources.join("replace.txt"))]
    ));
    assert_eq!(
        fs::read(destination.join("replace.txt"))?,
        b"new replacement"
    );
    assert_eq!(fs::read(destination.join("late.txt"))?, b"late arrival");
    assert!(!sources.join("replace.txt").exists());
    assert!(sources.join("late.txt").exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn staged_directory_replacement_does_not_merge_old_contents() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-directory-replacement-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("new"))?;
    fs::create_dir_all(target.join("old"))?;
    fs::write(source.join("new/item.txt"), b"new")?;
    fs::write(target.join("old/item.txt"), b"old")?;

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("new/item.txt"))?, b"new");
    assert!(!target.join("old").exists());
    assert!(source.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

fn test_file_entry(path: &Path) -> FileEntry {
    let name = path.file_name().unwrap_or_default().to_os_string();
    FileEntry {
        location: Location::local(path),
        native_name: name.clone(),
        display_name: name.to_string_lossy().into_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn run_compression(request: CompressRequest) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.compress(
        request,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Compressed { .. } | OperationEvent::Failed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }
    drop(operation);
    events.borrow().clone()
}

fn compression_stages(destination: &Path) -> Result<Vec<OsString>, Box<dyn Error>> {
    Ok(fs::read_dir(destination)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().starts_with(".strata-compression-"))
        .collect())
}

#[test]
fn compression_provider_rejects_escaping_archive_names() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let source = root.path().join("source.txt");
    fs::create_dir(&destination)?;
    fs::write(&source, b"source")?;

    let events = run_compression(CompressRequest {
        id: OperationRequestId(1),
        entries: vec![test_file_entry(&source)],
        destination: Location::local(&destination),
        archive_name: "../outside".to_owned(),
        conflict: TransferConflict::ReplaceExisting,
        format: ArchiveFormat::Zip,
        password: None,
    });

    assert!(matches!(events.as_slice(), [OperationEvent::Failed { .. }]));
    assert!(!root.path().join("outside.zip").exists());
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn compression_conflict_choices_preserve_or_replace_the_destination() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let source = root.path().join("source.txt");
    let archive = destination.join("existing.zip");
    fs::create_dir(&destination)?;
    fs::write(&source, b"replacement")?;
    fs::write(&archive, b"original")?;
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o640))?;
    let request = |conflict| CompressRequest {
        id: OperationRequestId(1),
        entries: vec![test_file_entry(&source)],
        destination: Location::local(&destination),
        archive_name: "existing".to_owned(),
        conflict,
        format: ArchiveFormat::Zip,
        password: None,
    };

    let refused = run_compression(request(TransferConflict::FailIfExists));
    assert!(
        refused
            .iter()
            .any(|event| matches!(event, OperationEvent::Failed { .. }))
    );
    assert_eq!(fs::read(&archive)?, b"original");
    assert_eq!(fs::metadata(&archive)?.permissions().mode() & 0o777, 0o640);

    let replaced = run_compression(request(TransferConflict::ReplaceExisting));
    assert!(
        replaced
            .iter()
            .any(|event| matches!(event, OperationEvent::Compressed { .. }))
    );
    let extracted = destination.join("extracted");
    fs::create_dir(&extracted)?;
    assert_eq!(
        extract_zip(&archive, &extracted)?,
        Some("source.txt".to_owned())
    );
    assert_eq!(fs::metadata(&archive)?.permissions().mode() & 0o777, 0o640);
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn compression_failure_preserves_an_existing_archive() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let missing = root.path().join("missing.txt");
    let archive = destination.join("existing.zip");
    fs::create_dir(&destination)?;
    fs::write(&archive, b"original")?;

    let events = run_compression(CompressRequest {
        id: OperationRequestId(1),
        entries: vec![test_file_entry(&missing)],
        destination: Location::local(&destination),
        archive_name: "existing".to_owned(),
        conflict: TransferConflict::ReplaceExisting,
        format: ArchiveFormat::Zip,
        password: None,
    });

    assert!(
        events
            .iter()
            .any(|event| matches!(event, OperationEvent::Failed { .. }))
    );
    assert_eq!(fs::read(&archive)?, b"original");
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn every_compression_format_commits_a_readable_archive() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let source = root.path().join("source.txt");
    let mode_reference = root.path().join("mode-reference");
    fs::create_dir(&destination)?;
    fs::write(&source, b"contents")?;
    fs::File::create(&mode_reference)?;
    let expected_mode = fs::metadata(&mode_reference)?.permissions().mode() & 0o777;

    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::TarGz,
        ArchiveFormat::Tar,
    ] {
        let base = format!("archive-{}", format.extension().replace('.', "-"));
        let events = run_compression(CompressRequest {
            id: OperationRequestId(1),
            entries: vec![test_file_entry(&source)],
            destination: Location::local(&destination),
            archive_name: base.clone(),
            conflict: TransferConflict::FailIfExists,
            format,
            password: None,
        });
        assert!(
            events
                .iter()
                .any(|event| matches!(event, OperationEvent::Compressed { .. }))
        );
        let archive = destination.join(format!("{base}.{}", format.extension()));
        let extracted = destination.join(format!("extracted-{base}"));
        fs::create_dir(&extracted)?;
        match format {
            ArchiveFormat::Zip => {
                extract_zip(&archive, &extracted)?;
            }
            ArchiveFormat::SevenZ => {
                extract_7z_from_reader(
                    fs::File::open(&archive)?,
                    &extracted,
                    sevenz_rust2::Password::empty(),
                    &Arc::new(AtomicUsize::new(0)),
                )?;
            }
            ArchiveFormat::TarGz => {
                extract_tar(&archive, &extracted, true, &Arc::new(AtomicUsize::new(0)))?;
            }
            ArchiveFormat::Tar => {
                extract_tar(&archive, &extracted, false, &Arc::new(AtomicUsize::new(0)))?;
            }
        }
        assert_eq!(fs::read(extracted.join("source.txt"))?, b"contents");
        assert_eq!(
            fs::metadata(&archive)?.permissions().mode() & 0o777,
            expected_mode
        );
    }
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn cancelling_staged_compression_unlinks_the_partial_output() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let archive = destination.join("existing.zip");
    fs::write(&archive, b"original")?;
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let worker_release = release.clone();
    let worker_finished = finished.clone();
    let worker_destination = destination.clone();
    let worker_archive = archive.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        write_staged_archive(
            &worker_destination,
            &worker_archive,
            TransferConflict::ReplaceExisting,
            move |mut file| {
                file.write_all(b"partial")
                    .map_err(|error| error.to_string())?;
                worker_started.store(true, Ordering::Release);
                while !worker_release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                worker_finished.store(true, Ordering::Release);
                Ok(())
            },
        )
        .await
    });
    let context = glib::MainContext::default();
    while !started.load(Ordering::Acquire) {
        context.iteration(false);
        std::thread::yield_now();
    }
    assert_eq!(compression_stages(&destination)?.len(), 1);

    task.abort();
    drop(task);
    while context.pending() {
        context.iteration(false);
    }
    let stage_was_removed = compression_stages(&destination)?.is_empty();
    let destination_was_preserved = fs::read(&archive)? == b"original";
    release.store(true, Ordering::Release);
    while !finished.load(Ordering::Acquire) {
        std::thread::yield_now();
    }

    assert!(stage_was_removed);
    assert!(destination_was_preserved);
    Ok(())
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) -> Result<(), Box<dyn Error>> {
    let mut writer = zip::ZipWriter::new(fs::File::create(path)?);
    for (name, contents) in entries {
        writer.start_file(*name, zip::write::SimpleFileOptions::default())?;
        writer.write_all(contents)?;
    }
    writer.finish()?;
    Ok(())
}

fn append_raw_tar_entry<W: Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    contents: &[u8],
) -> Result<(), Box<dyn Error>> {
    let mut header = tar::Header::new_gnu();
    header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
    header.set_mode(0o644);
    header.set_size(contents.len() as u64);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    builder.append(&header, contents)?;
    Ok(())
}

fn write_tar(path: &Path, name: &str, contents: &[u8], gzip: bool) -> Result<(), Box<dyn Error>> {
    let file = fs::File::create(path)?;
    if gzip {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        ));
        append_raw_tar_entry(&mut builder, name, contents)?;
        builder.into_inner()?.finish()?;
    } else {
        let mut builder = tar::Builder::new(file);
        append_raw_tar_entry(&mut builder, name, contents)?;
        builder.finish()?;
    }
    Ok(())
}

fn write_7z(path: &Path, name: &str, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut writer = sevenz_rust2::ArchiveWriter::create(path)?;
    writer.push_archive_entry(
        sevenz_rust2::ArchiveEntry::new_file(name),
        Some(Cursor::new(contents)),
    )?;
    writer.finish()?;
    Ok(())
}

fn extract_zip(path: &Path, destination: &Path) -> Result<Option<String>, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    extract_zip_from_archive(
        &mut archive,
        destination,
        None,
        &Arc::new(AtomicUsize::new(0)),
    )
}

#[test]
fn archive_paths_must_be_nonempty_confined_relative_paths() -> Result<(), Box<dyn Error>> {
    for path in [
        "",
        ".",
        "../marker",
        "safe/../marker",
        "/tmp/marker",
        "\\tmp\\marker",
        "C:\\tmp\\marker",
        "C:marker",
        "safe/C:/marker",
        "\\\\server\\share\\marker",
        "//server/share/marker",
    ] {
        assert!(validated_archive_path(path).is_err(), "accepted {path:?}");
    }
    assert_eq!(
        validated_archive_path("folder/./nested//item.txt")?,
        Path::new("folder/nested/item.txt")
    );
    Ok(())
}

#[test]
fn cancelling_between_deletions_reports_completed_and_unattempted_items()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-delete-cancel-test-{unique}"));
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    fs::create_dir_all(&root)?;
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let operation = Rc::new(RefCell::new(None::<LoadHandle>));
    let emitted = events.clone();
    let operation_for_emit = operation.clone();
    let handle = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(7),
            entries: vec![file_entry(&first), file_entry(&second)],
            permanent: true,
        },
        Rc::new(move |event| {
            let cancel = matches!(event, OperationEvent::DeleteProgress { completed: 1, .. });
            emitted.borrow_mut().push(event);
            if cancel {
                operation_for_emit.borrow_mut().take();
            }
        }),
    );
    operation.replace(Some(handle));
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Cancelled { .. }))
    {
        glib::MainContext::default().iteration(true);
    }

    let result = events
        .borrow()
        .iter()
        .find_map(|event| match event {
            OperationEvent::Cancelled { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("terminal cancellation result");
    assert_eq!(result.completed, [Location::local(&first)]);
    assert!(result.failed.is_empty());
    assert_eq!(result.not_attempted, [Location::local(&second)]);
    assert!(!first.exists());
    assert!(second.exists());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn every_archive_format_rejects_parent_traversal() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let zip_path = root.path().join("malicious.zip");
    let tar_path = root.path().join("malicious.tar");
    let tar_gz_path = root.path().join("malicious.tar.gz");
    let seven_z_path = root.path().join("malicious.7z");
    write_zip(&zip_path, &[("../zip-marker", b"escaped")])?;
    write_tar(&tar_path, "../tar-marker", b"escaped", false)?;
    write_tar(&tar_gz_path, "../tar-gz-marker", b"escaped", true)?;
    write_7z(&seven_z_path, "../seven-z-marker", b"escaped")?;

    assert!(extract_zip(&zip_path, &destination).is_err());
    assert!(
        extract_tar(
            &tar_path,
            &destination,
            false,
            &Arc::new(AtomicUsize::new(0)),
        )
        .is_err()
    );
    assert!(
        extract_tar(
            &tar_gz_path,
            &destination,
            true,
            &Arc::new(AtomicUsize::new(0)),
        )
        .is_err()
    );
    assert!(
        extract_7z_from_reader(
            fs::File::open(&seven_z_path)?,
            &destination,
            sevenz_rust2::Password::empty(),
            &Arc::new(AtomicUsize::new(0)),
        )
        .is_err()
    );

    for marker in [
        "zip-marker",
        "tar-marker",
        "tar-gz-marker",
        "seven-z-marker",
    ] {
        assert!(!root.path().join(marker).exists(), "created {marker}");
    }
    Ok(())
}

#[test]
fn cancelling_recursive_copy_removes_only_its_staging_output() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-copy-cancel-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("nested/item.txt"), b"contents")?;
    fs::write(root.join("pre-existing.txt"), b"keep")?;

    let cancellable = gio::Cancellable::new();
    let task = glib::MainContext::default().spawn_local(copy_new_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        cancellable.clone(),
    ));
    let context = glib::MainContext::default();
    loop {
        context.iteration(true);
        if fs::read_dir(&root)?.any(|entry| {
            entry.is_ok_and(|entry| entry.file_name().to_string_lossy().starts_with(".strata-"))
        }) {
            break;
        }
    }
    cancellable.cancel();
    let result = context.block_on(task)?;
    settle_cancelled_io(&context);

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert!(!target.exists());
    assert_eq!(fs::read(root.join("pre-existing.txt"))?, b"keep");
    assert_eq!(fs::read_dir(&root)?.count(), 2);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cancelling_recursive_delete_leaves_the_unfinished_root_in_place() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-recursive-delete-cancel-test-{unique}"));
    let nested = root.join("nested");
    fs::create_dir_all(&nested)?;
    for index in 0..4 {
        fs::write(nested.join(format!("item-{index}.txt")), b"contents")?;
    }

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(10),
            entries: vec![directory_entry(&root)],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while fs::read_dir(&nested)?.count() == 4 {
        context.iteration(true);
    }
    drop(operation);
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Cancelled { .. }))
    {
        context.iteration(true);
    }
    settle_cancelled_io(&context);

    let result = events
        .borrow()
        .iter()
        .find_map(|event| match event {
            OperationEvent::Cancelled { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("terminal cancellation result");
    assert!(result.failed == [Location::local(&root)]);
    assert!(result.affected_locations.contains(&Location::local(&root)));
    assert!(
        !result
            .affected_locations
            .contains(&Location::local(&nested))
    );
    assert!(root.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cancelling_between_moves_reports_completed_and_unattempted_sources() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-cancel-test-{unique}"));
    let sources = root.join("sources");
    let destination = root.join("destination");
    let first = sources.join("first.txt");
    let second = sources.join("second.txt");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let operation = Rc::new(RefCell::new(None::<LoadHandle>));
    let emitted = events.clone();
    let operation_for_emit = operation.clone();
    let handle = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(8),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(&first),
                    conflict: TransferConflict::FailIfExists,
                },
                PasteItem {
                    source: Location::local(&second),
                    conflict: TransferConflict::FailIfExists,
                },
            ],
            move_sources: true,
        },
        Rc::new(move |event| {
            let cancel = matches!(event, OperationEvent::TransferProgress { completed: 1, .. });
            emitted.borrow_mut().push(event);
            if cancel {
                operation_for_emit.borrow_mut().take();
            }
        }),
    );
    operation.replace(Some(handle));
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Cancelled { .. }))
    {
        glib::MainContext::default().iteration(true);
    }

    let result = events
        .borrow()
        .iter()
        .find_map(|event| match event {
            OperationEvent::Cancelled { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("terminal cancellation result");
    assert_eq!(result.completed, [Location::local(&first)]);
    assert!(result.failed.is_empty());
    assert_eq!(result.not_attempted, [Location::local(&second)]);
    assert!(destination.join("first.txt").exists());
    assert!(!first.exists());
    assert!(second.exists());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn extraction_rejects_final_and_intermediate_symlinks() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let external = root.path().join("external");
    fs::create_dir(&destination)?;
    fs::create_dir(&external)?;
    std::os::unix::fs::symlink(root.path().join("missing"), destination.join("dangling"))?;
    std::os::unix::fs::symlink(&external, destination.join("redirect"))?;
    let final_archive = root.path().join("final.zip");
    let intermediate_archive = root.path().join("intermediate.zip");
    write_zip(&final_archive, &[("dangling", b"escaped")])?;
    write_zip(&intermediate_archive, &[("redirect/marker", b"escaped")])?;

    assert!(extract_zip(&final_archive, &destination).is_err());
    assert!(extract_zip(&intermediate_archive, &destination).is_err());
    assert!(!root.path().join("missing").exists());
    assert!(!external.join("marker").exists());
    Ok(())
}

#[test]
fn extraction_supports_nesting_and_regular_conflicts() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    fs::write(destination.join("report.txt"), b"original")?;
    fs::create_dir(destination.join("existing"))?;
    fs::write(destination.join("existing/old.txt"), b"old")?;
    let archive_path = root.path().join("content.zip");
    write_zip(
        &archive_path,
        &[
            ("folder/nested/item.txt", b"nested"),
            ("report.txt", b"replacement"),
            ("existing/new.txt", b"new"),
        ],
    )?;

    assert_eq!(
        extract_zip(&archive_path, &destination)?.as_deref(),
        Some("folder")
    );
    assert_eq!(
        fs::read(destination.join("folder/nested/item.txt"))?,
        b"nested"
    );
    assert_eq!(fs::read(destination.join("report.txt"))?, b"original");
    assert_eq!(
        fs::read(destination.join("report (2).txt"))?,
        b"replacement"
    );
    assert_eq!(fs::read(destination.join("existing/old.txt"))?, b"old");
    assert_eq!(fs::read(destination.join("existing (2)/new.txt"))?, b"new");
    Ok(())
}

#[test]
fn home_trash_fallback_finds_broken_symlinks_the_virtual_backend_has_not_refreshed()
-> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let fixture = std::env::temp_dir().join(format!("strata-home-trash-fallback-{unique}"));
    let trash = fixture.join("Trash");
    let original = fixture.join("original report.txt");
    fs::create_dir_all(trash.join("files"))?;
    fs::create_dir_all(trash.join("info"))?;
    std::os::unix::fs::symlink("missing-target", trash.join("files/report.txt"))?;
    let encoded = original.display().to_string().replace(' ', "%20");
    fs::write(
        trash.join("info/report.txt.trashinfo"),
        format!("[Trash Info]\nPath={encoded}\nDeletionDate=2026-09-03T16:05:39\n"),
    )?;

    let entries = home_trash_entries_at(&trash, &HashSet::from([original.clone()]));

    let entry = entries.get(&original).expect("fallback entry");
    assert_eq!(
        entry.source,
        Location::local(trash.join("files/report.txt"))
    );
    assert_eq!(entry.original_target, Some(Location::local(&original)));
    assert_eq!(
        entry.trash_info.as_deref(),
        Some(trash.join("info/report.txt.trashinfo").as_path())
    );
    fs::remove_dir_all(fixture)?;
    Ok(())
}

#[test]
fn cancelling_restore_before_io_reports_every_item_as_unattempted() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let entries = vec![file_entry(std::path::Path::new("/fixture/trashed.txt"))];
    let operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(9),
            source: RestoreSource::TrashEntries(entries.clone()),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drop(operation);
    while events.borrow().is_empty() {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().as_slice(),
        [OperationEvent::Cancelled { result, .. }]
            if result.completed.is_empty()
                && result.failed.is_empty()
                && result.not_attempted == [entries[0].location.clone()]
    ));
    Ok(())
}

#[test]
fn copy_suffix_parsing_and_candidate_naming() {
    assert_eq!(
        parse_copy_suffix(OsStr::new("name")),
        (OsStr::new("name"), None)
    );
    assert_eq!(
        parse_copy_suffix(OsStr::new("name (1)")),
        (OsStr::new("name"), Some(1))
    );
    assert_eq!(
        parse_copy_suffix(OsStr::new("name (2)")),
        (OsStr::new("name"), Some(2))
    );
    assert_eq!(
        parse_copy_suffix(OsStr::new("name (42)")),
        (OsStr::new("name"), Some(42))
    );
    assert_eq!(
        parse_copy_suffix(OsStr::new("name (foo)")),
        (OsStr::new("name (foo)"), None)
    );
    assert_eq!(
        parse_copy_suffix(OsStr::new("name (0)")),
        (OsStr::new("name (0)"), None)
    );
    assert_eq!(
        parse_copy_suffix(OsStr::new("name (2")),
        (OsStr::new("name (2"), None)
    );
    assert_eq!(
        parse_copy_suffix(OsStr::new("name (18446744073709551615)")),
        (OsStr::new("name (18446744073709551615)"), None)
    );

    assert_eq!(
        duplicate_candidate_name(OsStr::new("name"), Some(OsStr::new("ext")), 1),
        OsString::from("name (1).ext")
    );
    assert_eq!(
        duplicate_candidate_name(OsStr::new("name"), Some(OsStr::new("ext")), 2),
        OsString::from("name (2).ext")
    );
    assert_eq!(
        duplicate_candidate_name(OsStr::new("name"), None, 1),
        OsString::from("name (1)")
    );
    assert_eq!(
        duplicate_candidate_name(OsStr::new("name"), None, 2),
        OsString::from("name (2)")
    );
}

#[test]
fn duplicating_a_file_generates_numbered_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("photo.jpg");
    fs::write(&source, b"original-content")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(10),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"original-content");
    let duplicate = destination.join("photo (1).jpg");
    assert!(duplicate.exists());
    assert_eq!(fs::read(&duplicate)?, b"original-content");
    Ok(())
}

#[test]
fn duplicating_a_file_preserves_non_utf8_name_bytes() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source_name = OsString::from_vec(b"photo-\xff.jpg".to_vec());
    let source = destination.join(&source_name);
    fs::write(&source, b"original-content")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(15),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    let duplicate_name = OsString::from_vec(b"photo-\xff (1).jpg".to_vec());
    let duplicate = destination.join(duplicate_name);
    assert_eq!(fs::read(duplicate)?, b"original-content");
    Ok(())
}

#[test]
fn duplicating_an_existing_numbered_name_advances_its_index() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("photo (1).jpg");
    fs::write(&source, b"copy-content")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(11),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"copy-content");
    let duplicate = destination.join("photo (2).jpg");
    assert!(duplicate.exists());
    assert_eq!(fs::read(&duplicate)?, b"copy-content");
    Ok(())
}

#[test]
fn duplicating_file_with_existing_numbered_name_advances_to_next_index()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("photo.jpg");
    fs::write(&source, b"original")?;
    fs::write(destination.join("photo (1).jpg"), b"first copy")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(12),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert_eq!(fs::read(destination.join("photo (2).jpg"))?, b"original");
    assert_eq!(fs::read(destination.join("photo (1).jpg"))?, b"first copy");
    Ok(())
}

#[test]
fn duplicating_a_directory_generates_numbered_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("documents");
    fs::create_dir_all(&source)?;
    fs::write(source.join("notes.txt"), b"nested-file")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(13),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(source.is_dir());
    let duplicate = destination.join("documents (1)");
    assert!(duplicate.is_dir());
    assert_eq!(fs::read(duplicate.join("notes.txt"))?, b"nested-file");
    Ok(())
}

#[test]
fn cutting_in_the_same_folder_remains_a_noop() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let file = destination.join("document.txt");
    let directory = destination.join("folder");
    fs::write(&file, b"content")?;
    fs::create_dir_all(&directory)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(14),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(&file),
                    conflict: TransferConflict::FailIfExists,
                },
                PasteItem {
                    source: Location::local(&directory),
                    conflict: TransferConflict::FailIfExists,
                },
            ],
            move_sources: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(file.exists());
    assert!(directory.is_dir());
    assert!(!destination.join("document (1).txt").exists());
    assert!(!destination.join("folder (1)").exists());
    Ok(())
}

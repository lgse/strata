// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
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

mod restore_safety;

use super::{
    LocalOperationProvider, TransferProgressTracker, await_cancellable, copy_failure_after_cleanup,
    copy_new_recursively, copy_new_remote_file_with, copy_recursively, deletion_error_message,
    deletion_error_summary, duplicate_candidate_name, home_trash_entries_at, io_error,
    is_trash_unsupported_failure, move_local, move_local_with, operation_error_summary,
    parse_copy_suffix, replace_local, replace_local_with, transfer_is_noop, validated_child,
};
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        DeleteRequest, LoadHandle, MoveRecord, OperationEvent, OperationProvider,
        OperationRequestId, PasteItem, PasteRequest, RestoreRequest, RestoreSource,
        RestoreTrashItem, TransferConflict, UndoMoveItem, UndoMoveRequest,
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
        is_hidden: false,
        mode: MetadataValue::Unknown,
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
fn rotational_deletes_cap_parallelism_without_disabling_it() {
    assert_eq!(super::bounded_local_delete_worker_count(0, false), 1);
    assert_eq!(super::bounded_local_delete_worker_count(1, true), 1);
    assert_eq!(super::bounded_local_delete_worker_count(8, true), 1);
    assert_eq!(super::bounded_local_delete_worker_count(8, false), 2);
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
fn copy_recursively_does_not_follow_a_symlink_nested_inside_the_tree() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-copy-symlink-test-{unique}"));
    let source = root.join("source");
    let outside = root.join("outside");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("secret.txt"), b"do not copy me")?;
    fs::write(source.join("nested/visible.txt"), b"contents")?;
    std::os::unix::fs::symlink(&outside, source.join("nested/decoy"))?;

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok());
    assert_eq!(fs::read(target.join("nested/visible.txt"))?, b"contents");
    let decoy_dest = target.join("nested/decoy");
    let decoy_metadata = fs::symlink_metadata(&decoy_dest)?;
    assert!(
        decoy_metadata.file_type().is_symlink(),
        "the decoy must be copied as a symlink, not followed into a real directory"
    );
    assert_eq!(fs::read_link(&decoy_dest)?, outside);
    // `is_symlink` above already rules out a real directory of copied
    // content existing under this name; confirm the thing it still points
    // at (unavoidably reachable by following the recreated symlink, same as
    // the original) was left untouched rather than overwritten.
    assert_eq!(fs::read(outside.join("secret.txt"))?, b"do not copy me");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn copy_recursively_of_a_symlink_creates_a_symlink_not_a_recursive_copy()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-copy-symlink-top-test-{unique}"));
    let outside = root.join("outside");
    let decoy = root.join("decoy");
    let target = root.join("target-link");
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("secret.txt"), b"do not copy me")?;
    std::os::unix::fs::symlink(&outside, &decoy)?;

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&decoy),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok());
    let target_metadata = fs::symlink_metadata(&target)?;
    assert!(
        target_metadata.file_type().is_symlink(),
        "copying a symlink must produce a symlink, not a recursive copy of its target"
    );
    assert_eq!(fs::read_link(&target)?, outside);
    assert_eq!(fs::read(outside.join("secret.txt"))?, b"do not copy me");

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
fn a_plain_move_relocates_the_entry_via_the_hardened_rename_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"payload")?;

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target)?, b"payload");
    Ok(())
}

#[test]
fn moving_a_directory_into_its_own_child_fails_instead_of_deleting_it() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("top.txt"), b"top")?;
    let target = source.join("nested").join("moved-source");

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_err());
    assert_eq!(fs::read(source.join("top.txt"))?, b"top");
    Ok(())
}

#[test]
fn move_accepts_a_symlink_in_the_sources_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual_parent = root.path().join("actual");
    let linked_parent = root.path().join("linked");
    fs::create_dir(&actual_parent)?;
    fs::write(actual_parent.join("source.txt"), b"keep")?;
    std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;
    let target = root.path().join("target.txt");

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(linked_parent.join("source.txt")),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert!(!actual_parent.join("source.txt").exists());
    assert_eq!(fs::read(target)?, b"keep");
    Ok(())
}

#[test]
fn move_accepts_a_symlink_in_the_destinations_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let actual_destination = root.path().join("actual");
    let linked_destination = root.path().join("linked");
    fs::create_dir(&actual_destination)?;
    fs::write(&source, b"keep")?;
    std::os::unix::fs::symlink(&actual_destination, &linked_destination)?;

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(linked_destination.join("target.txt")),
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert!(!source.exists());
    assert_eq!(fs::read(actual_destination.join("target.txt"))?, b"keep");
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
fn replacing_a_symlink_preserves_link_semantics() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source-link");
    let target = root.path().join("target-link");
    std::os::unix::fs::symlink("new-target", &source)?;
    std::os::unix::fs::symlink("old-target", &target)?;

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read_link(&target)?, Path::new("new-target"));
    assert_eq!(fs::read_link(&source)?, Path::new("new-target"));
    Ok(())
}

#[test]
fn replacement_move_does_not_delete_a_substituted_source() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let original_source = root.path().join("original-source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original target")?;

    let replaced_source = source.clone();
    let new_source = source.clone();
    let preserved_source = original_source.clone();
    let result = glib::MainContext::default().block_on(replace_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        gio::Cancellable::new(),
        None,
        Rc::new(move |_, staged, _, _| {
            let replaced_source = replaced_source.clone();
            let new_source = new_source.clone();
            let preserved_source = preserved_source.clone();
            Box::pin(async move {
                fs::write(staged.path().unwrap_or_default(), b"replacement").map_err(io_error)?;
                fs::rename(replaced_source, preserved_source).map_err(io_error)?;
                fs::write(new_source, b"new arrival").map_err(io_error)
            })
        }),
    ));

    let error = result.expect_err("a substituted source must fail identity validation");
    assert!(error.to_string().contains("changed"), "{error}");
    assert_eq!(fs::read(target)?, b"replacement");
    assert_eq!(fs::read(source)?, b"new arrival");
    assert_eq!(fs::read(original_source)?, b"replacement");
    Ok(())
}

#[test]
fn replace_accepts_a_symlink_in_the_sources_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual_parent = root.path().join("actual");
    let linked_parent = root.path().join("linked");
    fs::create_dir(&actual_parent)?;
    fs::write(actual_parent.join("source.txt"), b"new")?;
    std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;
    let target = root.path().join("target.txt");
    fs::write(&target, b"old")?;

    let mut affected_locations = HashSet::new();
    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(linked_parent.join("source.txt")),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        Some(&mut affected_locations),
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(&target)?, b"new");
    assert_eq!(fs::read(actual_parent.join("source.txt"))?, b"new");
    Ok(())
}

#[test]
fn copy_accepts_a_symlink_higher_in_the_sources_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual_root = root.path().join("actual");
    let linked_root = root.path().join("linked");
    fs::create_dir_all(actual_root.join("subdir"))?;
    fs::write(actual_root.join("subdir/source.txt"), b"keep")?;
    std::os::unix::fs::symlink(&actual_root, &linked_root)?;
    let target = root.path().join("target.txt");

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(linked_root.join("subdir/source.txt")),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target)?, b"keep");
    assert_eq!(fs::read(actual_root.join("subdir/source.txt"))?, b"keep");
    Ok(())
}

#[test]
fn replacement_stops_before_exchanging_a_substituted_target() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    let original_target = root.path().join("original-target.txt");
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original target")?;
    let staged_path = Rc::new(RefCell::new(None));
    let recorded_staged_path = staged_path.clone();
    let replaced_target = target.clone();
    let new_target = target.clone();
    let preserved_target = original_target.clone();

    let result = glib::MainContext::default().block_on(replace_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
        Rc::new(move |_, staged, _, _| {
            let staged = staged.path().unwrap_or_default();
            *recorded_staged_path.borrow_mut() = Some(staged.clone());
            let replaced_target = replaced_target.clone();
            let new_target = new_target.clone();
            let preserved_target = preserved_target.clone();
            Box::pin(async move {
                fs::write(&staged, b"replacement").map_err(io_error)?;
                fs::rename(replaced_target, preserved_target).map_err(io_error)?;
                fs::write(new_target, b"new arrival").map_err(io_error)
            })
        }),
    ));

    let error = result.expect_err("a substituted target must fail identity validation");
    assert!(error.to_string().contains("changed"), "{error}");
    assert_eq!(fs::read(target)?, b"new arrival");
    assert_eq!(fs::read(original_target)?, b"original target");
    let staged_path = staged_path
        .borrow()
        .clone()
        .ok_or("the staging path was not recorded")?;
    assert!(!staged_path.exists());
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

#[test]
fn replacing_a_directory_cleans_up_a_symlink_in_the_old_contents_without_following_it()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-replace-symlink-cleanup-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    let outside = root.join("outside");
    fs::create_dir_all(&source)?;
    fs::write(source.join("item.txt"), b"new")?;
    fs::create_dir_all(&target)?;
    fs::write(target.join("keep.txt"), b"old")?;
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("secret.txt"), b"do not delete me")?;
    std::os::unix::fs::symlink(&outside, target.join("decoy"))?;

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("item.txt"))?, b"new");
    assert!(
        !target.join("keep.txt").exists(),
        "the replaced directory's old contents must be gone"
    );
    assert!(
        !target.join("decoy").exists() && !target.join("decoy").is_symlink(),
        "the old decoy symlink itself must be gone from the replaced directory"
    );
    assert_eq!(
        fs::read(outside.join("secret.txt"))?,
        b"do not delete me",
        "cleaning up the old target must never follow a symlink it contained"
    );

    fs::remove_dir_all(root)?;
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
fn cancelling_staged_remote_file_copy_removes_only_the_incomplete_stage()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    fs::write(&source, b"source contents")?;

    let result = glib::MainContext::default().block_on(copy_new_remote_file_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|_, stage, _| {
            Box::pin(async move {
                fs::write(stage.path().expect("native stage"), b"partial")
                    .map_err(super::io_error)?;
                Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "injected cancellation",
                ))
            })
        }),
        Rc::new(|_, _, _| Box::pin(async { panic!("cancelled copy must not commit") })),
    ));

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert!(!target.exists());
    assert_eq!(fs::read(&source)?, b"source contents");
    assert_eq!(fs::read_dir(root.path())?.count(), 1);
    Ok(())
}

#[test]
fn staged_remote_file_copy_preserves_a_racing_destination() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    fs::write(&source, b"source contents")?;

    let result = glib::MainContext::default().block_on(copy_new_remote_file_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|source, stage, _| {
            Box::pin(async move {
                fs::copy(
                    source.path().expect("native source"),
                    stage.path().expect("native stage"),
                )
                .map(|_| ())
                .map_err(super::io_error)
            })
        }),
        Rc::new(|_, target, _| {
            Box::pin(async move {
                fs::write(target.path().expect("native target"), b"racing contents")
                    .map_err(super::io_error)?;
                Err(glib::Error::new(
                    gio::IOErrorEnum::Exists,
                    "injected destination race",
                ))
            })
        }),
    ));

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Exists)));
    assert_eq!(fs::read(&target)?, b"racing contents");
    assert_eq!(fs::read(&source)?, b"source contents");
    assert_eq!(fs::read_dir(root.path())?.count(), 2);
    Ok(())
}

#[test]
fn failed_incomplete_copy_cleanup_is_reported_as_a_failure() {
    let error = copy_failure_after_cleanup(
        glib::Error::new(gio::IOErrorEnum::Cancelled, "injected cancellation"),
        Err(glib::Error::new(
            gio::IOErrorEnum::PermissionDenied,
            "injected cleanup failure",
        )),
    );

    assert!(!error.matches(gio::IOErrorEnum::Cancelled));
    assert!(
        error
            .to_string()
            .contains("incomplete copy could not be removed")
    );
    assert!(error.to_string().contains("injected cleanup failure"));
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
fn permanent_delete_removes_a_symlink_standing_in_for_a_directory_without_following_it()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let outside = std::env::temp_dir().join(format!("strata-delete-symlink-outside-{unique}"));
    let decoy = std::env::temp_dir().join(format!("strata-delete-symlink-decoy-{unique}"));
    fs::create_dir_all(&outside)?;
    let sentinel = outside.join("sentinel.txt");
    fs::write(&sentinel, b"do not delete me")?;
    // `directory_entry` reports `kind: Directory` even though the entry is
    // actually a symlink on disk, standing in for a `FileEntry` whose type
    // went stale because the real directory was swapped for a symlink.
    std::os::unix::fs::symlink(&outside, &decoy)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(20),
            entries: vec![directory_entry(&decoy)],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Deleted { .. }))
    {
        context.iteration(true);
    }

    assert!(
        sentinel.exists(),
        "deleting the symlink must never touch what it points to"
    );
    assert_eq!(fs::read_dir(&outside)?.count(), 1);
    assert!(!decoy.exists() && !decoy.is_symlink());

    fs::remove_dir_all(outside)?;
    Ok(())
}

#[test]
fn permanent_delete_does_not_follow_a_symlink_nested_inside_the_tree() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-delete-nested-symlink-test-{unique}"));
    let outside = std::env::temp_dir().join(format!("strata-delete-nested-outside-{unique}"));
    let nested = root.join("nested");
    fs::create_dir_all(&nested)?;
    fs::create_dir_all(&outside)?;
    let sentinel = outside.join("sentinel.txt");
    fs::write(&sentinel, b"do not delete me")?;
    fs::write(nested.join("visible.txt"), b"contents")?;
    std::os::unix::fs::symlink(&outside, nested.join("decoy"))?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(21),
            entries: vec![directory_entry(&root)],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Deleted { .. }))
    {
        context.iteration(true);
    }

    assert!(
        sentinel.exists(),
        "a symlink nested inside the deleted tree must never lead outside it"
    );
    assert_eq!(fs::read_dir(&outside)?.count(), 1);
    assert!(!root.exists());

    fs::remove_dir_all(outside)?;
    Ok(())
}

#[test]
fn permanent_delete_accepts_a_symlink_in_the_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual_parent = root.path().join("actual");
    let linked_parent = root.path().join("linked");
    let target = actual_parent.join("target.txt");
    fs::create_dir(&actual_parent)?;
    fs::write(&target, b"keep")?;
    std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(22),
            entries: vec![file_entry(&linked_parent.join("target.txt"))],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Deleted { .. }
                | OperationEvent::CompletedWithErrors { .. }
                | OperationEvent::Failed { .. }
        )
    }) {
        context.iteration(true);
    }

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Deleted { .. })),
        "{:?}",
        events.borrow()
    );
    assert!(!target.exists());
    assert!(linked_parent.is_symlink());
    assert!(actual_parent.is_dir());
    Ok(())
}

#[test]
fn parent_resolution_accepts_absolute_relative_and_chained_aliases() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    fs::create_dir_all(actual.join("nested"))?;
    std::os::unix::fs::symlink(&actual, root.path().join("absolute"))?;
    std::os::unix::fs::symlink("actual", root.path().join("relative"))?;
    std::os::unix::fs::symlink("relative", root.path().join("chain"))?;
    std::os::unix::fs::symlink("../relative", actual.join("up"))?;
    let expected = super::LocalFileIdentity::from_stat(&rustix::fs::stat(&actual)?);

    for name in [
        "actual",
        "absolute",
        "relative",
        "chain",
        "actual/up",
        "chain/nested/..",
    ] {
        let parent = super::open_local_parent_directory(&root.path().join(name))?;
        assert_eq!(
            super::LocalFileIdentity::from_stat(&rustix::fs::fstat(&parent)?),
            expected,
            "{name}"
        );
    }
    let parent = super::open_local_parent_directory(Path::new("/"))?;
    assert_eq!(
        super::LocalFileIdentity::from_stat(&rustix::fs::fstat(&parent)?),
        super::LocalFileIdentity::from_stat(&rustix::fs::stat(c"/")?)
    );
    Ok(())
}

#[test]
fn parent_resolution_rejects_magic_links_loops_and_dangling_aliases() -> Result<(), Box<dyn Error>>
{
    use std::os::fd::AsRawFd;

    let root = tempfile::tempdir()?;
    let handle = fs::File::open(root.path())?;
    let magic = PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()));
    assert!(
        magic.is_dir(),
        "the fixture must expose a working procfs magic link"
    );
    std::os::unix::fs::symlink(&magic, root.path().join("magic"))?;
    std::os::unix::fs::symlink("loop", root.path().join("loop"))?;
    std::os::unix::fs::symlink("missing", root.path().join("dangling"))?;

    for path in [
        magic,
        root.path().join("magic"),
        root.path().join("loop"),
        root.path().join("dangling"),
        PathBuf::from("relative"),
    ] {
        assert!(
            super::open_local_parent_directory(&path).is_err(),
            "{}",
            path.display()
        );
    }
    Ok(())
}

#[test]
fn permanent_delete_keeps_the_open_parent_when_its_alias_is_retargeted()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let outside = root.path().join("outside");
    let alias = root.path().join("alias");
    fs::create_dir_all(actual.join("tree/nested"))?;
    fs::create_dir_all(outside.join("tree"))?;
    fs::write(actual.join("tree/nested/file.txt"), b"delete")?;
    fs::write(outside.join("tree/sentinel.txt"), b"keep")?;
    std::os::unix::fs::symlink(&outside, actual.join("tree/decoy"))?;
    std::os::unix::fs::symlink(&actual, &alias)?;
    let parent = super::open_local_parent_directory(&alias)?;

    std::os::unix::fs::symlink(&outside, root.path().join("replacement"))?;
    fs::rename(root.path().join("replacement"), &alias)?;
    glib::MainContext::default().block_on(super::permanently_delete_local(
        parent,
        OsString::from("tree"),
        None,
        gio::Cancellable::new(),
    ))?;

    assert!(!actual.join("tree").exists());
    assert_eq!(fs::read(outside.join("tree/sentinel.txt"))?, b"keep");
    assert_eq!(fs::read_link(alias)?, outside);
    Ok(())
}

#[test]
fn permanent_delete_revalidates_identity_after_a_parent_alias_changes() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let outside = root.path().join("outside");
    let alias = root.path().join("alias");
    fs::create_dir(&actual)?;
    fs::create_dir(&outside)?;
    fs::write(actual.join("file.txt"), b"original")?;
    fs::write(outside.join("file.txt"), b"keep")?;
    std::os::unix::fs::symlink(&actual, &alias)?;
    let file = gio::File::for_path(alias.join("file.txt"));
    let context = glib::MainContext::default();
    let expected = context.block_on(super::local_file_identity(&file))?;

    std::os::unix::fs::symlink(&outside, root.path().join("replacement"))?;
    fs::rename(root.path().join("replacement"), &alias)?;
    let error = context
        .block_on(super::permanently_delete_local_path_if_unchanged(
            alias.join("file.txt"),
            expected,
            gio::Cancellable::new(),
        ))
        .expect_err("retargeting the alias must not delete a different entry");

    assert!(error.to_string().contains("changed"), "{error}");
    assert_eq!(fs::read(actual.join("file.txt"))?, b"original");
    assert_eq!(fs::read(outside.join("file.txt"))?, b"keep");
    Ok(())
}

#[test]
fn permanent_delete_of_a_symlink_through_an_alias_keeps_its_referent() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let alias = root.path().join("alias");
    fs::create_dir(&actual)?;
    fs::write(actual.join("sentinel.txt"), b"keep")?;
    std::os::unix::fs::symlink("actual", &alias)?;
    std::os::unix::fs::symlink(".", actual.join("link"))?;

    glib::MainContext::default().block_on(super::permanently_delete_local_path_if_unchanged(
        alias.join("link"),
        None,
        gio::Cancellable::new(),
    ))?;

    assert!(!actual.join("link").is_symlink());
    assert_eq!(fs::read(actual.join("sentinel.txt"))?, b"keep");
    assert!(alias.is_symlink());
    Ok(())
}

#[test]
fn copying_and_replacing_symlinks_accepts_an_aliased_destination() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let alias = root.path().join("alias");
    let source = root.path().join("source");
    fs::create_dir(&actual)?;
    fs::write(root.path().join("sentinel.txt"), b"keep")?;
    std::os::unix::fs::symlink("actual", &alias)?;
    std::os::unix::fs::symlink("../sentinel.txt", &source)?;
    let context = glib::MainContext::default();

    for overwrite in [false, true] {
        context.block_on(copy_recursively(
            gio::File::for_path(&source),
            gio::File::for_path(alias.join("link")),
            overwrite,
            gio::Cancellable::new(),
            None,
        ))?;
        assert_eq!(
            fs::read_link(actual.join("link"))?,
            Path::new("../sentinel.txt")
        );
    }
    context.block_on(copy_new_recursively(
        gio::File::for_path(root.path().join("sentinel.txt")),
        gio::File::for_path(alias.join("new.txt")),
        gio::Cancellable::new(),
    ))?;
    assert_eq!(fs::read(actual.join("new.txt"))?, b"keep");
    assert_eq!(fs::read(root.path().join("sentinel.txt"))?, b"keep");
    Ok(())
}

#[test]
fn an_already_cancelled_recursive_delete_preserves_the_root() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-recursive-delete-cancel-test-{unique}"));
    let nested = root.join("nested");
    fs::create_dir_all(&nested)?;
    for index in 0..64 {
        fs::write(nested.join(format!("item-{index}.txt")), b"contents")?;
    }

    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    let parent = super::open_local_parent_directory(root.parent().ok_or("no parent")?)?;
    let delete_root = super::LocalDeleteRoot {
        parent: Arc::new(parent),
        name: root.file_name().ok_or("no name")?.to_owned(),
        expected: None,
    };
    let context = glib::MainContext::default();
    let error = context
        .block_on(super::parallel_delete_local(vec![delete_root], cancellable))
        .expect_err("cancelled delete must return error");

    assert!(super::was_cancelled(&error));
    assert!(root.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn transfer_progress_aggregates_completed_and_in_flight_file_bytes() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let tracker = TransferProgressTracker::new(
        OperationRequestId(24),
        Some(150),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    let first = tracker.begin_file();
    let mut first_callback = first.callback();
    first_callback(25, 100);
    first_callback(100, 100);
    first.finish();
    tracker.finish_item(0, Some(100), None);

    let second = tracker.begin_file();
    let mut second_callback = second.callback();
    second_callback(10, 50);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        OperationEvent::TransferProgress {
            completed_items: 0,
            transferred_bytes: 25,
            total_bytes: Some(150),
            ..
        }
    )));
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::TransferProgress {
            completed_items: 1,
            transferred_bytes: 110,
            total_bytes: Some(150),
            ..
        })
    ));
}

#[test]
fn copying_a_file_emits_bytes_before_item_completion() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination");
    let contents = vec![0x5a; 1024 * 1024];
    fs::write(&source, &contents)?;
    fs::create_dir(&destination)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(25),
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
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        OperationEvent::TransferProgress {
            completed_items: 0,
            transferred_bytes,
            total_bytes: Some(total_bytes),
            ..
        } if *transferred_bytes > 0 && *total_bytes == contents.len() as u64
    )));
    assert_eq!(fs::read(destination.join("source.bin"))?, contents);
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
            let cancel = matches!(
                event,
                OperationEvent::TransferProgress {
                    completed_items: 1,
                    ..
                }
            );
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
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        OperationEvent::TransferProgress {
            completed_items: 1,
            transferred_bytes: 5,
            total_bytes: Some(11),
            ..
        }
    )));
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

    let entries = home_trash_entries_at(
        &trash,
        &HashSet::from([original.clone()]),
        &gio::Cancellable::new(),
    );

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
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    assert!(home_trash_entries_at(&trash, &HashSet::from([original]), &cancellable).is_empty());
    assert!(fs::symlink_metadata(trash.join("files/report.txt")).is_ok());
    assert!(trash.join("info/report.txt.trashinfo").exists());
    fs::remove_dir_all(fixture)?;
    Ok(())
}

#[test]
fn cancelling_restore_before_io_reports_every_item_as_unattempted() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let location = Location::local("/fixture/trashed.txt");
    for (source, expected) in [
        (
            RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry: file_entry(Path::new("/fixture/trashed.txt")),
                destination: PathBuf::from("/fixture/trashed.txt"),
            }]),
            vec![location.clone()],
        ),
        (
            RestoreSource::OriginalLocations(vec![location.clone()]),
            vec![location],
        ),
        (RestoreSource::OriginalLocations(Vec::new()), Vec::new()),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let operation = LocalOperationProvider.restore(
            RestoreRequest {
                id: OperationRequestId(9),
                source,
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
                    && result.not_attempted == expected
        ));
    }
    Ok(())
}

fn wait_for_restore(events: &Rc<RefCell<Vec<OperationEvent>>>) {
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Restored { .. }
                | OperationEvent::RestoreCompletedWithErrors { .. }
                | OperationEvent::Failed { .. }
                | OperationEvent::Cancelled { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }
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

#[test]
fn restore_rejects_a_volume_orig_path_on_another_device() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let Some((home, stick)) = crate::test_support::distinct_device_dirs(
        "restore_rejects_a_volume_orig_path_on_another_device",
    ) else {
        return Ok(());
    };
    let dest = home.path().join(".config/autostart/payload.desktop");
    let (source, entry) = volume_trash_entry(
        stick.path(),
        "payload",
        &dest.to_string_lossy(),
        b"ssh-ed25519 AAAA attacker",
    )?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(478),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: dest.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::RestoreCompletedWithErrors { message, .. })
                if message.contains("outside the trash volume")
        ),
        "{:?}",
        events.borrow()
    );
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"ssh-ed25519 AAAA attacker");
    assert!(!dest.exists());
    Ok(())
}

#[test]
fn restore_returns_a_volume_item_to_a_path_on_the_same_volume() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let fixture = tempfile::tempdir()?;
    fs::create_dir_all(fixture.path().join("Documents"))?;
    let (source, entry) = volume_trash_entry(
        fixture.path(),
        "report.txt",
        "Documents/report.txt",
        b"notes",
    )?;
    let destination = fixture.path().canonicalize()?.join("Documents/report.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(479),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: destination.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(&destination)?, b"notes");
    assert!(!source.exists());
    Ok(())
}

#[test]
fn restore_fails_when_the_confirmed_destination_no_longer_matches() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let fixture = tempfile::tempdir()?;
    fs::create_dir_all(fixture.path().join("Documents"))?;
    let (source, entry) = volume_trash_entry(
        fixture.path(),
        "report.txt",
        "Documents/report.txt",
        b"notes",
    )?;
    let confirmed = fixture.path().canonicalize()?.join("Documents/other.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(480),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: confirmed.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::RestoreCompletedWithErrors { message, .. })
                if message.contains("no longer matches the confirmed destination")
        ),
        "{:?}",
        events.borrow()
    );
    assert!(source.exists());
    assert!(!confirmed.exists());
    assert!(!fixture.path().join("Documents/report.txt").exists());
    Ok(())
}

#[test]
fn restore_uses_the_trash_entry_target_path_as_the_physical_source() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let fixture = tempfile::tempdir()?;
    fs::create_dir_all(fixture.path().join("Documents"))?;
    let (source, mut entry) = volume_trash_entry(
        fixture.path(),
        "report.txt",
        "Documents/report.txt",
        b"notes",
    )?;
    entry.location = Location::uri("trash:///report.txt");
    entry.thumbnail_path = Some(source.clone());
    let destination = fixture.path().canonicalize()?.join("Documents/report.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(481),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: destination.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(&destination)?, b"notes");
    assert!(!source.exists());
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

fn drive_until_transfer_settles(events: &Rc<RefCell<Vec<OperationEvent>>>) {
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. }
                | OperationEvent::TransferFailed { .. }
                | OperationEvent::Cancelled { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }
}

#[test]
fn undoing_a_move_returns_each_item_to_its_original_directory() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let moved = archive.join("report.txt");
    fs::write(&moved, b"contents")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            id: OperationRequestId(40),
            items: vec![UndoMoveItem {
                record: MoveRecord {
                    original: Location::local(origin.join("report.txt")),
                    current: Location::local(&moved),
                },
                conflict: TransferConflict::FailIfExists,
            }],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drive_until_transfer_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(!moved.exists());
    assert_eq!(fs::read(origin.join("report.txt"))?, b"contents");
    Ok(())
}

#[test]
fn undoing_a_move_stops_at_an_unconfirmed_conflict() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let blocked = origin.join("report.txt");
    fs::write(&blocked, b"newer")?;
    let first = archive.join("notes.txt");
    let second = archive.join("report.txt");
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            id: OperationRequestId(41),
            items: vec![
                UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(origin.join("notes.txt")),
                        current: Location::local(&first),
                    },
                    conflict: TransferConflict::FailIfExists,
                },
                UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(&blocked),
                        current: Location::local(&second),
                    },
                    conflict: TransferConflict::FailIfExists,
                },
            ],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drive_until_transfer_settles(&events);

    let completed = match events.borrow().last() {
        Some(OperationEvent::TransferFailed {
            completed_locations,
            ..
        }) => completed_locations.clone(),
        other => panic!("expected a transfer failure, got {other:?}"),
    };
    assert_eq!(completed, vec![Location::local(&first)]);
    assert_eq!(fs::read(&blocked)?, b"newer");
    assert_eq!(fs::read(&second)?, b"second");
    assert_eq!(fs::read(origin.join("notes.txt"))?, b"first");
    Ok(())
}

#[test]
fn a_confirmed_undo_conflict_replaces_the_newer_item() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let original = origin.join("report.txt");
    let moved = archive.join("report.txt");
    fs::write(&original, b"newer")?;
    fs::write(&moved, b"moved")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            id: OperationRequestId(42),
            items: vec![UndoMoveItem {
                record: MoveRecord {
                    original: Location::local(&original),
                    current: Location::local(&moved),
                },
                conflict: TransferConflict::ReplaceExisting,
            }],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drive_until_transfer_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(!moved.exists());
    assert_eq!(fs::read(&original)?, b"moved");
    Ok(())
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

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

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

#[test]
fn a_copy_reports_the_destination_it_created() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("photo.jpg");
    let destination = root.path().join("album");
    fs::write(&source, b"original-content")?;
    fs::create_dir(&destination)?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(70),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::FailIfExists,
        }],
        move_sources: false,
    })?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("photo.jpg"))]
    );
    Ok(())
}

#[test]
fn duplicating_a_file_preserves_contents_and_reports_the_generated_name()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("photo.jpg");
    fs::write(&source, b"original-content")?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(71),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::FailIfExists,
        }],
        move_sources: false,
    })?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("photo (1).jpg"))]
    );
    assert_eq!(fs::read(&source)?, b"original-content");
    assert_eq!(
        fs::read(destination.join("photo (1).jpg"))?,
        b"original-content"
    );
    Ok(())
}

#[test]
fn a_copy_that_replaces_an_existing_item_reports_its_destination() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("photo.jpg");
    let destination = root.path().join("album");
    fs::write(&source, b"new-content")?;
    fs::create_dir(&destination)?;
    fs::write(destination.join("photo.jpg"), b"old-content")?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(72),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::ReplaceExisting,
        }],
        move_sources: false,
    })?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("photo.jpg"))]
    );
    Ok(())
}

#[test]
fn a_move_reports_no_created_destination() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("photo.jpg");
    let destination = root.path().join("album");
    fs::write(&source, b"original-content")?;
    fs::create_dir(&destination)?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(73),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::FailIfExists,
        }],
        move_sources: true,
    })?;

    assert!(created.into_iter().flatten().next().is_none());
    Ok(())
}

#[test]
fn keeping_both_preserves_transfer_noops() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let nested = source.join("nested");
    fs::create_dir_all(&nested)?;
    fs::write(source.join("report.txt"), b"original")?;

    for moving in [false, true] {
        for destination in [&source, &nested] {
            let created = run_paste_collecting_created(PasteRequest {
                id: OperationRequestId(77),
                destination: Location::local(destination),
                items: vec![PasteItem {
                    source: Location::local(&source),
                    conflict: TransferConflict::KeepBoth,
                }],
                move_sources: moving,
            })?;
            assert!(created.into_iter().flatten().next().is_none());
            assert_eq!(fs::read_dir(&source)?.count(), 2);
            assert_eq!(fs::read_dir(&nested)?.count(), 0);
            assert_eq!(fs::read(source.join("report.txt"))?, b"original");
        }
    }
    Ok(())
}

#[test]
fn keeping_both_in_a_cross_folder_paste_generates_a_unique_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let source = source_dir.join("report.txt");
    let destination = root.path().join("dest");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("report.txt"), b"existing")?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(74),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::KeepBoth,
        }],
        move_sources: false,
    })?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("report (1).txt"))]
    );
    assert_eq!(fs::read(destination.join("report.txt"))?, b"existing");
    assert_eq!(fs::read(destination.join("report (1).txt"))?, b"incoming");
    assert!(source.exists());
    Ok(())
}

#[test]
fn keeping_both_while_moving_renames_the_destination_instead_of_replacing_it()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let source = source_dir.join("report.txt");
    let destination = root.path().join("dest");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("report.txt"), b"existing")?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(75),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::KeepBoth,
        }],
        move_sources: true,
    })?;

    assert!(created.into_iter().flatten().next().is_none());
    assert_eq!(fs::read(destination.join("report.txt"))?, b"existing");
    assert_eq!(fs::read(destination.join("report (1).txt"))?, b"incoming");
    assert!(!source.exists());
    Ok(())
}

#[test]
fn mixed_conflict_choices_apply_independently_across_a_multi_item_paste()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    fs::write(sources.join("new.txt"), b"brand new")?;
    fs::write(sources.join("replace.txt"), b"new replacement")?;
    fs::write(destination.join("replace.txt"), b"old replacement")?;
    fs::write(sources.join("keep.txt"), b"new keep")?;
    fs::write(destination.join("keep.txt"), b"old keep")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(76),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(sources.join("new.txt")),
                    conflict: TransferConflict::FailIfExists,
                },
                PasteItem {
                    source: Location::local(sources.join("replace.txt")),
                    conflict: TransferConflict::ReplaceExisting,
                },
                PasteItem {
                    source: Location::local(sources.join("keep.txt")),
                    conflict: TransferConflict::KeepBoth,
                },
            ],
            move_sources: false,
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
        Some(OperationEvent::Pasted { .. })
    ));
    assert_eq!(fs::read(destination.join("new.txt"))?, b"brand new");
    assert_eq!(
        fs::read(destination.join("replace.txt"))?,
        b"new replacement"
    );
    assert_eq!(fs::read(destination.join("keep.txt"))?, b"old keep");
    assert_eq!(fs::read(destination.join("keep (1).txt"))?, b"new keep");
    Ok(())
}

#[test]
fn copying_a_tree_with_a_named_pipe_fails_instead_of_blocking() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("before.txt"), b"before")?;
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        source.join("pipe"),
        rustix::fs::Mode::from_bits_truncate(0o600),
    )?;

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    let error = result.expect_err("a named pipe cannot be copied as a regular file");
    assert!(
        error.to_string().contains("pipe"),
        "the error should name the entry: {error}"
    );
    assert!(!target.join("pipe").exists());
    Ok(())
}

#[test]
fn hidden_file_copy_suffix_parsing_preserves_dot_prefix() {
    assert_eq!(
        parse_copy_suffix(OsStr::new(".gitignore")),
        (OsStr::new(".gitignore"), None)
    );
    assert_eq!(
        parse_copy_suffix(OsStr::new(".gitignore (1)")),
        (OsStr::new(".gitignore"), Some(1))
    );
    assert_eq!(
        duplicate_candidate_name(OsStr::new(".gitignore"), None, 1),
        OsString::from(".gitignore (1)")
    );
    assert_eq!(
        duplicate_candidate_name(OsStr::new(".config"), None, 2),
        OsString::from(".config (2)")
    );
}

#[test]
fn multi_extension_copy_suffix_parsing_preserves_full_extension() {
    assert_eq!(
        parse_copy_suffix(OsStr::new("archive")),
        (OsStr::new("archive"), None)
    );
    assert_eq!(
        parse_copy_suffix(OsStr::new("archive (1)")),
        (OsStr::new("archive"), Some(1))
    );
    assert_eq!(
        duplicate_candidate_name(OsStr::new("archive"), Some(OsStr::new("tar.gz")), 1),
        OsString::from("archive (1).tar.gz")
    );
    assert_eq!(
        duplicate_candidate_name(OsStr::new("archive"), Some(OsStr::new("tar.gz")), 3),
        OsString::from("archive (3).tar.gz")
    );
    assert_eq!(
        duplicate_candidate_name(OsStr::new("backup.tar"), Some(OsStr::new("gz")), 1),
        OsString::from("backup.tar (1).gz")
    );
}

#[test]
fn duplicating_hidden_file_generates_correct_numbered_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join(".gitignore");
    fs::write(&source, b"target/")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(30),
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
    assert_eq!(fs::read(destination.join(".gitignore (1)"))?, b"target/");
    Ok(())
}

#[test]
fn duplicating_multi_extension_file_uses_last_extension_for_candidate_name()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("backup.tar.gz");
    fs::write(&source, b"contents")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(31),
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
    assert_eq!(
        fs::read(destination.join("backup.tar (1).gz"))?,
        b"contents"
    );
    Ok(())
}

#[test]
fn pasting_onto_itself_with_replace_is_a_noop() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("todo.txt");
    fs::write(&source, b"existing\n")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(32),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::ReplaceExisting,
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
    assert_eq!(fs::read(&source)?, b"existing\n");
    assert!(!destination.join("todo (1).txt").exists());
    Ok(())
}

#[test]
fn pasting_onto_itself_with_keep_both_creates_numbered_copy() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("todo.txt");
    fs::write(&source, b"existing\n")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(33),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::KeepBoth,
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
    assert_eq!(fs::read(&source)?, b"existing\n");
    assert_eq!(fs::read(destination.join("todo (1).txt"))?, b"existing\n");
    Ok(())
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

mod create_entry;
mod deletion;
mod trash_capabilities;

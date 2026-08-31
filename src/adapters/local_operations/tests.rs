// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    error::Error,
    fs,
    rc::Rc,
    sync::Mutex,
    time::SystemTime,
};

use gtk::{gio, glib, prelude::*};

static ASYNC_FILE_TEST: Mutex<()> = Mutex::new(());

use super::{
    LocalOperationProvider, copy_recursively, deletion_error_message, deletion_error_summary,
    operation_error_summary, replace_local, replace_local_with, transfer_is_noop, validated_child,
};
use crate::{
    model::Location,
    services::{
        OperationEvent, OperationProvider, OperationRequestId, PasteItem, PasteRequest,
        TransferConflict,
    },
};

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
fn recursive_copy_preserves_nested_directory_contents() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_FILE_TEST.lock().map_err(|error| error.to_string())?;
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
    ));

    assert!(result.is_ok());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    assert_eq!(fs::read(target.join("nested/child.txt"))?, b"child");

    fs::write(source.join("top.txt"), b"replacement")?;
    let overwrite = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
    ));
    assert!(overwrite.is_ok());
    assert_eq!(fs::read(target.join("top.txt"))?, b"replacement");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn staged_file_replacement_preserves_the_destination_on_disk_full() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_FILE_TEST.lock().map_err(|error| error.to_string())?;
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
        Rc::new(|_, staged, _| {
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

#[test]
fn cancelling_staging_preserves_the_destination_and_cleans_the_partial_copy()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_FILE_TEST.lock().map_err(|error| error.to_string())?;
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

    let task = glib::MainContext::default().spawn_local(replace_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        Rc::new(move |_, staged, _| {
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
                std::future::pending().await
            })
        }),
    ));
    let context = glib::MainContext::default();
    while !staging.get() {
        context.iteration(true);
    }
    task.abort();
    drop(task);
    while context.pending() {
        context.iteration(false);
    }

    assert_eq!(fs::read(&target)?, b"original");
    assert_eq!(fs::read(&source)?, b"replacement");
    assert_eq!(fs::read_dir(&root)?.count(), 2);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn staged_file_replacement_commits_then_removes_a_moved_source() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_FILE_TEST.lock().map_err(|error| error.to_string())?;
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
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(&target)?, b"replacement");
    assert!(!source.exists());
    assert_eq!(fs::read_dir(&root)?.count(), 1);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn each_transfer_item_keeps_its_own_conflict_decision() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_FILE_TEST.lock().map_err(|error| error.to_string())?;
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
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while events.borrow().is_empty() {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().as_slice(),
        [OperationEvent::Failed { .. }]
    ));
    assert_eq!(
        fs::read(destination.join("replace.txt"))?,
        b"new replacement"
    );
    assert_eq!(fs::read(destination.join("late.txt"))?, b"late arrival");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn staged_directory_replacement_does_not_merge_old_contents() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_FILE_TEST.lock().map_err(|error| error.to_string())?;
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
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("new/item.txt"))?, b"new");
    assert!(!target.join("old").exists());
    assert!(source.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

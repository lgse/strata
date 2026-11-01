// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn deletion_error_summaries_are_bounded_and_report_the_failure_count() {
    let errors = (1..=10)
        .map(|index| format!("item-{index}: denied"))
        .collect::<Vec<_>>();

    let summary = deletion_error_summary(&errors, true);

    assert!(summary.starts_with("10 items could not be deleted"));
    assert!(summary.contains("• item-1: denied"));
    assert!(summary.contains("• item-8: denied"));
    assert!(!summary.contains("• item-9: denied"));
    assert!(summary.ends_with("… and 2 more items"));
    assert!(
        deletion_error_summary(&errors[..1], false)
            .starts_with("1 item could not be moved to Trash")
    );
    assert!(
        operation_error_summary(&errors[..1], FailedAction::Restore)
            .starts_with("1 item could not be restored")
    );
}

#[test]
fn large_deletion_progress_reports_every_completed_location() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let mut entries = Vec::new();
    let mut expected = HashSet::new();
    for index in 0..40 {
        let path = root.path().join(format!("file-{index:02}.txt"));
        fs::write(&path, index.to_string())?;
        expected.insert(Location::local(&path));
        entries.push(file_entry(&path));
    }

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(24),
            entries,
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Deleted { .. }))
    {
        glib::MainContext::default().iteration(true);
    }

    let reported = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            OperationEvent::DeleteProgress {
                deleted_locations, ..
            } => Some(deleted_locations),
            _ => None,
        })
        .flatten()
        .cloned()
        .collect::<HashSet<_>>();
    assert_eq!(reported, expected);
    Ok(())
}

#[test]
fn deletion_failures_explain_trash_problems_and_keep_other_details() {
    use gio::IOErrorEnum as Io;
    let unsupported = TrashError::from(glib::Error::new(Io::NotSupported, "trash not supported"));
    let cases = [
        (
            unsupported,
            true,
            "share-folder: This location doesn't support Trash.",
        ),
        (
            TrashError::permanent(glib::Error::new(Io::NotSupported, "trash not supported")),
            false,
            "share-folder: trash not supported",
        ),
        (
            TrashError::from(glib::Error::new(Io::ReadOnly, "read-only")),
            false,
            "share-folder: This location is read-only.",
        ),
        (
            TrashError::from(glib::Error::new(Io::NoSpace, "full")),
            false,
            "share-folder: Not enough space to move to Trash.",
        ),
        (
            TrashError::from(glib::Error::new(
                Io::PermissionDenied,
                "Unable to trash file /tmp/secret.txt: access denied",
            )),
            false,
            "share-folder: Permission denied",
        ),
        (
            TrashError::from(glib::Error::new(Io::Failed, "disk exploded")),
            false,
            "share-folder: disk exploded",
        ),
    ];
    for (error, retryable, expected) in cases {
        assert_eq!(error.is_unsupported(), retryable, "{expected}");
        assert_eq!(deletion_error_message("share-folder", &error), expected);
    }
}

#[test]
fn a_refused_trash_is_retryable_and_remembered_only_on_fuse() -> Result<(), Box<dyn Error>> {
    use crate::adapters::trash_support::{
        tests::RefusingMount, tests::leftover_trash, unsupported_reason_in,
    };
    let _serial = serial()?;
    for (fs_type, unsupported) in [("fuse.sshfs", true), ("ext4", false)] {
        let root = tempfile::tempdir()?;
        let doomed = root.path().join("doomed.txt");
        fs::write(&doomed, b"doomed")?;
        let environment = Rc::new(RefusingMount::at(root.path(), 7301, fs_type));
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        glib::MainContext::default().block_on(run_deletion_in(
            environment.clone(),
            OperationRequestId(73),
            vec![DeletionTarget::from_entry(&file_entry(&doomed))],
            false,
            Rc::new(move |event| emitted.borrow_mut().push(event)),
            gio::Cancellable::new(),
        ));

        let events = events.borrow();
        let Some(OperationEvent::CompletedWithErrors {
            retryable_locations,
            has_non_retryable_failures,
            message,
            ..
        }) = events.last()
        else {
            panic!("{fs_type}: expected a partial failure, got {events:?}");
        };
        let expected_retryable = if unsupported {
            vec![Location::local(&doomed)]
        } else {
            Vec::new()
        };
        assert_eq!(retryable_locations, &expected_retryable, "{fs_type}");
        assert_eq!(*has_non_retryable_failures, !unsupported, "{fs_type}");
        assert_eq!(
            message.contains("doomed.txt: This location doesn't support Trash."),
            unsupported,
            "{fs_type}: {message}"
        );
        assert_eq!(
            unsupported_reason_in(&environment.mounts, &doomed).is_some(),
            unsupported,
            "{fs_type}"
        );
        assert_eq!(
            leftover_trash(root.path()).exists(),
            !unsupported,
            "{fs_type}"
        );
        assert!(doomed.exists(), "{fs_type}");
    }
    Ok(())
}

#[test]
fn cancelling_between_deletions_reports_completed_and_unattempted_items()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
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

/// Cancels as the delete of `cut` starts: before Trash takes it, or after
/// a permanent delete checks it or the file inside it.
fn cancel_at_cut(permanent: bool, cut_is_folder: bool, cancellable: &gio::Cancellable) -> Quirks {
    let cancel = cancellable.clone();
    let cancel: Racer = Box::new(move || cancel.cancel());
    if !permanent {
        return Quirks {
            before: vec![(Op::Trash, "cut", 0, cancel)],
            ..Quirks::default()
        };
    }
    let checked = if cut_is_folder { "inside.txt" } else { "cut" };
    Quirks {
        after: vec![(Op::Stat, checked, 0, cancel)],
        ..Quirks::default()
    }
}

fn check_cut_off_delete(
    row: &str,
    permanent: bool,
    cut_is_folder: bool,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let [denied, cut, rest] = ["denied.txt", "cut", "rest.txt"].map(|name| root.path().join(name));
    let (cut_entry, cut_inside) = if cut_is_folder {
        fs::create_dir(&cut)?;
        (directory_entry(&cut), cut.join("inside.txt"))
    } else {
        (file_entry(&cut), cut.clone())
    };
    for item in [&denied, &cut_inside, &rest] {
        fs::write(item, b"kept")?;
    }
    let cancellable = gio::Cancellable::new();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::Stat, "denied.txt", 0, rustix::io::Errno::ACCESS)],
            ..cancel_at_cut(permanent, cut_is_folder, &cancellable)
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();

    glib::MainContext::default().block_on(super::super::super::run_deletion(
        OperationRequestId(1615),
        [file_entry(&denied), cut_entry, file_entry(&rest)]
            .iter()
            .map(super::super::super::DeletionTarget::from_entry)
            .collect(),
        permanent,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        cancellable,
    ));

    let events = events.borrow();
    let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
        panic!("{row}: expected a cancel, got {events:?}");
    };
    assert!(result.completed.is_empty(), "{row}");
    let locations = |paths: &[&PathBuf]| paths.iter().map(Location::local).collect::<Vec<_>>();
    if permanent {
        assert!(
            !cut_inside.exists(),
            "{row}: the cancel landed before the delete"
        );
        assert_eq!(
            (&result.failed, &result.not_attempted),
            (&locations(&[&denied, &cut]), &locations(&[&rest])),
            "{row}"
        );
    } else {
        assert_eq!(fs::read(&cut_inside)?, b"kept", "{row}");
        assert_eq!(
            (&result.failed, &result.not_attempted),
            (&locations(&[&denied]), &locations(&[&cut, &rest])),
            "{row}"
        );
    }
    for item in [&denied, &rest] {
        assert_eq!(fs::read(item)?, b"kept", "{row}");
    }
    Ok(())
}

#[test]
fn a_cancelled_delete_counts_only_what_it_left_whole_as_not_attempted() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for (row, permanent, cut_is_folder) in [
        ("to Trash", false, false),
        ("a file permanently", true, false),
        ("a folder permanently", true, true),
    ] {
        check_cut_off_delete(row, permanent, cut_is_folder)?;
    }
    Ok(())
}

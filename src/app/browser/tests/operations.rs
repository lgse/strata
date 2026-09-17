// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn deleted_trash_entries_refresh_the_trash_root() {
    let entry = FileEntry {
        location: Location::uri("trash:///photo.jpg"),
        native_name: "photo.jpg".into(),
        thumbnail_path: None,
        display_name: "photo.jpg".into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(10),
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    };

    assert_eq!(
        deletion_parent_location(&entry.location),
        Some(Location::uri("trash:///"))
    );
}

#[test]
fn deletion_monitor_changes_publish_once_after_the_terminal_event() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let watched = Location::local("/fixture");
    let first = batch_entry("first");
    let second = batch_entry("second");
    {
        let mut state = browser.state.borrow_mut();
        state.navigate(watched.clone(), RequestId(1));
        let _ = state.apply_batch(RequestId(1), vec![first.clone(), second.clone()]);
    }
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = OperationRequestId(9);
    browser.current_operation.set(Some(request_id));
    browser.deletion_operation.set(true);
    let complete = browser.operation_callback(request_id, false, HashSet::new());

    for entry in [&first, &second] {
        browser.handle_directory_change(
            0,
            &watched,
            DirectoryChange::Remove(entry.location.clone()),
        );
    }
    complete(OperationEvent::DeleteProgress {
        request_id,
        completed: 1,
        total: 2,
        deleted_location: Some(first.location.clone()),
    });

    assert_eq!(
        browser.column_snapshot(0).map(|column| column.count),
        Some(2)
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
    );

    complete(OperationEvent::Deleted {
        request_id,
        locations: vec![first.location, second.location],
    });

    assert_eq!(
        browser.column_snapshot(0).map(|column| column.count),
        Some(0)
    );
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
            .count(),
        1
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::DeletionFinished { succeeded: true }))
    );
}

#[test]
fn large_deletion_refreshes_sources_missing_from_the_monitor_batch() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let watched = Location::local("/fixture");
    let entries: Vec<_> = (0..65)
        .map(|index| batch_entry(&index.to_string()))
        .collect();
    {
        let mut state = browser.state.borrow_mut();
        state.navigate(watched.clone(), RequestId(1));
        let _ = state.apply_batch(RequestId(1), entries.clone());
    }
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = OperationRequestId(11);
    browser.current_operation.set(Some(request_id));
    browser.deletion_operation.set(true);
    let complete = browser.operation_callback(request_id, false, HashSet::new());
    browser.handle_directory_change(
        1,
        &Location::local("/previous/child"),
        DirectoryChange::Rescan,
    );

    complete(OperationEvent::Deleted {
        request_id,
        locations: entries.into_iter().map(|entry| entry.location).collect(),
    });

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
}

#[test]
fn restoration_monitor_changes_publish_once_after_the_terminal_event() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let watched = Location::uri("trash:///");
    let first = trash_entry("first");
    let second = trash_entry("second");
    {
        let mut state = browser.state.borrow_mut();
        state.navigate(watched.clone(), RequestId(1));
        let _ = state.apply_batch(RequestId(1), vec![first.clone(), second.clone()]);
    }
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = OperationRequestId(10);
    browser.current_operation.set(Some(request_id));
    browser.restoration_operation.set(true);
    let complete = browser.operation_callback(request_id, false, HashSet::new());

    for entry in [&first, &second] {
        browser.handle_directory_change(
            0,
            &watched,
            DirectoryChange::Remove(entry.location.clone()),
        );
    }
    complete(OperationEvent::RestoreProgress {
        request_id,
        completed: 1,
        total: 2,
        restored_location: Some(first.location.clone()),
    });

    assert_eq!(
        browser.column_snapshot(0).map(|column| column.count),
        Some(2)
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
    );

    complete(OperationEvent::Restored {
        request_id,
        locations: vec![first.location, second.location],
    });

    assert_eq!(
        browser.column_snapshot(0).map(|column| column.count),
        Some(0)
    );
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
            .count(),
        1
    );
}

#[test]
fn invalid_new_entry_names_are_rejected_before_an_operation_starts() {
    for folder in [true, false] {
        for name in ["../escaped", "", "..", "nul\0name"] {
            assert_invalid_creation_is_rejected(name, |browser| {
                if folder {
                    browser.create_directory_with_naming(
                        Location::local("/fixture"),
                        name.to_owned(),
                        false,
                    );
                } else {
                    browser.create_file_with_naming(
                        Location::local("/fixture"),
                        name.to_owned(),
                        false,
                    );
                }
            });
        }
    }
}

#[test]
fn new_files_and_folders_request_unique_naming_and_report_the_created_location() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.create_new_folder(Location::local("/fixture"));
    assert!(events.borrow().iter().any(|event| matches!(event,
        BrowserEvent::EntryCreated { location } if location == &Location::local("/fixture/new folder")
    )));
    browser.create_new_file(Location::local("/fixture"));
    assert!(events.borrow().iter().any(|event| matches!(event,
        BrowserEvent::EntryCreated { location } if location == &Location::local("/fixture/new file")
    )));
    assert!(browser.current_operation.get().is_none());
}

#[test]
fn every_recent_spelling_is_rejected_by_creation_and_transfer_commands() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    for uri in ["recent:///", "recent://", "recent:///entry-id"] {
        let recent = Location::uri(uri);

        browser.create_new_folder(recent.clone());
        browser.create_new_file(recent.clone());
        browser.transfer(
            recent,
            vec![PasteItem {
                source: Location::local("/fixture/source.txt"),
                conflict: TransferConflict::FailIfExists,
            }],
            false,
            true,
        );

        assert!(events.borrow().is_empty(), "{uri} produced an operation");
        assert_eq!(browser.current_operation.get(), None, "{uri}");
    }
}

#[test]
fn large_restore_progress_defers_model_removal() {
    let browser = Browser::new(Rc::new(TrashFileSource));
    browser.navigate(Location::uri("trash:///"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = browser.begin_operation();
    browser.restoration_operation.set(true);
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::RestoreProgress {
        request_id,
        completed: 1,
        total: 3_000,
        restored_location: Some(Location::uri("trash:///item")),
    });

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::RestorationProgress {
            completed: 1,
            total: 3_000,
        }
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
    );
}

#[test]
fn cancellation_refreshes_an_affected_remote_root_and_its_open_descendants() {
    let enumerate_calls = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(CountingFileSource {
        enumerate_calls: enumerate_calls.clone(),
    }));
    let root = Location::uri("smb://host/share");
    browser.navigate(root.clone());
    browser.descend(0, Location::uri("smb://host/share/child"));
    assert_eq!(enumerate_calls.get(), 2);

    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let cancellations = Rc::new(Cell::new(0));
    let cancellations_for_handle = cancellations.clone();
    let request_id = browser.begin_operation();
    browser.deletion_operation.set(true);
    browser
        .operation_load
        .replace(Some(LoadHandle::new(move || {
            cancellations_for_handle.set(cancellations_for_handle.get() + 1);
        })));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    browser.cancel_file_operation();

    assert_eq!(cancellations.get(), 1);
    assert_eq!(browser.current_operation.get(), Some(request_id));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::DeletionFinished { .. }))
    );

    emit(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: vec![Location::uri("smb://host/share/completed")],
            failed: vec![Location::uri("smb://host/share/interrupted")],
            not_attempted: vec![Location::uri("smb://host/share/not-attempted")],
            affected_locations: HashSet::from([root]),
        },
    });

    assert_eq!(browser.current_operation.get(), None);
    assert_eq!(enumerate_calls.get(), 2);
    let affected_locations = events
        .borrow()
        .iter()
        .find_map(|event| match event {
            BrowserEvent::OperationCancelled {
                affected_locations, ..
            } => Some(affected_locations.clone()),
            _ => None,
        })
        .expect("cancellation event");
    browser.refresh_after_cancellation(&affected_locations);
    assert_eq!(enumerate_calls.get(), 4);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::DeletionFinished { succeeded: false }))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OperationCancelled {
            completed: 1,
            failed: 1,
            not_attempted: 1,
            ..
        }
    )));
}

#[test]
fn superseding_rename_emits_a_terminal_abandonment_event() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let cancelled = Rc::new(Cell::new(false));
    let cancelled_for_handle = cancelled.clone();
    let request_id = browser.begin_operation();
    browser.rename_operation.set(Some(request_id));
    browser
        .operation_load
        .replace(Some(LoadHandle::new(move || {
            cancelled_for_handle.set(true)
        })));

    let replacement = browser.begin_operation();

    assert!(cancelled.get());
    assert_eq!(browser.current_operation.get(), Some(replacement));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::RenameAbandoned { request_id: id } if *id == request_id
    )));
}

#[test]
fn cancelled_rename_emits_a_terminal_abandonment_event() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = browser.begin_operation();
    browser.rename_operation.set(Some(request_id));
    let emit = browser.operation_callback(request_id, true, HashSet::new());

    emit(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation::default(),
    });

    assert_eq!(browser.current_operation.get(), None);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::RenameAbandoned { request_id: id } if *id == request_id
    )));
}

#[test]
fn transfer_failure_reports_moves_completed_before_the_error() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    let emit = browser.operation_callback(request_id, false, HashSet::new());
    let completed = Location::local("/fixture/completed");

    emit(OperationEvent::TransferFailed {
        request_id,
        completed_locations: vec![completed.clone()],
        message: "injected failure".to_owned(),
    });

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::TransferFinished { moved_locations }
            if moved_locations == std::slice::from_ref(&completed)
    )));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OperationFailed { message } if message == "injected failure"
    )));
}

#[test]
fn cancelling_extraction_keeps_progress_until_the_worker_reports_cancellation() {
    let cancelled = Rc::new(Cell::new(false));
    let emit = Rc::new(RefCell::new(None));
    let request_id = Rc::new(Cell::new(None));
    let provider = Rc::new(HeldExtractProvider {
        cancelled: cancelled.clone(),
        emit: emit.clone(),
        request_id: request_id.clone(),
    });
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(provider);
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    let entry = FileEntry {
        location: Location::local("/fixture/archive.zip"),
        thumbnail_path: None,
        native_name: OsString::from("archive.zip"),
        display_name: "archive.zip".into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    };
    browser.extract(entry, Location::local("/fixture"), None);

    let request_id = request_id.get().expect("extract request");
    assert_eq!(browser.current_operation.get(), Some(request_id));
    browser.cancel_file_operation();

    assert!(cancelled.get());
    assert_eq!(browser.current_operation.get(), Some(request_id));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ArchiveCompleted { .. }))
    );

    let callback = emit.borrow().clone().expect("extract callback");
    callback(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: Vec::new(),
            failed: Vec::new(),
            not_attempted: vec![Location::local("/fixture/archive.zip")],
            affected_locations: HashSet::from([Location::local("/fixture")]),
        },
    });

    assert_eq!(browser.current_operation.get(), None);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ArchiveCompleted { select_name } if select_name.is_empty()
    )));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OperationCancelled {
            completed: 0,
            failed: 0,
            not_attempted: 1,
            ..
        }
    )));
}

#[test]
fn successful_transfers_reveal_actual_destination_names_only_without_navigation() {
    for moving in [false, true] {
        for navigate_away in [false, true] {
            let browser = Browser::new(Rc::new(FakeFileSource));
            let root = Location::local("/fixture");
            browser.navigate(root.clone());
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            browser.observe(move |event| observed.borrow_mut().push(event.clone()));
            let request_id = browser.begin_operation();
            browser.transfer_operation.set(Some(moving));
            let destination = Location::local("/fixture/archive");
            browser
                .transfer_destination
                .replace(Some(destination.clone()));
            if !moving {
                browser
                    .created_locations
                    .replace(vec![Location::local("/fixture/archive/report (copy).txt")]);
            }
            let emit = browser.operation_callback(request_id, false, HashSet::new());
            if navigate_away {
                browser.navigate(Location::local("/elsewhere"));
                browser.navigate(root);
            }
            emit(OperationEvent::Pasted {
                request_id,
                locations: vec![Location::local("/fixture/report.txt")],
            });
            let events = events.borrow();
            let reveals: Vec<_> = events
                .iter()
                .filter_map(|event| match event {
                    BrowserEvent::TransferReveal {
                        destination,
                        locations,
                    } => Some((destination, locations)),
                    _ => None,
                })
                .collect();
            if navigate_away {
                assert!(reveals.is_empty());
            } else {
                let name = if moving {
                    "report.txt"
                } else {
                    "report (copy).txt"
                };
                assert_eq!(
                    reveals,
                    vec![(
                        &destination,
                        &vec![Location::local(format!("/fixture/archive/{name}"))]
                    )]
                );
            }
        }
    }
}

#[test]
fn a_drop_onto_a_folder_does_not_navigate_into_it() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let root = Location::local("/fixture");
    browser.navigate(root);
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    let destination = Location::local("/fixture/archive");
    browser.transfer_destination.replace(Some(destination));
    browser.transfer_reveal.set(false);
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![Location::local("/fixture/report.txt")],
    });

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::TransferReveal { .. })),
        "a drop must move or copy the file without leaving the source listing"
    );
}

#[test]
fn failed_transfers_do_not_request_a_reveal() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());
    emit(OperationEvent::TransferFailed {
        request_id,
        completed_locations: Vec::new(),
        message: "Permission denied".to_owned(),
    });
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::TransferReveal { .. }))
    );
}

#[test]
fn create_and_rename_refresh_remote_columns_but_not_local_monitors() {
    for (location, remote) in [
        (Location::uri("smb://host/share"), true),
        (Location::local("/fixture"), false),
    ] {
        for create in [true, false] {
            let enumerate_calls = Rc::new(Cell::new(0));
            let browser = Browser::new(Rc::new(CountingFileSource {
                enumerate_calls: enumerate_calls.clone(),
            }));
            browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
            browser.navigate(location.clone());
            assert_eq!(enumerate_calls.get(), 1);

            if create {
                browser.create_directory_with_naming(
                    location.clone(),
                    "New Folder".to_owned(),
                    false,
                );
            } else {
                browser.rename(
                    FileEntry {
                        location: location
                            .child(OsStr::new("old-name.txt"))
                            .expect("rename target"),
                        native_name: "old-name.txt".into(),
                        thumbnail_path: None,
                        display_name: "old-name.txt".into(),
                        kind: EntryKind::File,
                        size: MetadataValue::Known(1),
                        modified_unix_seconds: MetadataValue::Unknown,
                        recent_unix_seconds: MetadataValue::Unknown,
                        is_hidden: false,
                        mode: MetadataValue::Unknown,
                        image_dimensions: MetadataValue::Unknown,
                        child_count: MetadataValue::Unknown,
                        duration_seconds: MetadataValue::Unknown,
                    },
                    "new-name.txt".to_owned(),
                );
            }

            assert_eq!(
                enumerate_calls.get(),
                if remote { 2 } else { 1 },
                "{}",
                if remote {
                    "a remote column has no live monitor, so it should be refreshed explicitly"
                } else {
                    "a local column already has a live file monitor; no extra refresh is needed"
                }
            );
        }
    }
}

#[test]
fn deletion_targets_the_entered_folder_when_the_child_has_no_selection() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));
    browser.select(0, 0);
    browser.descend(0, Location::local("/fixture/child"));

    let entries = browser.deletion_entries();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].location, Location::local("/fixture/child"));
}

#[test]
fn transfers_target_the_entered_folder_when_the_child_has_no_selection() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));
    browser.select(0, 0);
    browser.descend(0, Location::local("/fixture/child"));

    let entries = browser.transfer_entries();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].location, Location::local("/fixture/child"));
}

#[test]
fn completed_deletions_remove_entries_without_reloading_the_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.remove_deleted_locations(&[Location::local("/fixture/child")]);

    assert!(browser.entry_at(0, 0).is_none());
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesSpliced { splices, .. }
            if splices.iter().any(|splice| splice.removed == 1)
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );
}

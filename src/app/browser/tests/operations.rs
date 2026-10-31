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
        listable: MetadataValue::Unknown,
        recent_uri: None,
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
        deleted_locations: vec![first.location.clone()],
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
fn large_deletion_updates_sources_missing_from_the_monitor_batch_without_reloading() {
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

    assert_eq!(
        browser.column_snapshot(0).map(|column| column.count),
        Some(0)
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { depth: 0, .. }))
    );
    assert!(
        !events
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
        restored_location: Some(Location::local("/restored/first")),
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
        restored: Vec::new(),
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
                target_name: None,
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
fn cancellation_refreshes_only_an_affected_remote_folder_and_its_open_descendants() {
    let enumerate_calls = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(CountingFileSource {
        enumerate_calls: enumerate_calls.clone(),
    }));
    let affected = Location::uri("smb://host/share/child");
    browser.navigate(Location::uri("smb://host/share"));
    browser.descend(0, affected.clone());
    browser.descend(1, Location::uri("smb://host/share/child/grandchild"));
    assert_eq!(enumerate_calls.get(), 3);

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
            affected_locations: HashSet::from([affected]),
        },
    });

    assert_eq!(browser.current_operation.get(), None);
    assert_eq!(enumerate_calls.get(), 3);
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
    assert_eq!(enumerate_calls.get(), 5);
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
fn a_stopped_transfer_reloads_its_destination_once() {
    struct Destination {
        watched: bool,
        loads: Cell<usize>,
    }

    impl FileSource for Destination {
        fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
            Ok(())
        }

        fn enumerate(
            &self,
            request: DirectoryRequest,
            emit: Rc<dyn Fn(DirectoryEvent)>,
        ) -> LoadHandle {
            self.loads.set(self.loads.get() + 1);
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: None,
                can_delete: None,
            });
            LoadHandle::new(|| {})
        }

        fn watch(&self, _: Location, _: bool, _: WatchCallback) -> Option<LoadHandle> {
            self.watched.then(|| LoadHandle::new(|| {}))
        }
    }

    type Ending = fn(OperationRequestId) -> OperationEvent;
    let cancelled: Ending = |request_id| OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            affected_locations: HashSet::from([Location::uri("sftp://host/destination")]),
            ..CancelledOperation::default()
        },
    };
    let failed: Ending = |request_id| OperationEvent::Failed {
        request_id,
        message: "failed".to_owned(),
        password_failure: None,
    };
    let transfer_failed: Ending = |request_id| OperationEvent::TransferFailed {
        request_id,
        completed_locations: Vec::new(),
        message: "failed".to_owned(),
    };
    let rows = [
        ("cancelled", false, false, false, cancelled, 1),
        ("failed", false, false, false, failed, 1),
        ("transfer failed", false, false, false, transfer_failed, 1),
        (
            "transfer failed, watched",
            false,
            true,
            false,
            transfer_failed,
            0,
        ),
        (
            "transfer failed, rescanned",
            false,
            true,
            true,
            transfer_failed,
            1,
        ),
        (
            "cancelled in the background",
            true,
            false,
            false,
            cancelled,
            1,
        ),
        ("failed in the background", true, false, false, failed, 1),
        (
            "transfer failed in the background",
            true,
            false,
            false,
            transfer_failed,
            1,
        ),
    ];
    for (row, background, watched, rescanned, ending, reloads) in rows {
        let destination = Location::uri("sftp://host/destination");
        let source = Rc::new(Destination {
            watched,
            loads: Cell::new(0),
        });
        let browser = Browser::new(source.clone());
        let operations = Rc::new(crate::test_support::operations::HeldOperations::default());
        browser.set_operation_provider(operations.clone());
        browser.navigate(destination.clone());
        browser.transfer(
            destination.clone(),
            vec![PasteItem {
                source: Location::uri("sftp://host/source/item.txt"),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            false,
            false,
        );
        let id = browser.last_started_operation().expect("copy started");
        if background {
            assert!(browser.background_file_operation(id), "{row}");
        }
        if rescanned {
            browser.handle_directory_change(0, &destination, DirectoryChange::Rescan);
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        let loaded = source.loads.get();

        operations.emit(id, ending(id));
        for event in events.take() {
            match crate::test_support::operations::operation_event(&event) {
                BrowserEvent::OperationCancelled {
                    affected_locations, ..
                } => browser.refresh_after_cancellation(affected_locations),
                BrowserEvent::OperationRefreshRequired { depths } => {
                    browser.refresh_operation_columns(depths)
                }
                _ => {}
            }
        }

        assert_eq!(source.loads.get() - loaded, reloads, "{row}");
    }
}

#[test]
fn a_restore_reloads_the_open_folders_without_a_monitor_it_changed() {
    struct Watched(CountingFileSource);

    impl FileSource for Watched {
        fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
            self.0.validate_location(location)
        }

        fn enumerate(
            &self,
            request: DirectoryRequest,
            emit: Rc<dyn Fn(DirectoryEvent)>,
        ) -> LoadHandle {
            self.0.enumerate(request, emit)
        }

        fn watch(
            &self,
            _location: Location,
            _include_hidden: bool,
            _notify: Rc<dyn Fn(DirectoryChange)>,
        ) -> Option<LoadHandle> {
            Some(LoadHandle::new(|| {}))
        }
    }

    type Ending = fn(OperationRequestId, Vec<Location>) -> OperationEvent;
    let folder = Location::uri("smb://host/share/folder");
    let into_folder = Location::uri("smb://host/share/folder/restored.txt");
    let restored: Ending = |request_id, restored| OperationEvent::Restored {
        request_id,
        locations: vec![Location::uri("trash:///restored")],
        restored,
    };
    let restored_with_errors: Ending =
        |request_id, restored| OperationEvent::RestoreCompletedWithErrors {
            request_id,
            restored_locations: vec![Location::uri("trash:///restored")],
            restored,
            message: "one item failed".to_owned(),
        };
    let undone_with_errors: Ending = |request_id, _| OperationEvent::CompletedWithErrors {
        request_id,
        deleted_locations: Vec::new(),
        retryable_locations: Vec::new(),
        has_non_retryable_failures: true,
        message: "one item failed".to_owned(),
    };
    let rows = [
        (
            "restored into it",
            restored,
            vec![into_folder.clone()],
            None,
            false,
            1,
        ),
        (
            "restored into it with errors",
            restored_with_errors,
            vec![into_folder],
            None,
            false,
            1,
        ),
        (
            "restored it",
            restored,
            vec![folder.clone()],
            None,
            false,
            2,
        ),
        (
            "a merge undo in it restored nothing",
            undone_with_errors,
            Vec::new(),
            Some(folder.clone()),
            false,
            1,
        ),
        (
            "restored a watched folder",
            restored,
            vec![folder.clone()],
            None,
            true,
            0,
        ),
    ];
    for (row, ending, restored, undone_in, watched, reloaded) in rows {
        let enumerate_calls = Rc::new(Cell::new(0));
        let source = CountingFileSource {
            enumerate_calls: enumerate_calls.clone(),
        };
        let browser = if watched {
            Browser::new(Rc::new(Watched(source)))
        } else {
            Browser::new(Rc::new(source))
        };
        browser.navigate(Location::uri("smb://host/share"));
        browser.descend(0, folder.clone());
        let request_id = browser.begin_operation();
        browser.restoration_operation.set(true);
        let emit = browser.operation_callback(request_id, false, undone_in.into_iter().collect());

        emit(ending(request_id, restored));

        assert_eq!(enumerate_calls.get(), 2 + reloaded, "{row}");
    }
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
        BrowserEvent::OperationFailed { message, .. } if message == "injected failure"
    )));
}

#[test]
fn an_extraction_password_failure_reaches_the_view_with_its_kind() {
    for password_failure in [
        None,
        Some(PasswordFailure::Required),
        Some(PasswordFailure::Incorrect),
    ] {
        let browser = Browser::new(Rc::new(FakeFileSource));
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        let request_id = browser.begin_operation();

        browser.operation_callback(request_id, false, HashSet::new())(OperationEvent::Failed {
            request_id,
            message: "The password may be incorrect.".to_owned(),
            password_failure,
        });

        assert!(
            events.borrow().iter().any(|event| matches!(
                event,
                BrowserEvent::OperationFailed { password_failure: reported, .. }
                    if *reported == password_failure
            )),
            "{password_failure:?}"
        );
    }
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
        listable: MetadataValue::Unknown,
        recent_uri: None,
    };
    browser.extract(entry, Location::local("/fixture"), false, None);

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
            if moving {
                emit(OperationEvent::ItemMoved {
                    request_id,
                    from: Location::local("/fixture/report.txt"),
                    to: Location::local("/fixture/archive/report (1).txt"),
                    merged: false,
                });
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
                    "report (1).txt"
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
fn completed_operations_refresh_only_columns_without_a_monitor() {
    struct Watched(CountingFileSource);

    impl FileSource for Watched {
        fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
            self.0.validate_location(location)
        }

        fn enumerate(
            &self,
            request: DirectoryRequest,
            emit: Rc<dyn Fn(DirectoryEvent)>,
        ) -> LoadHandle {
            self.0.enumerate(request, emit)
        }

        fn watch(&self, _: Location, _: bool, _: WatchCallback) -> Option<LoadHandle> {
            Some(LoadHandle::new(|| {}))
        }
    }

    for (location, elsewhere, watched) in [
        (
            Location::uri("smb://host/share"),
            Location::uri("smb://host/elsewhere"),
            false,
        ),
        (
            Location::local("/fixture"),
            Location::local("/elsewhere"),
            false,
        ),
        (
            Location::local("/fixture"),
            Location::local("/elsewhere"),
            true,
        ),
    ] {
        for operation in ["create", "rename", "move away", "compress"] {
            let enumerate_calls = Rc::new(Cell::new(0));
            let source = CountingFileSource {
                enumerate_calls: enumerate_calls.clone(),
            };
            let browser = if watched {
                Browser::new(Rc::new(Watched(source)))
            } else {
                Browser::new(Rc::new(source))
            };
            browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
            browser.navigate(location.clone());
            assert_eq!(enumerate_calls.get(), 1);
            let item = FileEntry {
                location: location
                    .child(OsStr::new("old-name.txt"))
                    .expect("operation target"),
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
                listable: MetadataValue::Unknown,
                recent_uri: None,
            };

            match operation {
                "create" => browser.create_directory_with_naming(
                    location.clone(),
                    "New Folder".to_owned(),
                    false,
                ),
                "rename" => {
                    browser.rename(item, "new-name.txt".to_owned());
                }
                "move away" => browser.transfer(
                    elsewhere.clone(),
                    vec![PasteItem {
                        source: item.location,
                        conflict: TransferConflict::FailIfExists,
                        target_name: None,
                    }],
                    true,
                    false,
                ),
                _ => browser.compress(
                    vec![item],
                    location.clone(),
                    "archive".to_owned(),
                    TransferConflict::FailIfExists,
                    crate::services::ArchiveFormat::Zip,
                    None,
                ),
            }

            assert_eq!(
                enumerate_calls.get(),
                if watched { 1 } else { 2 },
                "{operation} in {location:?}, watched: {watched}"
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

    browser.remove_deleted_locations(&[Location::local("/fixture/child")], &[]);

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

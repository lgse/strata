// SPDX-License-Identifier: MIT

use super::*;
use crate::services::PlacedSnapshot;
use crate::test_support::operations::{HeldOperations, entry, operation_event};

fn copy(browser: &Rc<Browser>, name: &str) -> OperationRequestId {
    browser.transfer(
        Location::local("/fixture/destination"),
        vec![PasteItem {
            source: Location::local(format!("/fixture/{name}")),
            conflict: TransferConflict::FailIfExists,
            target_name: None,
        }],
        false,
        false,
    );
    browser.last_started_operation().expect("copy started")
}

fn complete_copy(operations: &HeldOperations, id: OperationRequestId, created: Location) {
    operations.emit(
        id,
        OperationEvent::TransferProgress {
            request_id: id,
            completed_items: 1,
            completed_files: 1,
            total_files: Some(1),
            current_file: Some(created.display_name()),
            transferred_bytes: 100,
            total_bytes: Some(100),
            created_location: Some(created.clone()),
        },
    );
    operations.emit(
        id,
        OperationEvent::Pasted {
            request_id: id,
            locations: vec![created],
        },
    );
}

#[test]
fn parked_operations_honor_cancellation_before_handle_installation() {
    for compression in [false, true] {
        let browser = Browser::new(Rc::new(FakeFileSource));
        let operations = Rc::new(HeldOperations::default());
        browser.set_operation_provider(operations.clone());
        let weak = Rc::downgrade(&browser);
        operations.before_return.replace(Some(Rc::new(move |id| {
            let browser = weak.upgrade().expect("browser retained by fixture");
            assert!(browser.background_file_operation(id));
            browser.cancel_operation(id);
        })));
        if compression {
            browser.compress(
                vec![entry(Location::local("/fixture/input.txt"))],
                Location::local("/fixture/destination"),
                "bundle.zip".into(),
                TransferConflict::FailIfExists,
                crate::services::ArchiveFormat::Zip,
                None,
            );
        } else {
            copy(&browser, "input.txt");
        }
        let id = browser.last_started_operation().expect("operation started");
        assert!(operations.cancelled(id));
        operations.before_return.take();
        copy(&browser, "next.txt");
        assert_eq!(browser.last_started_operation() == Some(id), !compression);
        operations.emit(
            id,
            OperationEvent::Cancelled {
                request_id: id,
                result: crate::services::CancelledOperation::default(),
            },
        );
        assert!(!browser.has_background_operations());
        if !compression {
            assert_ne!(copy(&browser, "next.txt"), id);
        }
    }
}

#[test]
fn background_completion_preserves_foreground_callbacks_and_ignores_late_events() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let operations = Rc::new(HeldOperations::default());
    browser.set_operation_provider(operations.clone());
    let first = copy(&browser, "first.txt");
    assert!(browser.background_file_operation(first));
    let second = copy(&browser, "second.txt");
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    complete_copy(
        &operations,
        first,
        Location::local("/fixture/destination/first.txt"),
    );
    assert!(browser.is_current_operation(second));
    assert!(!operations.cancelled(second));
    let count = events.borrow().len();
    operations.emit(
        first,
        OperationEvent::Failed {
            request_id: first,
            message: "Late failure".into(),
            password_failure: None,
        },
    );
    assert_eq!(events.borrow().len(), count);
    operations.emit(
        second,
        OperationEvent::Failed {
            request_id: second,
            message: "Foreground failure".into(),
            password_failure: None,
        },
    );
    assert!(events.borrow().iter().any(|event| matches!(event, BrowserEvent::OperationFailed { message, .. } if message == "Foreground failure")));
}

#[test]
fn background_copy_preserves_partial_redo_undo_and_remaining_items() {
    PENDING_UNDO.with(|pending| *pending.borrow_mut() = UndoState::default());
    let browser = Browser::new(Rc::new(FakeFileSource));
    let operations = Rc::new(HeldOperations::default());
    browser.set_operation_provider(operations.clone());
    let background = copy(&browser, "background.txt");
    assert!(browser.background_file_operation(background));
    let records = vec![
        MoveRecord {
            original: Location::local("/fixture/first.txt"),
            current: Location::local("/fixture/moved/first.txt"),
            renamed: Vec::new(),
        },
        MoveRecord {
            original: Location::local("/fixture/second.txt"),
            current: Location::local("/fixture/moved/second.txt"),
            renamed: Vec::new(),
        },
    ];
    push_pending_redo(UndoEntry::Move(records.clone()));
    let (generation, replay) = claim_replay(true, None).expect("redo claim");
    let foreground = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    browser.redo_claim.replace(Some((generation, replay)));
    let finish = browser.operation_callback(foreground, false, HashSet::new());
    complete_copy(
        &operations,
        background,
        Location::local("/fixture/destination/background.txt"),
    );
    finish(OperationEvent::Cancelled {
        request_id: foreground,
        result: crate::services::CancelledOperation {
            completed: vec![records[0].original.clone()],
            not_attempted: vec![records[1].original.clone()],
            ..Default::default()
        },
    });
    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![records[0].clone()]))
    );
    assert_eq!(
        pending_redo_entry(),
        Some(UndoEntry::Move(vec![records[1].clone()]))
    );
}

#[test]
fn background_history_pressure_retains_partial_undo_claims() {
    PENDING_UNDO.with(|pending| *pending.borrow_mut() = UndoState::default());
    let completed = Location::local("/fixture/first.txt");
    let remaining = Location::local("/fixture/second.txt");
    push_pending_undo(UndoEntry::Copy(vec![completed.clone(), remaining.clone()]));
    let (generation, _) = claim_pending_undo(None).expect("undo claim");
    mark_replay_item_completed(false, generation, &completed);
    let browser = Browser::new(Rc::new(FakeFileSource));
    let operations = Rc::new(HeldOperations::default());
    browser.set_operation_provider(operations.clone());
    for index in 0..MAX_UNDO_HISTORY + 1 {
        let id = copy(&browser, &format!("background-{index}.txt"));
        assert!(browser.background_file_operation(id));
        complete_copy(
            &operations,
            id,
            Location::local(format!("/fixture/destination/background-{index}.txt")),
        );
    }
    assert_eq!(
        completed_replay_items(false, generation),
        Some(UndoEntry::Copy(vec![completed]))
    );
    finish_undo(generation, false);
    PENDING_UNDO.with(|pending| {
        let pending = pending.borrow();
        assert!(
            pending
                .history
                .iter()
                .any(|entry| entry.generation == generation
                    && entry.entry == UndoEntry::Copy(vec![remaining.clone()])
                    && !entry.claimed)
        );
    });
}

#[test]
fn background_completion_preserves_grouped_undo_across_both_creation_windows() {
    for before_expect in [false, true] {
        PENDING_UNDO.with(|pending| *pending.borrow_mut() = UndoState::default());
        let browser = Browser::new(Rc::new(FakeFileSource));
        let operations = Rc::new(HeldOperations::default());
        browser.set_operation_provider(operations.clone());
        let id = copy(&browser, "background.txt");
        assert!(browser.background_file_operation(id));
        let folder = Location::local("/fixture/group");
        let background = Location::local("/fixture/destination/background.txt");
        push_pending_undo(UndoEntry::Copy(vec![folder.clone()]));
        if !before_expect {
            browser.expect_group_folder(folder.clone());
        }
        complete_copy(&operations, id, background.clone());
        if before_expect {
            browser.expect_group_folder(folder.clone());
        }
        let records = vec![MoveRecord {
            original: Location::local("/fixture/input.txt"),
            current: Location::local("/fixture/group/input.txt"),
            renamed: Vec::new(),
        }];
        push_pending_undo(UndoEntry::Move(records.clone()));
        assert_eq!(
            pending_undo_entry(),
            Some(UndoEntry::Group { folder, records })
        );
        PENDING_UNDO.with(|pending| {
            assert_eq!(
                pending.borrow().history[0].entry,
                UndoEntry::Copy(vec![background])
            )
        });
    }
}

#[test]
fn parked_delete_finishes_without_cancelling_foreground_and_respects_permanent_undo() {
    for permanent in [false, true] {
        PENDING_UNDO.with(|pending| *pending.borrow_mut() = UndoState::default());
        let browser = Browser::new(Rc::new(FakeFileSource));
        let operations = Rc::new(HeldOperations::default());
        browser.set_operation_provider(operations.clone());
        let deleted = Location::local("/fixture/deleted.txt");
        browser.delete(vec![entry(deleted.clone())], permanent);
        let id = browser.last_started_operation().expect("delete started");
        assert!(browser.background_file_operation(id));
        let foreground = copy(&browser, "other.txt");
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        let trashed = (!permanent).then(|| folder_identity(7, 1));
        if let Some(identity) = trashed {
            operations.emit(
                id,
                OperationEvent::ItemTrashed {
                    request_id: id,
                    location: deleted.clone(),
                    identity,
                },
            );
        }
        operations.emit(
            id,
            OperationEvent::DeleteProgress {
                request_id: id,
                completed: 1,
                total: 1,
                deleted_locations: vec![deleted.clone()],
            },
        );
        operations.emit(
            id,
            OperationEvent::Deleted {
                request_id: id,
                locations: vec![deleted.clone()],
            },
        );
        assert!(browser.is_current_operation(foreground));
        assert!(!operations.cancelled(foreground));
        assert_eq!(
            location_changes(&events.borrow()),
            [LocationChange::Removed {
                location: deleted.clone(),
                trash_identity: trashed,
            }]
        );
        assert_eq!(
            pending_undo_entry(),
            trashed.map(|identity| UndoEntry::Trash {
                locations: vec![deleted.clone()],
                originals: HashMap::from([(deleted, identity)]),
            })
        );
    }
}

#[test]
fn parking_deletion_flushes_its_changes_without_flushing_the_next_deletion() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let operations = Rc::new(HeldOperations::default());
    browser.set_operation_provider(operations.clone());
    let watched = Location::local("/fixture");
    let removed = batch_entry("removed");
    let remaining = batch_entry("remaining");
    {
        let mut state = browser.state.borrow_mut();
        state.navigate(watched.clone(), RequestId(1));
        let _ = state.apply_batch(RequestId(1), vec![removed.clone(), remaining.clone()]);
    }
    browser.delete(vec![removed.clone()], false);
    let background = browser
        .last_started_operation()
        .expect("background delete started");
    browser.handle_directory_change(
        0,
        &watched,
        DirectoryChange::Remove(removed.location.clone()),
    );
    assert_eq!(browser.column_snapshot(0).expect("watched column").count, 2);
    assert!(browser.background_file_operation(background));
    assert_eq!(
        browser
            .column_snapshot(0)
            .expect("parked deletion column")
            .count,
        1
    );
    browser.delete(vec![remaining.clone()], false);
    let foreground = browser
        .last_started_operation()
        .expect("foreground delete started");
    browser.handle_directory_change(
        0,
        &watched,
        DirectoryChange::Remove(remaining.location.clone()),
    );
    operations.emit(
        background,
        OperationEvent::Deleted {
            request_id: background,
            locations: vec![removed.location],
        },
    );
    assert_eq!(
        browser
            .column_snapshot(0)
            .expect("foreground deletion column")
            .count,
        1
    );
    assert!(browser.is_current_operation(foreground));
    operations.emit(
        foreground,
        OperationEvent::Deleted {
            request_id: foreground,
            locations: vec![remaining.location],
        },
    );
    assert_eq!(
        browser
            .column_snapshot(0)
            .expect("completed deletion column")
            .count,
        0
    );
}

fn renamed(name: &str) -> crate::services::TransferReport {
    crate::services::TransferReport {
        renamed: vec![crate::services::RenamedItem {
            requested: Location::local(format!("/fixture/destination/folder/{name}")),
            saved_as: Location::local(format!("/fixture/destination/folder/{name} (1)")),
            cause: crate::services::RenameCause::Alias,
        }],
        ..Default::default()
    }
}

fn reported(events: &[BrowserEvent]) -> Vec<(usize, &crate::services::TransferReport)> {
    events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| match operation_event(event) {
            BrowserEvent::ItemsReported { report }
            | BrowserEvent::OperationFailed { report, .. }
            | BrowserEvent::OperationCancelled { report, .. }
            | BrowserEvent::OperationCompletedWithErrors { report, .. }
                if !report.is_empty() =>
            {
                Some((index, report))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_transfer_reports_its_items_once_after_it_ends() {
    type Terminal = fn(OperationRequestId) -> OperationEvent;
    // Whether the ending's outcome carries the report, by whether it runs in the background.
    type Carries = fn(bool) -> bool;
    let terminals: [(Terminal, Carries); 7] = [
        (
            |request_id| OperationEvent::Pasted {
                request_id,
                locations: Vec::new(),
            },
            |_| false,
        ),
        (
            |request_id| OperationEvent::TransferFailed {
                request_id,
                completed_locations: Vec::new(),
                message: "Failed".into(),
            },
            |_| true,
        ),
        (
            |request_id| OperationEvent::Failed {
                request_id,
                message: "Failed".into(),
                password_failure: None,
            },
            |_| true,
        ),
        (
            |request_id| OperationEvent::Cancelled {
                request_id,
                result: crate::services::CancelledOperation::default(),
            },
            |_| true,
        ),
        (
            |request_id| OperationEvent::CompletedWithErrors {
                request_id,
                deleted_locations: Vec::new(),
                retryable_locations: Vec::new(),
                has_non_retryable_failures: true,
                message: "Failed".into(),
            },
            |_| true,
        ),
        (
            |request_id| OperationEvent::RestoreCompletedWithErrors {
                request_id,
                restored_locations: Vec::new(),
                restored: Vec::new(),
                message: "Failed".into(),
            },
            |_| true,
        ),
        (
            |request_id| OperationEvent::Restored {
                request_id,
                locations: Vec::new(),
                restored: Vec::new(),
            },
            |background| background,
        ),
    ];
    for background in [false, true] {
        for (terminal, carries) in terminals {
            let carried = carries(background);
            let browser = Browser::new(Rc::new(FakeFileSource));
            let operations = Rc::new(HeldOperations::default());
            browser.set_operation_provider(operations.clone());
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            browser.observe(move |event| observed.borrow_mut().push(event.clone()));
            let id = copy(&browser, "folder");
            let report = |name| OperationEvent::ItemsReported {
                request_id: id,
                report: renamed(name),
            };
            operations.emit(id, report("Makefile"));
            if background {
                assert!(browser.background_file_operation(id));
            }
            operations.emit(id, report("README"));
            let mut again = renamed("README (1)");
            again.renamed[0].saved_as = Location::local("/fixture/destination/folder/README (2)");
            again.renamed[0].cause = crate::services::RenameCause::Taken;
            operations.emit(
                id,
                OperationEvent::ItemsReported {
                    request_id: id,
                    report: again.clone(),
                },
            );
            assert!(reported(&events.borrow()).is_empty(), "reported early");

            let ending = terminal(id);
            let row = format!("{ending:?}, background: {background}");
            operations.emit(id, ending);

            let events = events.borrow();
            let mut expected = renamed("Makefile");
            let mut readme = renamed("README");
            readme.renamed[0].saved_as = again.renamed[0].saved_as.clone();
            expected.extend(readme);
            let reports = reported(&events);
            assert_eq!(
                reports
                    .iter()
                    .map(|(_, report)| *report)
                    .collect::<Vec<_>>(),
                [&expected],
                "{row}"
            );
            let ended = events.iter().position(|event| {
                matches!(
                    operation_event(event),
                    BrowserEvent::TransferCompleted
                        | BrowserEvent::OperationFailed { .. }
                        | BrowserEvent::OperationCancelled { .. }
                        | BrowserEvent::OperationCompletedWithErrors { .. }
                )
            });
            if carried {
                assert_eq!(ended, Some(reports[0].0), "{row}");
            } else {
                assert!(ended < Some(reports[0].0), "{row}");
            }
        }
    }
}

#[test]
fn a_background_merge_records_what_it_placed_for_undo() {
    PENDING_UNDO.with(|pending| *pending.borrow_mut() = UndoState::default());
    let browser = Browser::new(Rc::new(FakeFileSource));
    let operations = Rc::new(HeldOperations::default());
    browser.set_operation_provider(operations.clone());
    let id = copy(&browser, "folder");
    let created = Location::local("/fixture/destination/folder/new.txt");
    let overwritten = Location::local("/fixture/destination/folder/notes.txt");
    let placed = HashMap::from([
        (created.clone(), PlacedSnapshot::new(())),
        (overwritten.clone(), PlacedSnapshot::new(())),
    ]);
    let original = folder_identity(7, 1);
    assert!(browser.background_file_operation(id));

    operations.emit(
        id,
        OperationEvent::ItemTrashed {
            request_id: id,
            location: overwritten.clone(),
            identity: original,
        },
    );
    for (created, overwritten, placed) in [
        (
            vec![created.clone()],
            vec![overwritten.clone()],
            HashMap::new(),
        ),
        (Vec::new(), Vec::new(), placed.clone()),
    ] {
        operations.emit(
            id,
            OperationEvent::Merged {
                request_id: id,
                source: Location::local("/fixture/folder"),
                created,
                overwritten,
                placed,
            },
        );
    }
    operations.emit(
        id,
        OperationEvent::Pasted {
            request_id: id,
            locations: Vec::new(),
        },
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Merge {
            created: vec![created],
            overwritten: vec![overwritten.clone()],
            originals: HashMap::from([(overwritten, original)]),
            placed,
        })
    );
}

#[test]
fn a_background_merge_undo_removes_merged_items_under_the_names_they_were_saved_as() {
    PENDING_UNDO.with(|pending| *pending.borrow_mut() = UndoState::default());
    let browser = Browser::new(Rc::new(FakeFileSource));
    let operations = Rc::new(HeldOperations::default());
    browser.set_operation_provider(operations.clone());
    let id = copy(&browser, "folder");
    assert!(browser.background_file_operation(id));

    operations.emit(
        id,
        OperationEvent::Merged {
            request_id: id,
            source: Location::local("/fixture/folder"),
            created: vec![Location::local("/fixture/destination/folder/makefile")],
            overwritten: Vec::new(),
            placed: HashMap::new(),
        },
    );
    operations.emit(
        id,
        OperationEvent::ItemsReported {
            request_id: id,
            report: renamed("makefile"),
        },
    );
    operations.emit(
        id,
        OperationEvent::Pasted {
            request_id: id,
            locations: Vec::new(),
        },
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Merge {
            created: vec![Location::local("/fixture/destination/folder/makefile (1)")],
            overwritten: Vec::new(),
            originals: HashMap::new(),
            placed: HashMap::new(),
        })
    );
}

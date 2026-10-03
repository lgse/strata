// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::operations::{HeldOperations, entry};

fn copy(browser: &Rc<Browser>, name: &str) -> OperationRequestId {
    browser.transfer(
        Location::local("/fixture/destination"),
        vec![PasteItem {
            source: Location::local(format!("/fixture/{name}")),
            conflict: TransferConflict::FailIfExists,
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
        },
    );
    assert_eq!(events.borrow().len(), count);
    operations.emit(
        second,
        OperationEvent::Failed {
            request_id: second,
            message: "Foreground failure".into(),
        },
    );
    assert!(events.borrow().iter().any(|event| matches!(event, BrowserEvent::OperationFailed { message } if message == "Foreground failure")));
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
        },
        MoveRecord {
            original: Location::local("/fixture/second.txt"),
            current: Location::local("/fixture/moved/second.txt"),
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
        operations.emit(
            id,
            OperationEvent::DeleteProgress {
                request_id: id,
                completed: 1,
                total: 1,
                deleted_location: Some(deleted.clone()),
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
            pending_undo_entry(),
            if permanent {
                None
            } else {
                Some(UndoEntry::Trash(vec![deleted]))
            }
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

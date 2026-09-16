// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_completed_trash_operation_can_be_undone_once() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let location = Location::local("/fixture/report.txt");
    let entry = FileEntry {
        location: location.clone(),
        thumbnail_path: None,
        native_name: OsString::from("report.txt"),
        display_name: "report.txt".into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    };

    browser.delete(vec![entry], false);

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Trash(vec![location.clone()]))
    );
    assert!(browser.undo_last_trash());
    assert!(!browser.undo_last_trash());
}

#[test]
fn another_browser_can_undo_the_latest_trash_operation() {
    let deleting_browser = Browser::new(Rc::new(FakeFileSource));
    deleting_browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let undoing_browser = Browser::new(Rc::new(FakeFileSource));
    undoing_browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let entry = FileEntry {
        location: Location::local("/fixture/report.txt"),
        thumbnail_path: None,
        native_name: OsString::from("report.txt"),
        display_name: "report.txt".into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    };

    deleting_browser.delete(vec![entry], false);

    assert!(undoing_browser.undo_last_trash());
    assert!(!deleting_browser.undo_last_trash());
}

#[test]
fn a_completed_move_records_where_each_item_landed() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));

    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/report.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        true,
        true,
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![MoveRecord {
            original: Location::local("/fixture/report.txt"),
            current: Location::local("/fixture/archive/report.txt"),
        }]))
    );
}

#[test]
fn a_completed_copy_records_the_destinations_it_created() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));

    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/report.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        false,
        true,
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![Location::local(
            "/fixture/archive/report.txt"
        )]))
    );
}

#[test]
fn a_copy_that_created_nothing_leaves_the_previous_undo_current() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.delete(vec![fixture_entry("/fixture/report.txt")], false);
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![Location::local("/fixture/note.txt")],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Trash(vec![Location::local(
            "/fixture/report.txt"
        )]))
    );
}

#[test]
fn undoing_a_copy_removes_only_the_destinations_it_created() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_COPY_REQUESTS.with(|requests| requests.borrow_mut().clear());
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/report.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        false,
        true,
    );
    let (generation, locations) = browser.pending_undo_copy().expect("pending copy undo");

    assert!(browser.undo_copy(generation, locations));

    assert_eq!(
        UNDO_COPY_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![vec![Location::local("/fixture/archive/report.txt")]]
    );
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn undoing_a_copy_removes_the_destination_names_the_paste_reported() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_COPY_REQUESTS.with(|requests| requests.borrow_mut().clear());
    let generated = Location::local("/fixture/report (1).txt");
    let replaced = Location::local("/fixture/archive/note.txt");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());
    for created in [&generated, &replaced] {
        emit(OperationEvent::TransferProgress {
            request_id,
            completed_items: 1,
            transferred_bytes: 0,
            total_bytes: None,
            created_location: Some(created.clone()),
        });
    }
    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![
            Location::local("/fixture/report.txt"),
            Location::local("/fixture/note.txt"),
        ],
    });
    let (generation, locations) = browser.pending_undo_copy().expect("pending copy undo");

    assert!(browser.undo_copy(generation, locations));

    assert_eq!(
        UNDO_COPY_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![vec![generated, replaced]]
    );
}

#[test]
fn undoing_a_copy_leaves_the_previous_trash_undo_available() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.delete(vec![fixture_entry("/fixture/report.txt")], false);
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/note.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        false,
        true,
    );
    let (generation, locations) = browser.pending_undo_copy().expect("pending copy undo");

    assert!(browser.undo_copy(generation, locations));

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Trash(vec![Location::local(
            "/fixture/report.txt"
        )]))
    );
    assert!(browser.undo_last_trash());
}

#[test]
fn undoing_a_copy_records_no_trash_undo_of_its_own() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/report.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        false,
        true,
    );
    let (generation, locations) = browser.pending_undo_copy().expect("pending copy undo");

    assert!(browser.undo_copy(generation, locations));

    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn a_cancelled_copy_records_the_destinations_it_reached() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let created = Location::local("/fixture/archive/first.txt");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::TransferProgress {
        request_id,
        completed_items: 1,
        transferred_bytes: 0,
        total_bytes: None,
        created_location: Some(created.clone()),
    });
    emit(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: vec![Location::local("/fixture/first.txt")],
            failed: Vec::new(),
            not_attempted: vec![Location::local("/fixture/second.txt")],
            affected_locations: HashSet::new(),
        },
    });

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![created])));
}

#[test]
fn a_partial_copy_undo_keeps_the_destinations_still_to_remove() {
    let first = Location::local("/fixture/archive/first.txt");
    let second = Location::local("/fixture/archive/second.txt");
    push_pending_undo(UndoEntry::Copy(vec![first.clone(), second.clone()]));
    let (generation, _) = claim_pending_undo(None).expect("undo claim");

    mark_undo_item_completed(generation, &first);
    finish_undo(generation, false);

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![second])));
}

#[test]
fn the_undo_history_drops_its_oldest_entry_once_full() {
    for index in 0..MAX_UNDO_HISTORY + 1 {
        push_pending_undo(UndoEntry::Trash(vec![Location::local(format!(
            "/fixture/{index}.txt"
        ))]));
    }

    let retained = PENDING_UNDO.with(|pending| pending.borrow().history.len());

    assert_eq!(retained, MAX_UNDO_HISTORY);
    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Trash(vec![Location::local(format!(
            "/fixture/{}.txt",
            MAX_UNDO_HISTORY
        ))]))
    );
}

#[test]
fn a_completed_copy_displaces_an_older_trash_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.delete(vec![fixture_entry("/fixture/report.txt")], false);

    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/note.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        false,
        true,
    );

    assert!(!browser.undo_last_trash());
}

#[test]
fn a_move_into_the_items_own_directory_records_no_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));

    browser.transfer(
        Location::local("/fixture"),
        vec![PasteItem {
            source: Location::local("/fixture/report.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        true,
        true,
    );

    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn undoing_a_move_transfers_items_back_once() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_MOVE_REQUESTS.with(|requests| requests.borrow_mut().clear());
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/report.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        true,
        true,
    );
    let (generation, records) = browser.pending_undo_move().expect("pending move undo");

    assert!(
        browser.undo_move(
            generation,
            records
                .iter()
                .cloned()
                .map(|record| UndoMoveItem {
                    record,
                    conflict: TransferConflict::FailIfExists,
                })
                .collect(),
        )
    );

    assert_eq!(
        UNDO_MOVE_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![records]
    );
    assert_eq!(pending_undo_entry(), None);
    assert!(browser.pending_undo_move().is_none());
    assert!(!browser.undo_last_trash());
}

#[test]
fn an_undo_claim_from_an_earlier_operation_is_rejected() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let record = MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/archive/report.txt"),
    };
    push_pending_undo(UndoEntry::Move(vec![record.clone()]));
    let (stale_generation, _) = peek_pending_undo().expect("pending undo");
    push_pending_undo(UndoEntry::Trash(vec![Location::local("/fixture/note.txt")]));

    assert!(!browser.undo_move(
        stale_generation,
        vec![UndoMoveItem {
            record,
            conflict: TransferConflict::FailIfExists,
        }],
    ));
    assert!(browser.undo_last_trash());
}

#[test]
fn a_partial_move_undo_keeps_the_items_still_to_move_back() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let first = MoveRecord {
        original: Location::local("/fixture/first.txt"),
        current: Location::local("/fixture/archive/first.txt"),
    };
    let second = MoveRecord {
        original: Location::local("/fixture/second.txt"),
        current: Location::local("/fixture/archive/second.txt"),
    };
    push_pending_undo(UndoEntry::Move(vec![first.clone(), second.clone()]));
    let (generation, entry) = claim_pending_undo(None).expect("undo claim");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    browser.undo_claim.replace(Some((generation, entry)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::TransferFailed {
        request_id,
        completed_locations: vec![first.current.clone()],
        message: "injected failure".to_owned(),
    });

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Move(vec![second])));
}

#[test]
fn a_partial_move_undo_does_not_retry_items_excluded_before_transfer() {
    let skipped = MoveRecord {
        original: Location::local("/fixture/skipped.txt"),
        current: Location::local("/fixture/archive/skipped.txt"),
    };
    let completed = MoveRecord {
        original: Location::local("/fixture/completed.txt"),
        current: Location::local("/fixture/archive/completed.txt"),
    };
    let retryable = MoveRecord {
        original: Location::local("/fixture/retryable.txt"),
        current: Location::local("/fixture/archive/retryable.txt"),
    };
    push_pending_undo(UndoEntry::Move(vec![
        skipped,
        completed.clone(),
        retryable.clone(),
    ]));
    let (generation, _) = claim_pending_undo(None).expect("undo claim");
    let submitted = [completed.clone(), retryable.clone()].map(|record| UndoMoveItem {
        record,
        conflict: TransferConflict::FailIfExists,
    });

    retain_pending_move_items(generation, &submitted);
    mark_undo_item_completed(generation, &completed.current);
    finish_undo(generation, false);

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Move(vec![retryable])));
}

#[test]
fn undoing_a_move_leaves_a_pending_cut_untouched() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let record = MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/archive/report.txt"),
    };
    push_pending_undo(UndoEntry::Move(vec![record.clone()]));
    let (generation, entry) = claim_pending_undo(None).expect("undo claim");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    browser.undo_claim.replace(Some((generation, entry)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![record.current.clone()],
    });

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::TransferFinished { moved_locations } if moved_locations.is_empty()
    )));
}

#[test]
fn permanent_delete_preserves_the_previous_trash_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let trashed = FileEntry {
        location: Location::local("/fixture/report.txt"),
        thumbnail_path: None,
        native_name: OsString::from("report.txt"),
        display_name: "report.txt".into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    };
    let permanently_deleted = FileEntry {
        location: Location::local("/fixture/draft.txt"),
        native_name: OsString::from("draft.txt"),
        thumbnail_path: None,
        display_name: "draft.txt".into(),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        ..trashed.clone()
    };

    browser.delete(vec![trashed], false);
    browser.delete(vec![permanently_deleted], true);

    assert!(browser.undo_last_trash());
}

#[test]
fn failed_and_partial_undo_operations_can_be_retried() {
    let first = Location::local("/fixture/first.txt");
    let second = Location::local("/fixture/second.txt");
    push_pending_undo(UndoEntry::Trash(vec![first.clone(), second.clone()]));
    let (generation, _) = claim_pending_undo(None).expect("undo claim");

    mark_undo_item_completed(generation, &first);
    finish_undo(generation, false);

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Trash(vec![second.clone()]))
    );
    let (retry_generation, retry) = claim_pending_undo(None).expect("retry claim");
    assert_eq!(retry, UndoEntry::Trash(vec![second]));
    finish_undo(retry_generation, true);
    assert_eq!(pending_undo_entry(), None);
}

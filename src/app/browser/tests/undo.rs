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
        recent_unix_seconds: MetadataValue::Unknown,
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
fn pending_trash_undo_reports_original_locations_until_claimed() {
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
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    };

    browser.delete(vec![entry], false);

    assert_eq!(browser.pending_undo_trash(), Some(vec![location]));
    assert!(browser.pending_undo_trash().is_some());
    assert!(browser.undo_last_trash());
    assert_eq!(browser.pending_undo_trash(), None);
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
        recent_unix_seconds: MetadataValue::Unknown,
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

    mark_replay_item_completed(false, generation, &first);
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
    let (stale_generation, _) = peek_replay(false).expect("pending undo");
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

    retain_replay_move_items(false, generation, &submitted);
    mark_replay_item_completed(false, generation, &completed.current);
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
        recent_unix_seconds: MetadataValue::Unknown,
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

    mark_replay_item_completed(false, generation, &first);
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

fn rename_undo_entry() -> UndoEntry {
    let entry = fixture_entry("/fixture/original.txt");
    UndoEntry::Rename(RenameRecord {
        original: entry.location.clone(),
        current: Location::local("/fixture/renamed.txt"),
        native_name: entry.native_name,
        display_name: entry.display_name,
        is_hidden: entry.is_hidden,
    })
}

#[test]
fn rename_undo_records_the_exact_original_and_current_locations() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let entry = fixture_entry("/fixture/original.txt");

    browser.rename(entry.clone(), "renamed.txt".to_owned());

    let Some(UndoEntry::Rename(RenameRecord {
        original,
        current,
        native_name,
        display_name,
        is_hidden,
    })) = pending_undo_entry()
    else {
        panic!("expected a Rename undo entry");
    };
    assert_eq!(original, entry.location);
    assert_eq!(current, Location::local("/fixture/renamed.txt"));
    assert_eq!(native_name, entry.native_name);
    assert_eq!(display_name, entry.display_name);
    assert_eq!(is_hidden, entry.is_hidden);
}

#[test]
fn rename_undo_excludes_failed_cancelled_or_invalid_forward_rename() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let previous = UndoEntry::Trash(vec![Location::local("/fixture/previous.txt")]);
    push_pending_undo(previous.clone());
    let entry = fixture_entry("/fixture/original.txt");

    assert!(
        browser
            .rename(entry.clone(), "bad/name".to_owned())
            .is_none()
    );
    assert_eq!(pending_undo_entry(), Some(previous.clone()));

    FORWARD_RENAME_OUTCOME.with(|outcome| outcome.set(Some(ForwardRenameOutcome::Failed)));
    browser.rename(entry.clone(), "renamed.txt".to_owned());
    assert_eq!(pending_undo_entry(), Some(previous.clone()));

    FORWARD_RENAME_OUTCOME.with(|outcome| outcome.set(Some(ForwardRenameOutcome::Cancelled)));
    browser.rename(entry, "renamed.txt".to_owned());
    assert_eq!(pending_undo_entry(), Some(previous));
}

#[test]
fn rename_undo_rejects_a_stale_generation() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    push_pending_undo(rename_undo_entry());
    let (stale_generation, _, _) = browser.pending_undo_rename().expect("pending Rename undo");
    let newer = Location::local("/fixture/newer.txt");
    push_pending_undo(UndoEntry::Trash(vec![newer.clone()]));

    assert!(!browser.undo_rename(stale_generation));
    assert_eq!(pending_undo_entry(), Some(UndoEntry::Trash(vec![newer])));
}

#[test]
fn rename_undo_preserves_mixed_latest_operation_order() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.delete(vec![fixture_entry("/fixture/trashed.txt")], false);
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/moved.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        true,
        true,
    );
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/copied.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        false,
        true,
    );
    browser.rename(
        fixture_entry("/fixture/original.txt"),
        "renamed.txt".to_owned(),
    );

    let (rename_generation, _, _) = browser.pending_undo_rename().expect("Rename undo");
    assert!(browser.undo_rename(rename_generation));
    let (copy_generation, copied) = browser.pending_undo_copy().expect("Copy undo");
    assert!(browser.undo_copy(copy_generation, copied));
    let (move_generation, moved) = browser.pending_undo_move().expect("Move undo");
    assert!(
        browser.undo_move(
            move_generation,
            moved
                .iter()
                .cloned()
                .map(|record| UndoMoveItem {
                    record,
                    conflict: TransferConflict::FailIfExists,
                })
                .collect(),
        )
    );
    assert!(browser.undo_last_trash());
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn rename_undo_consumes_only_the_latest_rename_entry() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_RENAME_REQUESTS.with(|requests| requests.borrow_mut().clear());
    let older = Location::local("/fixture/older.txt");
    browser.delete(vec![fixture_entry("/fixture/older.txt")], false);
    let entry = fixture_entry("/fixture/original.txt");
    browser.rename(entry, "renamed.txt".to_owned());
    let (generation, current, original) =
        browser.pending_undo_rename().expect("pending Rename undo");

    assert!(browser.undo_rename(generation));
    assert_eq!(
        UNDO_RENAME_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![(current, original)]
    );
    assert_eq!(pending_undo_entry(), Some(UndoEntry::Trash(vec![older])));
}

#[test]
fn rename_undo_preserves_current_metadata_after_the_forward_rename() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let parent = Location::local("/fixture");
    let mut original = fixture_entry("/fixture/original");
    original.kind = EntryKind::Directory;
    {
        let mut state = browser.state.borrow_mut();
        state.navigate(parent.clone(), RequestId(1));
        state
            .apply_batch(RequestId(1), vec![original.clone()])
            .expect("initial directory batch");
    }

    browser.descend(0, original.location.clone());
    browser.rename(original.clone(), "renamed".to_owned());
    let current = Location::local("/fixture/renamed");
    assert!(browser.entry_at_location(&original.location).is_none());
    let mut updated = browser
        .entry_at_location(&current)
        .expect("forward rename published current entry");
    updated.size = MetadataValue::Known(99);
    updated.modified_unix_seconds = MetadataValue::Known(2);
    updated.mode = MetadataValue::Known(0o640);
    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(updated));
    let current_entry = browser
        .entry_at_location(&current)
        .expect("current entry after metadata update");
    assert_eq!(current_entry.location, current);
    assert_eq!(current_entry.size, MetadataValue::Known(99));
    assert_eq!(current_entry.modified_unix_seconds, MetadataValue::Known(2));
    assert_eq!(current_entry.mode, MetadataValue::Known(0o640));

    let (generation, _, _) = browser.pending_undo_rename().expect("pending Rename undo");
    assert!(browser.undo_rename(generation));

    let restored = browser
        .entry_at_location(&original.location)
        .expect("restored entry");
    assert_eq!(restored.location, original.location);
    assert_eq!(restored.size, MetadataValue::Known(99));
    assert_eq!(restored.modified_unix_seconds, MetadataValue::Known(2));
    assert_eq!(restored.mode, MetadataValue::Known(0o640));
}

#[test]
fn rename_undo_does_not_publish_historical_metadata_when_current_entry_is_unavailable() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let parent = Location::local("/fixture");
    let mut original = fixture_entry("/fixture/original");
    original.kind = EntryKind::Directory;
    original.size = MetadataValue::Known(7);
    original.modified_unix_seconds = MetadataValue::Known(1);
    original.mode = MetadataValue::Known(0o600);
    let current = Location::local("/fixture/renamed");

    browser.navigate(parent.clone());
    browser.handle_directory_change(
        0,
        &parent,
        DirectoryChange::Remove(Location::local("/fixture/child")),
    );
    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(original.clone()));
    browser.descend(0, original.location.clone());
    browser.rename(original.clone(), "renamed".to_owned());

    let mut updated = browser
        .entry_at_location(&current)
        .expect("forward rename published current entry");
    updated.size = MetadataValue::Known(99);
    updated.modified_unix_seconds = MetadataValue::Known(2);
    updated.mode = MetadataValue::Known(0o640);
    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(updated));
    assert_eq!(
        browser
            .entry_at_location(&current)
            .expect("current entry after metadata update")
            .size,
        MetadataValue::Known(99)
    );

    browser.refresh_column(0);
    assert!(browser.entry_at_location(&original.location).is_none());
    assert!(browser.entry_at_location(&current).is_none());
    assert_eq!(browser.location_at(1), Some(current.clone()));

    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let (generation, _, _) = browser.pending_undo_rename().expect("pending Rename undo");

    assert!(browser.undo_rename(generation));
    assert!(browser.entry_at_location(&original.location).is_none());
    assert_eq!(browser.location_at(1), Some(original.location.clone()));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
    );
}

#[test]
fn failed_rename_undo_releases_its_claim_and_remains_retryable() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let expected = rename_undo_entry();
    push_pending_undo(expected.clone());
    let (generation, claimed) = claim_pending_undo(None).expect("Rename undo claim");
    let request_id = browser.begin_operation();
    browser.undo_claim.replace(Some((generation, claimed)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Failed {
        request_id,
        message: "occupied".to_owned(),
    });

    assert_eq!(pending_undo_entry(), Some(expected));
    assert!(browser.pending_undo_rename().is_some());
}

#[test]
fn cancelled_rename_undo_releases_its_claim_and_remains_retryable() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let expected = rename_undo_entry();
    push_pending_undo(expected.clone());
    let (generation, claimed) = claim_pending_undo(None).expect("Rename undo claim");
    let request_id = browser.begin_operation();
    browser.undo_claim.replace(Some((generation, claimed)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation::default(),
    });

    assert_eq!(pending_undo_entry(), Some(expected));
    assert!(browser.pending_undo_rename().is_some());
}

#[test]
fn a_merged_copy_records_created_and_overwritten_paths_for_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_MERGE_REQUESTS.with(|requests| requests.borrow_mut().clear());
    let source = Location::local("/fixture/folder");
    let created = Location::local("/fixture/archive/folder/incoming.txt");
    let overwritten = Location::local("/fixture/archive/folder/shared.txt");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created: vec![created.clone()],
        overwritten: vec![overwritten.clone()],
    });
    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![source],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Merge {
            created: vec![created.clone()],
            overwritten: vec![overwritten.clone()],
            originals: HashMap::new(),
        })
    );
    let Some((generation, created, overwritten)) = browser.pending_undo_merge() else {
        panic!("expected a pending merge undo");
    };
    assert!(browser.undo_merge(generation, created.clone(), overwritten.clone()));
    assert_eq!(
        UNDO_MERGE_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![(created, overwritten, HashMap::new())]
    );
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn a_paste_mixing_plain_copies_and_a_merge_records_one_merge_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let copied = Location::local("/fixture/archive/report.txt");
    let merged_created = Location::local("/fixture/archive/folder/incoming.txt");
    let merged_overwritten = Location::local("/fixture/archive/folder/shared.txt");
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
        created_location: Some(copied.clone()),
    });
    emit(OperationEvent::Merged {
        request_id,
        source: Location::local("/fixture/folder"),
        created: vec![merged_created.clone()],
        overwritten: vec![merged_overwritten.clone()],
    });
    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![
            Location::local("/fixture/report.txt"),
            Location::local("/fixture/folder"),
        ],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Merge {
            created: vec![copied, merged_created],
            overwritten: vec![merged_overwritten],
            originals: HashMap::new(),
        })
    );
}

#[test]
fn a_merged_move_source_is_excluded_from_the_move_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let merged = Location::local("/fixture/folder");
    let moved = Location::local("/fixture/report.txt");
    let destination = Location::local("/fixture/archive");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    browser
        .transfer_destination
        .replace(Some(destination.clone()));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Merged {
        request_id,
        source: merged.clone(),
        created: vec![Location::local("/fixture/archive/folder/incoming.txt")],
        overwritten: vec![Location::local("/fixture/archive/folder/shared.txt")],
    });
    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![merged, moved.clone()],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![MoveRecord {
            original: moved,
            current: Location::local("/fixture/archive/report.txt"),
        }])),
        "a merged move deletes its source, so only the plain move can be undone"
    );
}

#[test]
fn a_cancelled_merge_still_records_the_staged_originals_for_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let source = Location::local("/fixture/folder");
    let overwritten = Location::local("/fixture/archive/folder/shared.txt");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created: Vec::new(),
        overwritten: vec![overwritten.clone()],
    });
    emit(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: Vec::new(),
            failed: vec![source.clone()],
            not_attempted: vec![Location::local("/fixture/second")],
            affected_locations: HashSet::new(),
        },
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Merge {
            created: Vec::new(),
            overwritten: vec![overwritten],
            originals: HashMap::new(),
        }),
        "originals staged before the cancelled copy still need restoring"
    );
}

#[test]
fn a_partial_merge_undo_keeps_the_paths_still_to_revert() {
    let first = Location::local("/fixture/archive/first.txt");
    let second = Location::local("/fixture/archive/second.txt");
    let originals = HashMap::from([(
        second.clone(),
        TrashedOriginal {
            device: 1,
            inode: 42,
        },
    )]);
    push_pending_undo(UndoEntry::Merge {
        created: vec![first.clone()],
        overwritten: vec![second.clone()],
        originals: originals.clone(),
    });
    let (generation, _) = claim_pending_undo(None).expect("undo claim");

    mark_replay_item_completed(false, generation, &first);
    finish_undo(generation, false);

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Merge {
            created: Vec::new(),
            overwritten: vec![second],
            originals,
        })
    );
}

#[test]
fn a_created_folder_can_be_undone_by_trashing_it() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_COPY_REQUESTS.with(|requests| requests.borrow_mut().clear());

    browser.create_new_folder(Location::local("/fixture"));

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![Location::local(
            "/fixture/new folder"
        )]))
    );
    let (generation, locations) = browser.pending_undo_copy().expect("pending copy undo");
    assert!(browser.undo_copy(generation, locations));
    assert_eq!(
        UNDO_COPY_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![vec![Location::local("/fixture/new folder")]]
    );
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn a_created_file_can_be_undone_by_trashing_it() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));

    browser.create_new_file(Location::local("/fixture"));

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![Location::local("/fixture/new file")]))
    );
}

#[test]
fn a_completed_compression_records_the_archive_for_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));

    browser.compress(
        vec![fixture_entry("/fixture/report.txt")],
        Location::local("/fixture"),
        "report.zip".to_owned(),
        TransferConflict::FailIfExists,
        ArchiveFormat::Zip,
        None,
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![Location::local(
            "/fixture/report.zip"
        )]))
    );
}

#[test]
fn an_undone_trash_operation_can_be_redone() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let location = Location::local("/fixture/report.txt");
    browser.delete(vec![fixture_entry("/fixture/report.txt")], false);

    assert!(browser.undo_last_trash());
    assert_eq!(pending_undo_entry(), None);
    assert_eq!(
        pending_redo_entry(),
        Some(UndoEntry::Trash(vec![location.clone()]))
    );

    let (generation, locations) = browser.pending_redo_trash().expect("pending redo trash");
    UNDO_COPY_REQUESTS.with(|requests| requests.borrow_mut().clear());
    assert!(browser.redo_trash(generation, locations));

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Trash(vec![location.clone()]))
    );
    UNDO_COPY_REQUESTS.with(|requests| {
        assert_eq!(&*requests.borrow(), &vec![vec![location]]);
    });
}

#[test]
fn a_new_operation_clears_the_redo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.delete(vec![fixture_entry("/fixture/report.txt")], false);
    assert!(browser.undo_last_trash());
    assert!(pending_redo_entry().is_some());

    browser.delete(vec![fixture_entry("/fixture/note.txt")], false);

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Trash(vec![Location::local("/fixture/note.txt")]))
    );
}

#[test]
fn an_undone_move_can_be_redone() {
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
    let record = MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/archive/report.txt"),
    };

    let (generation, records) = browser.pending_undo_move().expect("pending undo move");
    assert!(
        browser.undo_move(
            generation,
            records
                .into_iter()
                .map(|record| UndoMoveItem {
                    record,
                    conflict: TransferConflict::FailIfExists,
                })
                .collect()
        )
    );
    assert_eq!(
        pending_redo_entry(),
        Some(UndoEntry::Move(vec![record.clone()]))
    );

    let (generation, records) = browser.pending_redo_move().expect("pending redo move");
    UNDO_MOVE_REQUESTS.with(|requests| requests.borrow_mut().clear());
    assert!(
        browser.redo_move(
            generation,
            records
                .into_iter()
                .map(|record| UndoMoveItem {
                    record,
                    conflict: TransferConflict::FailIfExists,
                })
                .collect()
        )
    );

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![record.clone()]))
    );
    UNDO_MOVE_REQUESTS.with(|requests| {
        assert_eq!(
            &*requests.borrow(),
            &vec![vec![MoveRecord {
                original: record.current,
                current: record.original,
            }]]
        );
    });
}

#[test]
fn an_undone_copy_can_be_redone() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let created = Location::local("/fixture/archive/report.txt");
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/report.txt"),
            conflict: TransferConflict::FailIfExists,
        }],
        false,
        true,
    );

    let (generation, locations) = browser.pending_undo_copy().expect("pending undo copy");
    assert!(browser.undo_copy(generation, locations));
    assert_eq!(
        pending_redo_entry(),
        Some(UndoEntry::Copy(vec![created.clone()]))
    );

    let (generation, locations) = browser.pending_redo_copy().expect("pending redo copy");
    assert!(browser.redo_copy(generation, locations));

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![created])));
}

#[test]
fn a_put_back_can_be_retrashed_and_redone() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let destination = Location::local("/fixture/report.txt");
    let mut trash_entry = fixture_entry("/fixture/report.txt");
    trash_entry.location = Location::uri("trash:///report.txt");
    browser.restore(vec![RestoreTrashItem {
        entry: trash_entry,
        destination: PathBuf::from("/fixture/report.txt"),
    }]);

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![destination.clone()]))
    );
    let (generation, locations) = browser.pending_undo_copy().expect("pending undo copy");
    assert!(browser.undo_copy(generation, locations));
    assert_eq!(
        pending_redo_entry(),
        Some(UndoEntry::Copy(vec![destination.clone()]))
    );

    let (generation, locations) = browser.pending_redo_copy().expect("pending redo copy");
    assert!(browser.redo_copy(generation, locations));
    assert_eq!(pending_redo_entry(), None);
    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![destination]))
    );
}

#[test]
fn a_replaced_archive_restores_the_original_on_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_MERGE_REQUESTS.with(|requests| requests.borrow_mut().clear());
    let archive = Location::local("/fixture/report.zip");
    let original = TrashedOriginal {
        device: 1,
        inode: 42,
    };
    let request_id = browser.begin_operation();
    browser.archive_operation.set(true);
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Compressed {
        request_id,
        archive_name: "report.zip".to_owned(),
        archive: archive.clone(),
        original: Some(original),
    });

    let (generation, created, overwritten) =
        browser.pending_undo_merge().expect("replacement undo");
    assert!(created.is_empty());
    assert_eq!(overwritten, vec![archive.clone()]);
    assert!(browser.undo_merge(generation, created, overwritten));
    assert_eq!(
        UNDO_MERGE_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![(
            Vec::new(),
            vec![archive.clone()],
            HashMap::from([(archive, original)])
        )]
    );
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn a_completed_restore_records_the_restored_locations_for_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let restored = Location::local("/fixture/report.txt");
    let request_id = browser.begin_operation();
    browser.restoration_operation.set(true);
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Restored {
        request_id,
        locations: vec![Location::uri("trash:///report.txt")],
        restored: vec![restored.clone()],
    });

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![restored])));
}

#[test]
fn a_partially_completed_restore_records_only_the_restored_locations() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let restored = Location::local("/fixture/report.txt");
    let request_id = browser.begin_operation();
    browser.restoration_operation.set(true);
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::RestoreCompletedWithErrors {
        request_id,
        restored_locations: vec![Location::uri("trash:///report.txt")],
        restored: vec![restored.clone()],
        message: "one item failed".to_owned(),
    });

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![restored])));
}

#[test]
fn a_cancelled_restore_records_only_the_completed_restores() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let restored = Location::local("/fixture/report.txt");
    let request_id = browser.begin_operation();
    browser.restoration_operation.set(true);
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: vec![restored.clone()],
            failed: Vec::new(),
            not_attempted: vec![Location::local("/fixture/second.txt")],
            affected_locations: HashSet::new(),
        },
    });

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![restored])));
}

#[test]
fn undoing_a_trash_delete_does_not_record_the_restore_it_performed() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let trashed = Location::local("/fixture/report.txt");
    push_pending_undo(UndoEntry::Trash(vec![trashed.clone()]));
    let (generation, entry) = claim_pending_undo(None).expect("undo claim");
    let request_id = browser.begin_operation();
    browser.restoration_operation.set(true);
    browser.undo_claim.replace(Some((generation, entry)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Restored {
        request_id,
        locations: Vec::new(),
        restored: vec![trashed],
    });

    assert_eq!(
        pending_undo_entry(),
        None,
        "the restore performed by a trash undo must not record a new entry"
    );
}

#[test]
fn a_partial_undo_only_redoes_the_completed_items() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let first = Location::local("/fixture/first.txt");
    let second = Location::local("/fixture/second.txt");
    push_pending_undo(UndoEntry::Copy(vec![first.clone(), second.clone()]));
    let (generation, claimed) = claim_pending_undo(None).expect("undo claim");
    let request_id = browser.begin_operation();
    browser.undo_claim.replace(Some((generation, claimed)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::CompletedWithErrors {
        request_id,
        deleted_locations: vec![first.clone()],
        retryable_locations: Vec::new(),
        has_non_retryable_failures: true,
        message: "one copy could not be removed".into(),
    });

    assert_eq!(
        pending_redo_entry(),
        Some(UndoEntry::Copy(vec![first.clone()]))
    );
    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![second.clone()]))
    );
}

#[test]
fn a_non_successful_replay_with_all_items_applied_remains_redoable() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let location = Location::local("/fixture/report.txt");
    push_pending_undo(UndoEntry::Copy(vec![location.clone()]));
    let (generation, claimed) = claim_pending_undo(None).expect("undo claim");
    let request_id = browser.begin_operation();
    browser.undo_claim.replace(Some((generation, claimed)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::CompletedWithErrors {
        request_id,
        deleted_locations: vec![location.clone()],
        retryable_locations: Vec::new(),
        has_non_retryable_failures: true,
        message: "failure after deletion".into(),
    });

    assert_eq!(pending_undo_entry(), None);
    assert_eq!(pending_redo_entry(), Some(UndoEntry::Copy(vec![location])));
}

#[test]
fn a_replaced_copy_records_the_overwritten_original_for_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_MERGE_REQUESTS.with(|requests| requests.borrow_mut().clear());
    let source = Location::local("/fixture/report.txt");
    let target = Location::local("/fixture/archive/report.txt");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    // A replace reports its target through transfer progress like a plain
    // copy, then through Merged once the original is staged in Trash.
    emit(OperationEvent::TransferProgress {
        request_id,
        completed_items: 1,
        transferred_bytes: 0,
        total_bytes: None,
        created_location: Some(target.clone()),
    });
    emit(OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created: Vec::new(),
        overwritten: vec![target.clone()],
    });
    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![source],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Merge {
            created: Vec::new(),
            overwritten: vec![target.clone()],
            originals: HashMap::new(),
        }),
        "the replaced path must undo through the restore path, not be trashed"
    );
    let Some((generation, created, overwritten)) = browser.pending_undo_merge() else {
        panic!("expected a pending merge undo");
    };
    assert!(browser.undo_merge(generation, created.clone(), overwritten.clone()));
    assert_eq!(
        UNDO_MERGE_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![(created, overwritten, HashMap::new())]
    );
}

#[test]
fn a_replaced_move_keeps_the_move_undo_record() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let source = Location::local("/fixture/report.txt");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![source.clone()],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![MoveRecord {
            original: source,
            current: Location::local("/fixture/archive/report.txt"),
        }])),
        "a move-replace restores the moved item to its source; the original stays in Trash"
    );
}

#[test]
fn a_failed_undo_offers_no_redo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let location = Location::local("/fixture/report.txt");
    push_pending_undo(UndoEntry::Trash(vec![location.clone()]));
    let (generation, claimed) = claim_pending_undo(None).expect("undo claim");
    let request_id = browser.begin_operation();
    browser.undo_claim.replace(Some((generation, claimed)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Failed {
        request_id,
        message: "restore failed".into(),
    });

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(pending_undo_entry(), Some(UndoEntry::Trash(vec![location])));
}

#[test]
fn an_undone_rename_can_be_redone() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let expected = rename_undo_entry();
    push_pending_undo(expected.clone());

    let (generation, _, _) = browser.pending_undo_rename().expect("pending undo rename");
    assert!(browser.undo_rename(generation));
    assert_eq!(pending_redo_entry(), Some(expected.clone()));

    let (generation, _, _) = browser.pending_redo_rename().expect("pending redo rename");
    UNDO_RENAME_REQUESTS.with(|requests| requests.borrow_mut().clear());
    assert!(browser.redo_rename(generation));

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(pending_undo_entry(), Some(expected));
    UNDO_RENAME_REQUESTS.with(|requests| {
        assert_eq!(
            &*requests.borrow(),
            &vec![(
                Location::local("/fixture/original.txt"),
                Location::local("/fixture/renamed.txt"),
            )]
        );
    });
}

#[test]
fn a_failed_rename_undo_offers_no_redo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let expected = rename_undo_entry();
    push_pending_undo(expected.clone());
    let (generation, claimed) = claim_pending_undo(None).expect("undo claim");
    let request_id = browser.begin_operation();
    browser.undo_claim.replace(Some((generation, claimed)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Failed {
        request_id,
        message: "rename failed".into(),
    });

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(pending_undo_entry(), Some(expected));
}

#[test]
fn another_browser_can_redo_the_undone_operation() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let other = Browser::new(Rc::new(FakeFileSource));
    other.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let location = Location::local("/fixture/report.txt");
    browser.delete(vec![fixture_entry("/fixture/report.txt")], false);

    assert!(browser.undo_last_trash());
    let (generation, locations) = other.pending_redo_trash().expect("shared pending redo");
    assert!(other.redo_trash(generation, locations));

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(pending_undo_entry(), Some(UndoEntry::Trash(vec![location])));
}

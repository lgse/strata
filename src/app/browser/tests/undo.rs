// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{PlacedSnapshot, ReplacedOriginal};

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
        listable: MetadataValue::Unknown,
        recent_uri: None,
    };

    browser.delete(vec![entry], false);

    assert_eq!(
        pending_undo_entry(),
        Some(trash_undo(vec![location.clone()]))
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
        listable: MetadataValue::Unknown,
        recent_uri: None,
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
        listable: MetadataValue::Unknown,
        recent_uri: None,
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
            target_name: None,
        }],
        true,
        true,
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![MoveRecord {
            original: Location::local("/fixture/report.txt"),
            current: Location::local("/fixture/archive/report.txt"),
            renamed: Vec::new(),
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
            target_name: None,
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
        Some(trash_undo(vec![Location::local("/fixture/report.txt")]))
    );
}

#[test]
fn a_copy_into_an_admin_folder_records_no_trash_based_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.delete(vec![fixture_entry("/fixture/report.txt")], false);
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::uri("admin:///srv/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![Location::uri("admin:///srv/archive/note.txt")],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(trash_undo(vec![Location::local("/fixture/report.txt")]))
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
            target_name: None,
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
            completed_files: 1,
            total_files: None,
            current_file: None,
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
            target_name: None,
        }],
        false,
        true,
    );
    let (generation, locations) = browser.pending_undo_copy().expect("pending copy undo");

    assert!(browser.undo_copy(generation, locations));

    assert_eq!(
        pending_undo_entry(),
        Some(trash_undo(vec![Location::local("/fixture/report.txt")]))
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
            target_name: None,
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
        completed_files: 1,
        total_files: None,
        current_file: None,
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
        push_pending_undo(trash_undo(vec![Location::local(format!(
            "/fixture/{index}.txt"
        ))]));
    }

    let retained = PENDING_UNDO.with(|pending| pending.borrow().history.len());

    assert_eq!(retained, MAX_UNDO_HISTORY);
    assert_eq!(
        pending_undo_entry(),
        Some(trash_undo(vec![Location::local(format!(
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
            target_name: None,
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
            target_name: None,
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
            target_name: None,
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
        renamed: Vec::new(),
    };
    push_pending_undo(UndoEntry::Move(vec![record.clone()]));
    let (stale_generation, _) = peek_replay(false).expect("pending undo");
    push_pending_undo(trash_undo(vec![Location::local("/fixture/note.txt")]));

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
        renamed: Vec::new(),
    };
    let second = MoveRecord {
        original: Location::local("/fixture/second.txt"),
        current: Location::local("/fixture/archive/second.txt"),
        renamed: Vec::new(),
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
        renamed: Vec::new(),
    };
    let completed = MoveRecord {
        original: Location::local("/fixture/completed.txt"),
        current: Location::local("/fixture/archive/completed.txt"),
        renamed: Vec::new(),
    };
    let retryable = MoveRecord {
        original: Location::local("/fixture/retryable.txt"),
        current: Location::local("/fixture/archive/retryable.txt"),
        renamed: Vec::new(),
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
        renamed: Vec::new(),
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
        listable: MetadataValue::Unknown,
        recent_uri: None,
    };
    let permanently_deleted = FileEntry {
        location: Location::local("/fixture/draft.txt"),
        native_name: OsString::from("draft.txt"),
        thumbnail_path: None,
        display_name: "draft.txt".into(),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
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
    push_pending_undo(trash_undo(vec![first.clone(), second.clone()]));
    let (generation, _) = claim_pending_undo(None).expect("undo claim");

    mark_replay_item_completed(false, generation, &first);
    finish_undo(generation, false);

    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![second.clone()])));
    let (retry_generation, retry) = claim_pending_undo(None).expect("retry claim");
    assert_eq!(retry, trash_undo(vec![second]));
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
    let previous = trash_undo(vec![Location::local("/fixture/previous.txt")]);
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
    push_pending_undo(trash_undo(vec![newer.clone()]));

    assert!(!browser.undo_rename(stale_generation));
    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![newer])));
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
            target_name: None,
        }],
        true,
        true,
    );
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/copied.txt"),
            conflict: TransferConflict::FailIfExists,
            target_name: None,
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
    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![older])));
}

#[test]
fn rename_undo_preserves_current_metadata_after_the_forward_rename() {
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: Rc::default(),
    }));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let parent = Location::local("/fixture");
    let mut original = fixture_entry("/fixture/original");
    original.kind = EntryKind::Directory;
    browser.navigate(parent.clone());
    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(original.clone()));

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
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: Rc::default(),
    }));
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
        password_failure: None,
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

    let placed = HashMap::from([
        (created.clone(), PlacedSnapshot::new(())),
        (overwritten.clone(), PlacedSnapshot::new(())),
    ]);
    emit(OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created: vec![created.clone()],
        overwritten: vec![overwritten.clone()],
        placed: HashMap::new(),
    });
    emit(OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created: Vec::new(),
        overwritten: Vec::new(),
        placed: placed.clone(),
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
            placed: placed.clone(),
        })
    );
    let Some((generation, created, overwritten)) = browser.pending_undo_merge() else {
        panic!("expected a pending merge undo");
    };
    assert!(browser.undo_merge(generation, created.clone(), overwritten.clone()));
    assert_eq!(
        UNDO_MERGE_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![(created, overwritten, HashMap::new(), placed)]
    );
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn a_merge_undo_removes_merged_items_under_the_names_they_were_saved_as() {
    use crate::services::{RenameCause, RenamedItem};
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let source = Location::local("/fixture/folder");
    let created = Location::local("/fixture/archive/folder/makefile");
    let saved_as = Location::local("/fixture/archive/folder/makefile (1)");
    let overwritten = Location::local("/fixture/archive/folder/notes.txt");
    let numbered = Location::local("/fixture/archive/folder/notes (1).txt");
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
        placed: HashMap::new(),
    });
    emit(OperationEvent::ItemsReported {
        request_id,
        report: crate::services::TransferReport {
            renamed: vec![
                RenamedItem {
                    requested: created,
                    saved_as: saved_as.clone(),
                    cause: RenameCause::Alias,
                },
                RenamedItem {
                    requested: overwritten,
                    saved_as: numbered.clone(),
                    cause: RenameCause::Taken,
                },
            ],
            ..Default::default()
        },
    });
    let placed = HashMap::from([
        (saved_as.clone(), PlacedSnapshot::new(())),
        (numbered.clone(), PlacedSnapshot::new(())),
    ]);
    emit(OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created: Vec::new(),
        overwritten: Vec::new(),
        placed: placed.clone(),
    });
    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![source],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Merge {
            created: vec![saved_as, numbered],
            overwritten: Vec::new(),
            originals: HashMap::new(),
            placed,
        })
    );
}

#[test]
fn a_merge_undo_removes_a_fixed_name_the_destination_took_for_another_under_its_last_name() {
    use crate::services::{RenameCause, RenamedItem, TransferReport};
    let item = |requested: &str, saved_as: &str, cause| RenamedItem {
        requested: Location::local(format!("/fixture/archive/folder/{requested}")),
        saved_as: Location::local(format!("/fixture/archive/folder/{saved_as}")),
        cause,
    };
    // The merge planned `a_b` for `a:b`; the folder then took `a_b` for an
    // entry it already held, so the copy was saved as `a_b (1)`.
    let fixed = item("a:b", "a_b", RenameCause::Invalid);
    let alias = item("a_b", "a_b (1)", RenameCause::Alias);
    for renamed in [
        vec![fixed.clone(), alias.clone()],
        vec![alias.clone(), fixed.clone()],
    ] {
        let browser = Browser::new(Rc::new(FakeFileSource));
        browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
        let shown = Rc::new(RefCell::new(Vec::new()));
        let observed = shown.clone();
        browser.observe(move |event| {
            if let BrowserEvent::ItemsReported { report } = event {
                observed.borrow_mut().push(report.clone());
            }
        });
        let source = Location::local("/fixture/folder");
        let request_id = browser.begin_operation();
        browser.transfer_operation.set(Some(false));
        browser
            .transfer_destination
            .replace(Some(Location::local("/fixture/archive")));
        let emit = browser.operation_callback(request_id, false, HashSet::new());

        emit(OperationEvent::Merged {
            request_id,
            source: source.clone(),
            created: vec![fixed.saved_as.clone()],
            overwritten: Vec::new(),
            placed: HashMap::new(),
        });
        emit(OperationEvent::ItemsReported {
            request_id,
            report: TransferReport {
                renamed,
                ..Default::default()
            },
        });
        emit(OperationEvent::Pasted {
            request_id,
            locations: vec![source],
        });

        assert_eq!(
            pending_undo_entry(),
            Some(UndoEntry::Merge {
                created: vec![alias.saved_as.clone()],
                overwritten: Vec::new(),
                originals: HashMap::new(),
                placed: HashMap::new(),
            })
        );
        assert_eq!(
            *shown.borrow(),
            [TransferReport {
                renamed: vec![item("a:b", "a_b (1)", RenameCause::Invalid)],
                ..Default::default()
            }]
        );
    }
}

#[test]
fn a_move_undo_record_keeps_each_renamed_entry_once_beneath_the_item_it_moved() {
    use crate::services::{RenameCause, RenamedItem, TransferReport};
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let item = |requested: &str, saved_as: &str, cause| RenamedItem {
        requested: Location::local(format!("/fixture/archive/{requested}")),
        saved_as: Location::local(format!("/fixture/archive/{saved_as}")),
        cause,
    };
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::ItemMoved {
        request_id,
        from: Location::local("/fixture/pro:ject"),
        to: Location::local("/fixture/archive/project"),
        merged: false,
    });
    emit(OperationEvent::ItemsReported {
        request_id,
        report: TransferReport {
            renamed: vec![
                item("project/x:y", "project/x_y", RenameCause::Invalid),
                item("project/x_y/c_d", "project/x_y/c_d (1)", RenameCause::Alias),
                item("notes/e?f", "notes/e_f", RenameCause::Invalid),
                item("pro:ject", "project", RenameCause::Invalid),
                item("elsewhere/g:h", "elsewhere/g_h", RenameCause::Invalid),
                item("project/x_y/c:d", "project/x_y/c_d", RenameCause::Invalid),
            ],
            ..Default::default()
        },
    });
    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![
            Location::local("/fixture/pro:ject"),
            Location::local("/fixture/notes"),
        ],
    });

    let Some(UndoEntry::Move(records)) = pending_undo_entry() else {
        panic!("the move's undo");
    };
    assert_eq!(
        records
            .into_iter()
            .map(|record| (record.original, record.renamed))
            .collect::<Vec<_>>(),
        [
            (
                Location::local("/fixture/pro:ject"),
                vec![
                    item("project/x:y", "project/x_y", RenameCause::Invalid),
                    item(
                        "project/x_y/c:d",
                        "project/x_y/c_d (1)",
                        RenameCause::Invalid
                    ),
                ]
            ),
            (
                Location::local("/fixture/notes"),
                vec![item("notes/e?f", "notes/e_f", RenameCause::Invalid)]
            ),
        ]
    );
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
        completed_files: 1,
        total_files: None,
        current_file: None,
        transferred_bytes: 0,
        total_bytes: None,
        created_location: Some(copied.clone()),
    });
    let placed = HashMap::from([(copied.clone(), PlacedSnapshot::new(()))]);
    emit(OperationEvent::Merged {
        request_id,
        source: Location::local("/fixture/report.txt"),
        created: Vec::new(),
        overwritten: Vec::new(),
        placed: placed.clone(),
    });
    emit(OperationEvent::Merged {
        request_id,
        source: Location::local("/fixture/folder"),
        created: vec![merged_created.clone()],
        overwritten: vec![merged_overwritten.clone()],
        placed: HashMap::new(),
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
            placed,
        })
    );
}

#[test]
fn a_move_is_undone_from_where_the_item_went() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let source = Location::local("/fixture/report.txt");
    let kept_both = Location::local("/fixture/archive/report (1).txt");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::ItemMoved {
        request_id,
        from: source.clone(),
        to: kept_both.clone(),
        merged: false,
    });
    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![source.clone()],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![MoveRecord {
            original: source,
            current: kept_both,
            renamed: Vec::new(),
        }]))
    );
}

#[test]
fn a_move_undo_record_keeps_the_names_the_move_changed_inside_its_item() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let source = Location::local("/fixture/project");
    let alias = |folder: &str| crate::services::RenamedItem {
        requested: Location::local(format!("/fixture/archive/{folder}/makefile")),
        saved_as: Location::local(format!("/fixture/archive/{folder}/makefile (1)")),
        cause: crate::services::RenameCause::Alias,
    };
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/archive")));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::ItemsReported {
        request_id,
        report: crate::services::TransferReport {
            renamed: vec![alias("project"), alias("elsewhere")],
            ..Default::default()
        },
    });
    emit(OperationEvent::Pasted {
        request_id,
        locations: vec![source.clone()],
    });

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![MoveRecord {
            original: source,
            current: Location::local("/fixture/archive/project"),
            renamed: vec![alias("project")],
        }]))
    );
}

#[test]
fn an_item_whose_move_left_entries_behind_is_excluded_from_the_move_undo() {
    let project = Location::local("/fixture/project");
    let (zeta, alpha) = (
        Location::local("/fixture/zeta"),
        Location::local("/fixture/alpha"),
    );
    let moved = Location::local("/fixture/report.txt");
    use crate::services::{KeptCause, SpecialKind};
    // An entry the move left in the project, the project itself, a link left
    // out whole, which never moved, and entries left in two items, which are
    // named in the order they moved.
    for (left, cause, moving, kept_out_of_undo) in [
        (
            &["/fixture/project/notes.txt"][..],
            KeptCause::Changed,
            vec![project.clone()],
            vec![project.clone()],
        ),
        (
            &["/fixture/project"],
            KeptCause::Changed,
            vec![project.clone()],
            vec![project.clone()],
        ),
        (
            &["/fixture/link"],
            KeptCause::Skipped(SpecialKind::Symlink),
            Vec::new(),
            Vec::new(),
        ),
        (
            &["/fixture/alpha/a.txt", "/fixture/zeta/z.txt"],
            KeptCause::Changed,
            vec![zeta.clone(), alpha.clone()],
            vec![zeta.clone(), alpha.clone()],
        ),
    ] {
        let browser = Browser::new(Rc::new(FakeFileSource));
        browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
        let shown = Rc::new(RefCell::new(Vec::new()));
        let observed = shown.clone();
        browser.observe(move |event| {
            if let BrowserEvent::ItemsReported { report } = event {
                observed.borrow_mut().push(report.kept_out_of_undo.clone());
            }
        });
        let request_id = browser.begin_operation();
        browser.transfer_operation.set(Some(true));
        browser
            .transfer_destination
            .replace(Some(Location::local("/fixture/archive")));
        let emit = browser.operation_callback(request_id, false, HashSet::new());

        emit(OperationEvent::ItemsReported {
            request_id,
            report: crate::services::TransferReport {
                kept: left
                    .iter()
                    .map(|left| crate::services::KeptItem {
                        location: Location::local(left),
                        cause: cause.clone(),
                    })
                    .collect(),
                ..Default::default()
            },
        });
        let mut locations = moving;
        locations.push(moved.clone());
        emit(OperationEvent::Pasted {
            request_id,
            locations,
        });

        assert_eq!(
            pending_undo_entry(),
            Some(UndoEntry::Move(vec![MoveRecord {
                original: moved.clone(),
                current: Location::local("/fixture/archive/report.txt"),
                renamed: Vec::new(),
            }])),
            "moving the project back would replace the newer notes left in its folder"
        );
        assert_eq!(*shown.borrow(), [kept_out_of_undo], "{left:?}");
    }
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
        placed: HashMap::new(),
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
            renamed: Vec::new(),
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
        placed: HashMap::new(),
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
            placed: HashMap::new(),
        }),
        "originals staged before the cancelled copy still need restoring"
    );
}

#[test]
fn a_partial_merge_undo_keeps_the_paths_still_to_revert() {
    let first = Location::local("/fixture/archive/first.txt");
    let second = Location::local("/fixture/archive/second.txt");
    let originals = HashMap::from([(second.clone(), folder_identity(42, 1))]);
    let placed = HashMap::from([(second.clone(), PlacedSnapshot::new(()))]);
    push_pending_undo(UndoEntry::Merge {
        created: vec![first.clone()],
        overwritten: vec![second.clone()],
        originals: originals.clone(),
        placed: placed.clone(),
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
            placed,
        })
    );
}

#[test]
fn a_retried_merge_undo_keeps_the_records_of_what_it_created() {
    let created = Location::local("/fixture/archive/created.txt");
    let placed = HashMap::from([(created.clone(), PlacedSnapshot::new(()))]);
    let entry = UndoEntry::Merge {
        created: vec![created.clone()],
        overwritten: Vec::new(),
        originals: HashMap::new(),
        placed,
    };
    push_pending_undo(entry.clone());
    let (generation, _) = claim_pending_undo(None).expect("undo claim");

    retain_replay_merge_items(false, generation, std::slice::from_ref(&created), &[]);
    finish_undo(generation, false);

    assert_eq!(pending_undo_entry(), Some(entry));
}

#[test]
fn a_merge_undo_leaves_what_it_kept_out_of_undo_history() {
    let kept = Location::local("/fixture/archive/kept.txt");
    let cut = Location::local("/fixture/archive/cut.txt");
    let earlier = trash_undo(vec![Location::local("/fixture/earlier.txt")]);
    let rows = [
        (false, vec![kept.clone()], earlier.clone()),
        (
            true,
            vec![kept.clone(), cut.clone()],
            UndoEntry::Merge {
                created: vec![cut.clone()],
                overwritten: Vec::new(),
                originals: HashMap::new(),
                placed: HashMap::new(),
            },
        ),
    ];
    for (cancelled, created, expected) in rows {
        PENDING_UNDO.with(|pending| *pending.borrow_mut() = UndoState::default());
        let browser = Browser::new(Rc::new(FakeFileSource));
        browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        push_pending_undo(earlier.clone());
        push_pending_undo(UndoEntry::Merge {
            created,
            overwritten: Vec::new(),
            originals: HashMap::new(),
            placed: HashMap::new(),
        });
        let (generation, claimed) = claim_pending_undo(None).expect("undo claim");
        let request_id = browser.begin_operation();
        browser.restoration_operation.set(true);
        browser.undo_claim.replace(Some((generation, claimed)));
        let emit = browser.operation_callback(request_id, false, HashSet::new());

        emit(OperationEvent::ItemKept {
            request_id,
            location: kept.clone(),
        });
        emit(if cancelled {
            OperationEvent::Cancelled {
                request_id,
                result: crate::services::CancelledOperation {
                    failed: vec![kept.clone()],
                    not_attempted: vec![cut.clone()],
                    ..Default::default()
                },
            }
        } else {
            OperationEvent::CompletedWithErrors {
                request_id,
                deleted_locations: Vec::new(),
                retryable_locations: Vec::new(),
                has_non_retryable_failures: true,
                message: "kept.txt: It was kept".to_owned(),
            }
        });

        assert_eq!(pending_undo_entry(), Some(expected));
        assert!(
            !events.borrow().iter().any(|event| matches!(
                event,
                BrowserEvent::ItemRemoved { location, .. } if *location == kept
            )),
            "a kept item stays in view"
        );
    }
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
fn a_completed_extraction_records_created_entries_for_undo() {
    let emit = Rc::new(RefCell::new(None));
    let request_id = Rc::new(Cell::new(None));
    let provider = Rc::new(HeldExtractProvider {
        cancelled: Rc::new(Cell::new(false)),
        emit: emit.clone(),
        request_id: request_id.clone(),
    });
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(provider);
    browser.extract(
        fixture_entry("/fixture/archive.zip"),
        Location::local("/fixture"),
        false,
        None,
    );

    let request_id = request_id.get().expect("extract request");
    let callback = emit.borrow().clone().expect("extract callback");
    let created = vec![Location::local("/fixture/readme.txt")];
    callback(OperationEvent::Extracted {
        request_id,
        first_name: Some("readme.txt".into()),
        created: created.clone(),
    });

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(created)));
}

#[test]
fn an_empty_extraction_records_no_undo() {
    let emit = Rc::new(RefCell::new(None));
    let request_id = Rc::new(Cell::new(None));
    let provider = Rc::new(HeldExtractProvider {
        cancelled: Rc::new(Cell::new(false)),
        emit: emit.clone(),
        request_id: request_id.clone(),
    });
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(provider);
    browser.extract(
        fixture_entry("/fixture/archive.zip"),
        Location::local("/fixture"),
        false,
        None,
    );

    let request_id = request_id.get().expect("extract request");
    let callback = emit.borrow().clone().expect("extract callback");
    callback(OperationEvent::Extracted {
        request_id,
        first_name: None,
        created: Vec::new(),
    });

    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn an_undone_trash_operation_can_be_redone() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let location = Location::local("/fixture/report.txt");
    let (first, second) = (folder_identity(41, 1), folder_identity(42, 2));
    let trashed = |identity| {
        TRASHED_IDENTITIES
            .with(|trashed| trashed.replace(HashMap::from([(location.clone(), identity)])));
        UndoEntry::Trash {
            locations: vec![location.clone()],
            originals: HashMap::from([(location.clone(), identity)]),
        }
    };
    let deleted = trashed(first);
    browser.delete(vec![fixture_entry("/fixture/report.txt")], false);
    assert_eq!(pending_undo_entry(), Some(deleted.clone()));

    assert!(browser.undo_last_trash());
    assert_eq!(pending_undo_entry(), None);
    assert_eq!(pending_redo_entry(), Some(deleted));

    let (generation, locations) = browser.pending_redo_trash().expect("pending redo trash");
    UNDO_COPY_REQUESTS.with(|requests| requests.borrow_mut().clear());
    let redone = trashed(second);
    assert!(browser.redo_trash(generation, locations));
    TRASHED_IDENTITIES.with(|trashed| trashed.borrow_mut().clear());

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(pending_undo_entry(), Some(redone));
    let removed_as = |identity| LocationChange::Removed {
        location: location.clone(),
        trash_identity: Some(identity),
    };
    assert_eq!(
        location_changes(&events.borrow()),
        [
            removed_as(first),
            LocationChange::Restored(location.clone(), None),
            removed_as(second)
        ]
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
        Some(trash_undo(vec![Location::local("/fixture/note.txt")]))
    );
}

#[test]
fn an_undone_move_can_be_redone() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_MOVE_NAMES.with(|names| names.borrow_mut().clear());
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/report.txt"),
            conflict: TransferConflict::FailIfExists,
            target_name: None,
        }],
        true,
        true,
    );
    let record = MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/archive/report.txt"),
        renamed: Vec::new(),
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
    assert_eq!(
        location_changes(&events.borrow()),
        [
            relocated(&record.original, &record.current),
            relocated(&record.current, &record.original),
            relocated(&record.original, &record.current),
        ],
        "a move, its undo and its redo each relocate the item"
    );
    UNDO_MOVE_REQUESTS.with(|requests| {
        assert_eq!(
            &*requests.borrow(),
            &vec![vec![MoveRecord {
                original: record.current,
                current: record.original,
                renamed: Vec::new(),
            }]]
        );
    });
    assert_eq!(
        UNDO_MOVE_NAMES.with(|names| names.borrow().clone()),
        [NameMode::Keep, NameMode::Fix]
    );
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
            target_name: None,
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
fn a_replaced_archive_restores_the_original_on_undo_even_after_a_cancel_request() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let archive = Location::local("/fixture/report.zip");
    let original = folder_identity(42, 1);
    for cancel_requested in [false, true] {
        UNDO_MERGE_REQUESTS.with(|requests| requests.borrow_mut().clear());
        let request_id = browser.begin_operation();
        browser.archive_operation.set(true);
        let emit = browser.operation_callback(request_id, false, HashSet::new());
        if cancel_requested {
            browser.cancel_file_operation();
        }

        let placed = PlacedSnapshot::new(());
        emit(OperationEvent::Compressed {
            request_id,
            archive_name: "report.zip".to_owned(),
            archive: archive.clone(),
            original: Some(ReplacedOriginal::Trashed(Some(original))),
            placed: Some(placed.clone()),
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
                HashMap::from([(archive.clone(), original)]),
                HashMap::from([(archive.clone(), placed)]),
            )]
        );
        assert_eq!(pending_undo_entry(), None);
    }
}

#[test]
fn an_archive_that_replaced_its_original_without_trash_records_no_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let request_id = browser.begin_operation();
    browser.archive_operation.set(true);
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Compressed {
        request_id,
        archive_name: "report.zip".to_owned(),
        archive: Location::local("/fixture/report.zip"),
        original: Some(ReplacedOriginal::Deleted),
        placed: None,
    });

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
    push_pending_undo(trash_undo(vec![trashed.clone()]));
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
        completed_files: 1,
        total_files: None,
        current_file: None,
        transferred_bytes: 0,
        total_bytes: None,
        created_location: Some(target.clone()),
    });
    let placed = HashMap::from([(target.clone(), PlacedSnapshot::new(()))]);
    emit(OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created: Vec::new(),
        overwritten: vec![target.clone()],
        placed: placed.clone(),
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
            placed: placed.clone(),
        }),
        "the replaced path must undo through the restore path, not be trashed"
    );
    let Some((generation, created, overwritten)) = browser.pending_undo_merge() else {
        panic!("expected a pending merge undo");
    };
    assert!(browser.undo_merge(generation, created.clone(), overwritten.clone()));
    assert_eq!(
        UNDO_MERGE_REQUESTS.with(|requests| requests.borrow().clone()),
        vec![(created, overwritten, HashMap::new(), placed)]
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
            renamed: Vec::new(),
        }])),
        "a move-replace restores the moved item to its source; the original stays in Trash"
    );
}

#[test]
fn a_failed_undo_offers_no_redo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let location = Location::local("/fixture/report.txt");
    push_pending_undo(trash_undo(vec![location.clone()]));
    let (generation, claimed) = claim_pending_undo(None).expect("undo claim");
    let request_id = browser.begin_operation();
    browser.undo_claim.replace(Some((generation, claimed)));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::Failed {
        request_id,
        message: "restore failed".into(),
        password_failure: None,
    });

    assert_eq!(pending_redo_entry(), None);
    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![location])));
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
        password_failure: None,
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
    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![location])));
}

#[test]
fn a_completed_grouping_gesture_collapses_into_one_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let created = Location::local("/fixture/new folder");
    push_pending_undo(UndoEntry::Copy(vec![created.clone()]));
    browser.expect_group_folder(created.clone());
    push_pending_undo(UndoEntry::Move(vec![
        MoveRecord {
            original: Location::local("/fixture/report.txt"),
            current: Location::local("/fixture/new folder/report.txt"),
            renamed: Vec::new(),
        },
        MoveRecord {
            original: Location::local("/fixture/notes.txt"),
            current: Location::local("/fixture/new folder/notes.txt"),
            renamed: Vec::new(),
        },
    ]));

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Group {
            folder: created.clone(),
            records: vec![
                MoveRecord {
                    original: Location::local("/fixture/report.txt"),
                    current: Location::local("/fixture/new folder/report.txt"),
                    renamed: Vec::new(),
                },
                MoveRecord {
                    original: Location::local("/fixture/notes.txt"),
                    current: Location::local("/fixture/new folder/notes.txt"),
                    renamed: Vec::new(),
                },
            ],
        })
    );

    push_pending_undo(UndoEntry::Rename(RenameRecord {
        original: created.clone(),
        current: Location::local("/fixture/grouped"),
        native_name: OsString::from("new folder"),
        display_name: "new folder".to_owned(),
        is_hidden: false,
    }));

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Group {
            folder: Location::local("/fixture/grouped"),
            records: vec![
                MoveRecord {
                    original: Location::local("/fixture/report.txt"),
                    current: Location::local("/fixture/grouped/report.txt"),
                    renamed: Vec::new(),
                },
                MoveRecord {
                    original: Location::local("/fixture/notes.txt"),
                    current: Location::local("/fixture/grouped/notes.txt"),
                    renamed: Vec::new(),
                },
            ],
        })
    );
}

#[test]
fn dismissing_the_gesture_rename_leaves_a_later_folder_rename_separate() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let created = Location::local("/fixture/new folder");
    push_pending_undo(UndoEntry::Copy(vec![created.clone()]));
    browser.expect_group_folder(created.clone());
    push_pending_undo(UndoEntry::Move(vec![MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/new folder/report.txt"),
        renamed: Vec::new(),
    }]));
    assert!(matches!(
        pending_undo_entry(),
        Some(UndoEntry::Group { .. })
    ));

    browser.clear_group_folder();
    let rename = UndoEntry::Rename(RenameRecord {
        original: created.clone(),
        current: Location::local("/fixture/grouped"),
        native_name: OsString::from("new folder"),
        display_name: "new folder".to_owned(),
        is_hidden: false,
    });
    push_pending_undo(rename.clone());

    assert_eq!(pending_undo_entry(), Some(rename));
}

#[test]
fn group_folding_leaves_an_interleaved_entry_untouched() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let created = Location::local("/fixture/new folder");
    push_pending_undo(UndoEntry::Copy(vec![created.clone()]));
    browser.expect_group_folder(created.clone());
    push_pending_undo(UndoEntry::Move(vec![MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/new folder/report.txt"),
        renamed: Vec::new(),
    }]));
    let interleaved = trash_undo(vec![Location::local("/fixture/elsewhere.txt")]);
    push_pending_undo(interleaved.clone());

    assert_eq!(pending_undo_entry(), Some(interleaved));
    let (generation, _) = claim_pending_undo(None).expect("undo claim");
    finish_undo(generation, true);
    assert!(matches!(
        pending_undo_entry(),
        Some(UndoEntry::Group { .. })
    ));
}

#[test]
fn group_folding_ignores_moves_into_another_folder() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let created = Location::local("/fixture/new folder");
    let unrelated = UndoEntry::Move(vec![MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/archive/report.txt"),
        renamed: Vec::new(),
    }]);
    push_pending_undo(UndoEntry::Copy(vec![created.clone()]));
    browser.expect_group_folder(created.clone());
    push_pending_undo(unrelated.clone());

    assert_eq!(pending_undo_entry(), Some(unrelated));
    let (generation, _) = claim_pending_undo(None).expect("undo claim");
    finish_undo(generation, true);
    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![created.clone()]))
    );
}

#[test]
fn undoing_a_group_moves_back_and_trashes_the_folder_in_one_operation() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_MOVE_REQUESTS.with(|requests| requests.borrow_mut().clear());
    UNDO_MOVE_CLEANUPS.with(|requests| requests.borrow_mut().clear());
    UNDO_MOVE_NAMES.with(|names| names.borrow_mut().clear());
    let created = Location::local("/fixture/new folder");
    push_pending_undo(UndoEntry::Copy(vec![created.clone()]));
    browser.expect_group_folder(created.clone());
    push_pending_undo(UndoEntry::Move(vec![MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/new folder/report.txt"),
        renamed: Vec::new(),
    }]));
    let (generation, records) = browser.pending_undo_group().expect("pending group undo");

    assert!(
        browser.undo_group(
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
    assert_eq!(
        UNDO_MOVE_CLEANUPS.with(|requests| requests.borrow().clone()),
        vec![vec![created]]
    );
    assert_eq!(
        UNDO_MOVE_NAMES.with(|names| names.borrow().clone()),
        [NameMode::Keep]
    );
    assert_eq!(pending_undo_entry(), None);
    assert!(peek_replay(true).is_none(), "a group undo offers no redo");
}

#[test]
fn failed_or_cancelled_group_cleanup_remains_undoable() {
    for cancelled in [false, true] {
        let browser = Browser::new(Rc::new(FakeFileSource));
        browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
        let folder = Location::local("/fixture/grouped");
        let record = MoveRecord {
            original: Location::local("/fixture/report.txt"),
            current: Location::local("/fixture/grouped/report.txt"),
            renamed: Vec::new(),
        };
        push_pending_undo(UndoEntry::Group {
            folder: folder.clone(),
            records: vec![record.clone()],
        });
        let (generation, entry) = claim_pending_undo(None).expect("undo claim");
        let request_id = browser.begin_operation();
        browser.transfer_operation.set(Some(true));
        browser.undo_claim.replace(Some((generation, entry)));
        let emit = browser.operation_callback(request_id, false, HashSet::new());
        if cancelled {
            emit(OperationEvent::Cancelled {
                request_id,
                result: CancelledOperation {
                    completed: vec![record.current],
                    ..Default::default()
                },
            });
        } else {
            emit(OperationEvent::TransferFailed {
                request_id,
                completed_locations: vec![record.current],
                message: "injected cleanup failure".to_owned(),
            });
        }
        assert_eq!(
            pending_undo_entry(),
            Some(UndoEntry::Copy(vec![folder.clone()]))
        );
        UNDO_COPY_REQUESTS.with(|requests| requests.borrow_mut().clear());
        let (generation, locations) = browser.pending_undo_copy().expect("cleanup undo");
        assert!(browser.undo_copy(generation, locations));
        assert_eq!(
            UNDO_COPY_REQUESTS.with(|requests| requests.borrow().clone()),
            vec![vec![folder]]
        );
        assert_eq!(pending_undo_entry(), None);
    }
}

#[test]
fn skipping_a_grouped_item_does_not_trash_its_folder() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_MOVE_CLEANUPS.with(|requests| requests.borrow_mut().clear());
    let moved = MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/grouped/report.txt"),
        renamed: Vec::new(),
    };
    push_pending_undo(UndoEntry::Group {
        folder: Location::local("/fixture/grouped"),
        records: vec![
            moved.clone(),
            MoveRecord {
                original: Location::local("/fixture/skipped.txt"),
                current: Location::local("/fixture/grouped/skipped.txt"),
                renamed: Vec::new(),
            },
        ],
    });
    let (generation, _) = browser.pending_undo_group().expect("group undo");
    assert!(browser.undo_group(
        generation,
        vec![UndoMoveItem {
            record: moved,
            conflict: TransferConflict::FailIfExists,
        }]
    ));
    assert_eq!(
        UNDO_MOVE_CLEANUPS.with(|requests| requests.borrow().clone()),
        vec![Vec::<Location>::new()]
    );
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn a_missing_grouped_item_keeps_the_folder_cleanup() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    UNDO_MOVE_CLEANUPS.with(|requests| requests.borrow_mut().clear());
    let folder = Location::local("/fixture/grouped");
    let moved = MoveRecord {
        original: Location::local("/fixture/report.txt"),
        current: Location::local("/fixture/grouped/report.txt"),
        renamed: Vec::new(),
    };
    push_pending_undo(UndoEntry::Group {
        folder: folder.clone(),
        records: vec![
            moved.clone(),
            MoveRecord {
                original: Location::local("/fixture/gone.txt"),
                current: Location::local("/fixture/grouped/gone.txt"),
                renamed: Vec::new(),
            },
        ],
    });
    let (generation, _) = browser.pending_undo_group().expect("group undo");
    assert!(browser.undo_group_retaining_folder(
        generation,
        vec![UndoMoveItem {
            record: moved,
            conflict: TransferConflict::FailIfExists,
        }]
    ));
    assert_eq!(
        UNDO_MOVE_CLEANUPS.with(|requests| requests.borrow().clone()),
        vec![vec![folder]]
    );
}

#[test]
fn an_all_missing_group_leaves_its_folder_undoable() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let folder = Location::local("/fixture/grouped");
    push_pending_undo(UndoEntry::Group {
        folder: folder.clone(),
        records: vec![MoveRecord {
            original: Location::local("/fixture/gone.txt"),
            current: Location::local("/fixture/grouped/gone.txt"),
            renamed: Vec::new(),
        }],
    });
    let (generation, _) = browser.pending_undo_group().expect("group undo");
    browser.discard_group_keep_folder(false, generation);
    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![folder])));
}

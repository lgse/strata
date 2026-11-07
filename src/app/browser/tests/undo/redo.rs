// SPDX-License-Identifier: MIT

use super::*;

fn assert_undo_and_redo(undo: Option<UndoEntry>, redo: Option<UndoEntry>) {
    assert_eq!((pending_undo_entry(), pending_redo_entry()), (undo, redo));
}

/// Undoes the pending copy of `created`, redoes it, and checks it is undoable again.
fn assert_the_copy_undo_round_trips(browser: &Rc<Browser>, created: &Location) {
    let copy = UndoEntry::Copy(vec![created.clone()]);
    assert!(undo_pending_copy(browser));
    assert_eq!(pending_redo_entry(), Some(copy.clone()));

    let (generation, locations) = browser.pending_redo_copy().expect("pending redo copy");
    assert!(browser.redo_copy(generation, locations));
    assert_undo_and_redo(Some(copy), None);
}

/// A copy undo that removed only `deleted` before reporting errors.
fn replay_copy_undo_removing(
    locations: Vec<Location>,
    deleted: Vec<Location>,
    message: &str,
) -> Rc<Browser> {
    let browser = operating_browser();
    let (request_id, emit) =
        start_replay(&browser, UndoEntry::Copy(locations), OperationSetup::Plain);

    emit(OperationEvent::CompletedWithErrors {
        request_id,
        deleted_locations: deleted,
        retryable_locations: Vec::new(),
        has_non_retryable_failures: true,
        message: message.into(),
    });
    browser
}

fn assert_a_failed_undo_offers_no_redo(entry: UndoEntry, message: &str) {
    let browser = operating_browser();
    let (request_id, emit) = start_replay(&browser, entry.clone(), OperationSetup::Plain);

    emit(failed(request_id, message));

    assert_undo_and_redo(Some(entry), None);
}

#[test]
fn an_undone_trash_operation_can_be_redone() {
    let browser = operating_browser();
    let events = record_events(&browser);
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
    trash_report(&browser);
    assert_eq!(pending_undo_entry(), Some(deleted.clone()));

    assert!(browser.undo_last_trash());
    assert_undo_and_redo(None, Some(deleted));

    let (generation, locations) = browser.pending_redo_trash().expect("pending redo trash");
    forget_requests(&UNDO_COPY_REQUESTS);
    let redone = trashed(second);
    assert!(browser.redo_trash(generation, locations));
    TRASHED_IDENTITIES.with(|trashed| trashed.borrow_mut().clear());

    assert_undo_and_redo(Some(redone), None);
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
    assert_eq!(requests_in(&UNDO_COPY_REQUESTS), vec![vec![location]]);
}

#[test]
fn a_new_operation_clears_the_redo() {
    let browser = operating_browser();
    trash_report(&browser);
    assert!(browser.undo_last_trash());
    assert!(pending_redo_entry().is_some());

    browser.delete(vec![fixture_entry("/fixture/note.txt")], false);

    assert_undo_and_redo(
        Some(trash_undo(vec![Location::local("/fixture/note.txt")])),
        None,
    );
}

#[test]
fn an_undone_move_can_be_redone() {
    let browser = operating_browser();
    forget_requests(&UNDO_MOVE_NAMES);
    let events = record_events(&browser);
    transfer_one(&browser, "/fixture/archive", "/fixture/report.txt", true);
    let record = move_record("/fixture/report.txt", "/fixture/archive/report.txt");
    let moved = UndoEntry::Move(vec![record.clone()]);

    assert!(undo_pending_move(&browser));
    assert_eq!(pending_redo_entry(), Some(moved.clone()));

    let (generation, records) = browser.pending_redo_move().expect("pending redo move");
    forget_requests(&UNDO_MOVE_REQUESTS);
    assert!(browser.redo_move(generation, undo_move_items(records)));

    assert_undo_and_redo(Some(moved), None);
    assert_eq!(
        location_changes(&events.borrow()),
        [
            relocated(&record.original, &record.current),
            relocated(&record.current, &record.original),
            relocated(&record.original, &record.current),
        ],
        "a move, its undo and its redo each relocate the item"
    );
    assert_eq!(
        (
            requests_in(&UNDO_MOVE_REQUESTS),
            requests_in(&UNDO_MOVE_NAMES)
        ),
        (
            vec![vec![MoveRecord {
                original: record.current,
                current: record.original,
                renamed: Vec::new(),
                replaced: None,
            }]],
            vec![NameMode::Keep, NameMode::Fix]
        )
    );
}

fn finish_move_replay(
    browser: &Rc<Browser>,
    redo: bool,
    trashed: Option<(&Location, ItemIdentity)>,
    moved: &Location,
) {
    let claim = claim_replay(redo, None).expect("replay claim");
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    browser.replay_claim(redo).replace(Some(claim));
    let emit = browser.operation_callback(request_id, false, HashSet::new());
    if let Some((location, identity)) = trashed {
        emit(OperationEvent::ItemTrashed {
            request_id,
            location: location.clone(),
            identity,
        });
    }
    emit(pasted(request_id, vec![moved.clone()]));
}

#[test]
fn each_move_replay_records_only_the_original_it_replaced() {
    let browser = operating_browser();
    let record = move_record("/fixture/report.txt", "/fixture/archive/report.txt");
    let staged = |identity| {
        UndoEntry::Move(vec![MoveRecord {
            replaced: identity,
            ..record.clone()
        }])
    };
    let (first, at_source, at_target) = (
        folder_identity(41, 1),
        folder_identity(42, 2),
        folder_identity(43, 3),
    );
    push_pending_undo(staged(Some(first)));

    finish_move_replay(
        &browser,
        false,
        Some((&record.original, at_source)),
        &record.current,
    );
    assert_undo_and_redo(None, Some(staged(Some(at_source))));

    finish_move_replay(
        &browser,
        true,
        Some((&record.current, at_target)),
        &record.original,
    );
    assert_undo_and_redo(Some(staged(Some(at_target))), None);

    finish_move_replay(&browser, false, None, &record.current);
    assert_undo_and_redo(None, Some(staged(None)));
}

#[test]
fn an_undone_copy_can_be_redone() {
    let browser = operating_browser();
    transfer_one(&browser, "/fixture/archive", "/fixture/report.txt", false);

    assert_the_copy_undo_round_trips(&browser, &Location::local("/fixture/archive/report.txt"));
}

#[test]
fn a_put_back_can_be_retrashed_and_redone() {
    let browser = operating_browser();
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
    assert_the_copy_undo_round_trips(&browser, &destination);
}

#[test]
fn a_partial_undo_only_redoes_the_completed_items() {
    let first = Location::local("/fixture/first.txt");
    let second = Location::local("/fixture/second.txt");

    let _browser = replay_copy_undo_removing(
        vec![first.clone(), second.clone()],
        vec![first.clone()],
        "one copy could not be removed",
    );

    assert_undo_and_redo(
        Some(UndoEntry::Copy(vec![second])),
        Some(UndoEntry::Copy(vec![first])),
    );
}

#[test]
fn a_non_successful_replay_with_all_items_applied_remains_redoable() {
    let location = Location::local("/fixture/report.txt");

    let _browser = replay_copy_undo_removing(
        vec![location.clone()],
        vec![location.clone()],
        "failure after deletion",
    );

    assert_undo_and_redo(None, Some(UndoEntry::Copy(vec![location])));
}

#[test]
fn a_failed_undo_offers_no_redo() {
    assert_a_failed_undo_offers_no_redo(
        trash_undo(vec![Location::local("/fixture/report.txt")]),
        "restore failed",
    );
}

#[test]
fn an_undone_rename_can_be_redone() {
    let browser = operating_browser();
    let expected = rename_undo_entry();
    push_pending_undo(expected.clone());

    assert!(undo_pending_rename(&browser));
    assert_eq!(pending_redo_entry(), Some(expected.clone()));

    let (generation, _, _) = browser.pending_redo_rename().expect("pending redo rename");
    forget_requests(&UNDO_RENAME_REQUESTS);
    assert!(browser.redo_rename(generation));

    assert_undo_and_redo(Some(expected), None);
    assert_eq!(
        requests_in(&UNDO_RENAME_REQUESTS),
        vec![(
            Location::local("/fixture/original.txt"),
            Location::local("/fixture/renamed.txt"),
        )]
    );
}

#[test]
fn a_failed_rename_undo_offers_no_redo() {
    assert_a_failed_undo_offers_no_redo(rename_undo_entry(), "rename failed");
}

#[test]
fn another_browser_can_redo_the_undone_operation() {
    let browser = operating_browser();
    let other = operating_browser();
    let location = trash_report(&browser);

    assert!(browser.undo_last_trash());
    let (generation, locations) = other.pending_redo_trash().expect("shared pending redo");
    assert!(other.redo_trash(generation, locations));

    assert_undo_and_redo(Some(trash_undo(vec![location])), None);
}

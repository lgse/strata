// SPDX-License-Identifier: MIT

use super::*;

fn new_folder() -> Location {
    Location::local("/fixture/new folder")
}

/// The report and notes moves of a grouping gesture into `folder`.
fn grouped_moves(folder: &str) -> Vec<MoveRecord> {
    ["report.txt", "notes.txt"]
        .map(|name| move_record(&format!("/fixture/{name}"), &format!("{folder}/{name}")))
        .into()
}

fn grouped_report() -> MoveRecord {
    move_record("/fixture/report.txt", "/fixture/new folder/report.txt")
}

/// Records creating `created` as the expected group folder, then `moves`.
fn start_grouping(browser: &Rc<Browser>, created: &Location, moves: Vec<MoveRecord>) {
    push_pending_undo(UndoEntry::Copy(vec![created.clone()]));
    browser.expect_group_folder(created.clone());
    push_pending_undo(UndoEntry::Move(moves));
}

fn renamed_new_folder() -> UndoEntry {
    UndoEntry::Rename(RenameRecord {
        original: new_folder(),
        current: Location::local("/fixture/grouped"),
        native_name: OsString::from("new folder"),
        display_name: "new folder".to_owned(),
        is_hidden: false,
    })
}

/// A group in `/fixture/grouped` holding the moved report and `other`, undone without `other`.
fn undo_group_without(
    other: &str,
    undo: impl FnOnce(&Rc<Browser>, u64, Vec<UndoMoveItem>) -> bool,
) -> (Rc<Browser>, bool) {
    let browser = operating_browser();
    forget_requests(&UNDO_MOVE_CLEANUPS);
    let moved = move_record("/fixture/report.txt", "/fixture/grouped/report.txt");
    push_pending_undo(UndoEntry::Group {
        folder: Location::local("/fixture/grouped"),
        records: vec![
            moved.clone(),
            move_record(
                &format!("/fixture/{other}"),
                &format!("/fixture/grouped/{other}"),
            ),
        ],
    });
    let (generation, _) = browser.pending_undo_group().expect("group undo");
    let undone = undo(&browser, generation, undo_move_items([moved]));
    (browser, undone)
}

fn assert_an_interrupted_group_cleanup_remains_undoable(cancelled: bool) {
    let browser = operating_browser();
    let folder = Location::local("/fixture/grouped");
    let record = move_record("/fixture/report.txt", "/fixture/grouped/report.txt");
    let (request_id, emit) = start_replay(
        &browser,
        UndoEntry::Group {
            folder: folder.clone(),
            records: vec![record.clone()],
        },
        OperationSetup::MoveBack,
    );
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
    forget_requests(&UNDO_COPY_REQUESTS);
    let (generation, locations) = browser.pending_undo_copy().expect("cleanup undo");
    assert!(browser.undo_copy(generation, locations));
    assert_eq!(
        (requests_in(&UNDO_COPY_REQUESTS), pending_undo_entry()),
        (vec![vec![folder]], None)
    );
}

#[test]
fn a_completed_grouping_gesture_collapses_into_one_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let created = new_folder();
    start_grouping(&browser, &created, grouped_moves("/fixture/new folder"));

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Group {
            folder: created,
            records: grouped_moves("/fixture/new folder"),
        })
    );

    push_pending_undo(renamed_new_folder());

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Group {
            folder: Location::local("/fixture/grouped"),
            records: grouped_moves("/fixture/grouped"),
        })
    );
}

#[test]
fn dismissing_the_gesture_rename_leaves_a_later_folder_rename_separate() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    start_grouping(&browser, &new_folder(), vec![grouped_report()]);
    assert!(matches!(
        pending_undo_entry(),
        Some(UndoEntry::Group { .. })
    ));

    browser.clear_group_folder();
    push_pending_undo(renamed_new_folder());

    assert_eq!(pending_undo_entry(), Some(renamed_new_folder()));
}

#[test]
fn group_folding_leaves_an_interleaved_entry_untouched() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    start_grouping(&browser, &new_folder(), vec![grouped_report()]);
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
    let created = new_folder();
    let unrelated = move_record("/fixture/report.txt", "/fixture/archive/report.txt");
    start_grouping(&browser, &created, vec![unrelated.clone()]);

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Move(vec![unrelated])));
    let (generation, _) = claim_pending_undo(None).expect("undo claim");
    finish_undo(generation, true);
    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![created])));
}

#[test]
fn undoing_a_group_moves_back_and_trashes_the_folder_in_one_operation() {
    let browser = operating_browser();
    forget_requests(&UNDO_MOVE_REQUESTS);
    forget_requests(&UNDO_MOVE_CLEANUPS);
    forget_requests(&UNDO_MOVE_NAMES);
    let created = new_folder();
    start_grouping(&browser, &created, vec![grouped_report()]);
    let (generation, records) = browser.pending_undo_group().expect("pending group undo");

    assert!(browser.undo_group(generation, undo_move_items(records.clone())));

    assert_eq!(
        (
            requests_in(&UNDO_MOVE_REQUESTS),
            requests_in(&UNDO_MOVE_CLEANUPS),
            requests_in(&UNDO_MOVE_NAMES)
        ),
        (vec![records], vec![vec![created]], vec![NameMode::Keep])
    );
    assert_eq!(pending_undo_entry(), None);
    assert!(peek_replay(true).is_none(), "a group undo offers no redo");
}

#[test]
fn failed_or_cancelled_group_cleanup_remains_undoable() {
    for cancelled in [false, true] {
        assert_an_interrupted_group_cleanup_remains_undoable(cancelled);
    }
}

#[test]
fn skipping_a_grouped_item_does_not_trash_its_folder() {
    let (_browser, undone) = undo_group_without("skipped.txt", Browser::undo_group);

    assert!(undone);

    assert_eq!(
        requests_in(&UNDO_MOVE_CLEANUPS),
        vec![Vec::<Location>::new()]
    );
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn a_missing_grouped_item_keeps_the_folder_cleanup() {
    let (_browser, undone) = undo_group_without("gone.txt", Browser::undo_group_retaining_folder);

    assert!(undone);

    assert_eq!(
        requests_in(&UNDO_MOVE_CLEANUPS),
        vec![vec![Location::local("/fixture/grouped")]]
    );
}

#[test]
fn an_all_missing_group_leaves_its_folder_undoable() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let folder = Location::local("/fixture/grouped");
    push_pending_undo(UndoEntry::Group {
        folder: folder.clone(),
        records: vec![move_record(
            "/fixture/gone.txt",
            "/fixture/grouped/gone.txt",
        )],
    });
    let (generation, _) = browser.pending_undo_group().expect("group undo");
    browser.discard_group_keep_folder(false, generation);
    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![folder])));
}

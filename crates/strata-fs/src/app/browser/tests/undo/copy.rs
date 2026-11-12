// SPDX-License-Identifier: MIT

use super::*;

fn copy_into_archive(browser: &Rc<Browser>, source: &str) {
    transfer_one(browser, "/fixture/archive", source, false);
}

fn assert_a_copy_creating_nothing_keeps_the_trash_undo(destination: Location, pasted_at: Location) {
    let browser = operating_browser();
    let trashed = trash_report(&browser);
    let (request_id, emit) = start_operation(&browser, OperationSetup::Copy(destination));

    emit(pasted(request_id, vec![pasted_at]));

    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![trashed])));
}

#[test]
fn a_completed_copy_records_the_destinations_it_created() {
    let browser = operating_browser();

    copy_into_archive(&browser, "/fixture/report.txt");

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![Location::local(
            "/fixture/archive/report.txt"
        )]))
    );
}

#[test]
fn a_copy_that_created_nothing_leaves_the_previous_undo_current() {
    assert_a_copy_creating_nothing_keeps_the_trash_undo(
        Location::local("/fixture/archive"),
        Location::local("/fixture/note.txt"),
    );
}

#[test]
fn a_copy_into_an_admin_folder_records_no_trash_based_undo() {
    assert_a_copy_creating_nothing_keeps_the_trash_undo(
        Location::uri("admin:///srv/archive"),
        Location::uri("admin:///srv/archive/note.txt"),
    );
}

#[test]
fn undoing_a_copy_removes_only_the_destinations_it_created() {
    let browser = operating_browser();
    forget_requests(&UNDO_COPY_REQUESTS);
    copy_into_archive(&browser, "/fixture/report.txt");

    assert!(undo_pending_copy(&browser));

    assert_eq!(
        requests_in(&UNDO_COPY_REQUESTS),
        vec![vec![Location::local("/fixture/archive/report.txt")]]
    );
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn undoing_a_copy_removes_the_destination_names_the_paste_reported() {
    let browser = operating_browser();
    forget_requests(&UNDO_COPY_REQUESTS);
    let generated = Location::local("/fixture/report (1).txt");
    let replaced = Location::local("/fixture/archive/note.txt");
    let (request_id, emit) =
        start_operation(&browser, OperationSetup::Copy(Location::local("/fixture")));
    for created in [&generated, &replaced] {
        emit(transfer_progress(request_id, created));
    }
    emit(pasted(
        request_id,
        vec![
            Location::local("/fixture/report.txt"),
            Location::local("/fixture/note.txt"),
        ],
    ));

    assert!(undo_pending_copy(&browser));

    assert_eq!(
        requests_in(&UNDO_COPY_REQUESTS),
        vec![vec![generated, replaced]]
    );
}

#[test]
fn undoing_a_copy_leaves_the_previous_trash_undo_available() {
    let browser = operating_browser();
    let trashed = trash_report(&browser);
    copy_into_archive(&browser, "/fixture/note.txt");

    assert!(undo_pending_copy(&browser));

    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![trashed])));
    assert!(browser.undo_last_trash());
}

#[test]
fn undoing_a_copy_records_no_trash_undo_of_its_own() {
    let browser = operating_browser();
    copy_into_archive(&browser, "/fixture/report.txt");

    assert!(undo_pending_copy(&browser));

    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn a_cancelled_copy_records_the_destinations_it_reached() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let created = Location::local("/fixture/archive/first.txt");
    let (request_id, emit) = start_operation(
        &browser,
        OperationSetup::Copy(Location::local("/fixture/archive")),
    );

    emit(transfer_progress(request_id, &created));
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

    fail_replay_after(UndoEntry::Copy(vec![first.clone(), second.clone()]), &first);

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![second])));
}

#[test]
fn a_completed_copy_displaces_an_older_trash_undo() {
    let browser = operating_browser();
    trash_report(&browser);

    copy_into_archive(&browser, "/fixture/note.txt");

    assert!(!browser.undo_last_trash());
}

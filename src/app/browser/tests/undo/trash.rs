// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_completed_trash_operation_can_be_undone_once() {
    let browser = operating_browser();
    let location = trash_report(&browser);

    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![location])));
    assert!(browser.undo_last_trash());
    assert!(!browser.undo_last_trash());
}

#[test]
fn pending_trash_undo_reports_original_locations_until_claimed() {
    let browser = operating_browser();
    let location = trash_report(&browser);

    assert_eq!(browser.pending_undo_trash(), Some(vec![location]));
    assert!(browser.pending_undo_trash().is_some());
    assert_eq!(
        (browser.undo_last_trash(), browser.pending_undo_trash()),
        (true, None)
    );
}

#[test]
fn another_browser_can_undo_the_latest_trash_operation() {
    let deleting_browser = operating_browser();
    let undoing_browser = operating_browser();

    trash_report(&deleting_browser);

    assert!(undoing_browser.undo_last_trash());
    assert!(!deleting_browser.undo_last_trash());
}

#[test]
fn the_undo_history_drops_its_oldest_entry_once_full() {
    let numbered = |index| trash_undo(vec![Location::local(format!("/fixture/{index}.txt"))]);
    for index in 0..MAX_UNDO_HISTORY + 1 {
        push_pending_undo(numbered(index));
    }

    let retained = PENDING_UNDO.with(|pending| pending.borrow().history.len());

    assert_eq!(retained, MAX_UNDO_HISTORY);
    assert_eq!(pending_undo_entry(), Some(numbered(MAX_UNDO_HISTORY)));
}

#[test]
fn permanent_delete_preserves_the_previous_trash_undo() {
    let browser = operating_browser();

    trash_report(&browser);
    browser.delete(vec![fixture_entry("/fixture/draft.txt")], true);

    assert!(browser.undo_last_trash());
}

#[test]
fn failed_and_partial_undo_operations_can_be_retried() {
    let first = Location::local("/fixture/first.txt");
    let second = Location::local("/fixture/second.txt");

    fail_replay_after(trash_undo(vec![first.clone(), second.clone()]), &first);

    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![second.clone()])));
    let (retry_generation, retry) = claim_pending_undo(None).expect("retry claim");
    assert_eq!(retry, trash_undo(vec![second]));
    finish_undo(retry_generation, true);
    assert_eq!(pending_undo_entry(), None);
}

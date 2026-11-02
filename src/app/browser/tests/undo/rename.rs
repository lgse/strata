// SPDX-License-Identifier: MIT

use super::*;

/// A browser listing `/fixture` from a source that publishes watched changes.
fn watching_browser() -> (Rc<Browser>, Location) {
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: Rc::default(),
    }));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let parent = Location::local("/fixture");
    browser.navigate(parent.clone());
    (browser, parent)
}

fn rename_listed_folder(browser: &Rc<Browser>, parent: &Location, original: &FileEntry) {
    browser.handle_directory_change(0, parent, DirectoryChange::Upsert(original.clone()));
    browser.descend(0, original.location.clone());
    browser.rename(original.clone(), "renamed".to_owned());
}

/// Publishes newer metadata for the entry now listed at `current`.
fn update_listed_metadata(browser: &Rc<Browser>, parent: &Location, current: &Location) {
    let mut updated = browser
        .entry_at_location(current)
        .expect("forward rename published current entry");
    updated.size = MetadataValue::Known(99);
    updated.modified_unix_seconds = MetadataValue::Known(2);
    updated.mode = MetadataValue::Known(0o640);
    browser.handle_directory_change(0, parent, DirectoryChange::Upsert(updated));
}

fn assert_updated_metadata_at(entry: &FileEntry, location: &Location) {
    assert_eq!(
        (
            &entry.location,
            &entry.size,
            &entry.modified_unix_seconds,
            &entry.mode
        ),
        (
            location,
            &MetadataValue::Known(99),
            &MetadataValue::Known(2),
            &MetadataValue::Known(0o640)
        )
    );
}

fn assert_an_interrupted_rename_undo_stays_retryable(
    interruption: impl FnOnce(OperationRequestId) -> OperationEvent,
) {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let expected = rename_undo_entry();
    let (request_id, emit) = start_replay(&browser, expected.clone(), OperationSetup::Plain);

    emit(interruption(request_id));

    assert_eq!(pending_undo_entry(), Some(expected));
    assert!(browser.pending_undo_rename().is_some());
}

#[test]
fn rename_undo_records_the_exact_original_and_current_locations() {
    let browser = operating_browser();
    let entry = fixture_entry("/fixture/original.txt");

    browser.rename(entry.clone(), "renamed.txt".to_owned());

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Rename(RenameRecord {
            original: entry.location,
            current: Location::local("/fixture/renamed.txt"),
            native_name: entry.native_name,
            display_name: entry.display_name,
            is_hidden: entry.is_hidden,
        }))
    );
}

#[test]
fn rename_undo_excludes_failed_cancelled_or_invalid_forward_rename() {
    let browser = operating_browser();
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
    let browser = operating_browser();
    push_pending_undo(rename_undo_entry());
    let (stale_generation, _, _) = browser.pending_undo_rename().expect("pending Rename undo");
    let newer = Location::local("/fixture/newer.txt");
    push_pending_undo(trash_undo(vec![newer.clone()]));

    assert!(!browser.undo_rename(stale_generation));
    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![newer])));
}

#[test]
fn rename_undo_preserves_mixed_latest_operation_order() {
    let browser = operating_browser();
    browser.delete(vec![fixture_entry("/fixture/trashed.txt")], false);
    transfer_one(&browser, "/fixture/archive", "/fixture/moved.txt", true);
    transfer_one(&browser, "/fixture/archive", "/fixture/copied.txt", false);
    browser.rename(
        fixture_entry("/fixture/original.txt"),
        "renamed.txt".to_owned(),
    );

    let undone_latest_first = [
        undo_pending_rename(&browser),
        undo_pending_copy(&browser),
        undo_pending_move(&browser),
        browser.undo_last_trash(),
    ];

    assert_eq!(undone_latest_first, [true; 4]);
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn rename_undo_consumes_only_the_latest_rename_entry() {
    let browser = operating_browser();
    forget_requests(&UNDO_RENAME_REQUESTS);
    let older = Location::local("/fixture/older.txt");
    browser.delete(vec![fixture_entry("/fixture/older.txt")], false);
    let entry = fixture_entry("/fixture/original.txt");
    browser.rename(entry, "renamed.txt".to_owned());
    let (generation, current, original) =
        browser.pending_undo_rename().expect("pending Rename undo");

    assert!(browser.undo_rename(generation));
    assert_eq!(
        requests_in(&UNDO_RENAME_REQUESTS),
        vec![(current, original)]
    );
    assert_eq!(pending_undo_entry(), Some(trash_undo(vec![older])));
}

#[test]
fn rename_undo_preserves_current_metadata_after_the_forward_rename() {
    let (browser, parent) = watching_browser();
    let original = directory_entry("/fixture/original");
    let current = Location::local("/fixture/renamed");

    rename_listed_folder(&browser, &parent, &original);
    assert!(browser.entry_at_location(&original.location).is_none());
    update_listed_metadata(&browser, &parent, &current);
    let current_entry = browser
        .entry_at_location(&current)
        .expect("current entry after metadata update");
    assert_updated_metadata_at(&current_entry, &current);

    assert!(undo_pending_rename(&browser));

    let restored = browser
        .entry_at_location(&original.location)
        .expect("restored entry");
    assert_updated_metadata_at(&restored, &original.location);
}

#[test]
fn rename_undo_does_not_publish_historical_metadata_when_current_entry_is_unavailable() {
    let (browser, parent) = watching_browser();
    let mut original = directory_entry("/fixture/original");
    original.size = MetadataValue::Known(7);
    original.modified_unix_seconds = MetadataValue::Known(1);
    original.mode = MetadataValue::Known(0o600);
    let current = Location::local("/fixture/renamed");

    browser.handle_directory_change(
        0,
        &parent,
        DirectoryChange::Remove(Location::local("/fixture/child")),
    );
    rename_listed_folder(&browser, &parent, &original);
    update_listed_metadata(&browser, &parent, &current);
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

    let events = record_events(&browser);

    assert!(undo_pending_rename(&browser));
    assert_eq!(
        (
            browser.entry_at_location(&original.location).is_none(),
            browser.location_at(1)
        ),
        (true, Some(original.location.clone()))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
    );
}

#[test]
fn failed_rename_undo_releases_its_claim_and_remains_retryable() {
    assert_an_interrupted_rename_undo_stays_retryable(|request_id| failed(request_id, "occupied"));
}

#[test]
fn cancelled_rename_undo_releases_its_claim_and_remains_retryable() {
    assert_an_interrupted_rename_undo_stays_retryable(|request_id| OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation::default(),
    });
}

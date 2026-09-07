// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::{FileEntry, Location};

#[test]
fn delete_confirmation_direction_keys_choose_an_action() {
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::Left),
        Some(DeleteConfirmationFocus::Cancel)
    );
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::h),
        Some(DeleteConfirmationFocus::Cancel)
    );
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::Right),
        Some(DeleteConfirmationFocus::Confirm)
    );
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::l),
        Some(DeleteConfirmationFocus::Confirm)
    );
    assert_eq!(delete_confirmation_focus_target(gtk::gdk::Key::Tab), None);
}

#[test]
fn retryable_delete_entries_keeps_only_the_named_locations() {
    let entry = |name: &str| FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        thumbnail_path: None,
        display_name: name.into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
    };
    let retryable = entry("share-file.txt");
    let denied = entry("locked-file.txt");
    let entries = vec![retryable.clone(), denied];

    let kept = retryable_delete_entries(entries, std::slice::from_ref(&retryable.location));

    assert_eq!(kept, vec![retryable]);
}

#[test]
fn retryable_delete_entries_is_empty_when_nothing_matches() {
    let entry = FileEntry {
        location: Location::local("/fixture/photo"),
        native_name: "photo".into(),
        thumbnail_path: None,
        display_name: "photo".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
    };

    let kept = retryable_delete_entries(vec![entry], &[]);

    assert!(kept.is_empty());
}

#[test]
fn restore_confirmation_shows_the_full_destination_path() {
    assert_eq!(
        restore_destination_text(std::path::Path::new(
            "/home/user/Documents/Projects/report.txt"
        )),
        "/home/user/Documents/Projects/report.txt"
    );
}

#[test]
fn restore_confirmation_names_the_item_count_and_destination_action() {
    assert_eq!(restore_confirmation_title(1), "Restore 1 item?");
    assert_eq!(restore_confirmation_title(3), "Restore 3 items?");
    assert_eq!(restore_confirmation_confirm_label(1), "Restore");
    assert_eq!(restore_confirmation_confirm_label(2), "Restore 2 items");
}

#[test]
fn restore_error_summary_includes_the_failure_reason() {
    assert_eq!(
        restore_error_summary(&[
            "notes.txt: The original location is outside the trash volume and cannot be restored"
                .to_owned()
        ]),
        "notes.txt: The original location is outside the trash volume and cannot be restored"
    );
    let summary = restore_error_summary(&["a: denied".to_owned(), "b: denied".to_owned()]);
    assert!(summary.starts_with("2 items could not be restored."));
    assert!(summary.contains("a: denied"));
}

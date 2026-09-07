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

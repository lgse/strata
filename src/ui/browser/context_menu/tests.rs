// SPDX-License-Identifier: GPL-3.0-or-later

mod actions;

use super::*;
use crate::model::{FileEntry, Location};

#[test]
fn multi_selection_summary_lists_at_most_three_names() {
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

    assert_eq!(
        selected_items_summary(&[entry("one"), entry("two"), entry("three")]),
        "one, two, three"
    );
    assert_eq!(
        selected_items_summary(&[entry("one"), entry("two"), entry("three"), entry("four")]),
        "one, two, three, …"
    );
    let summary = selected_items_summary(&[
        entry("a-very-long-file-name-that-would-expand-the-context-menu"),
        entry("another-very-long-file-name-that-would-expand-the-menu"),
    ]);
    assert_eq!(
        summary.chars().count(),
        ITEM_CONTEXT_SUMMARY_MAX_CHARS as usize
    );
    assert!(summary.ends_with('…'));
}

#[test]
fn context_menu_uses_the_roomier_side_of_the_click() {
    assert_eq!(
        context_menu_placement(800, 120.0),
        (gtk::PositionType::Bottom, 656)
    );
    assert_eq!(
        context_menu_placement(800, 680.0),
        (gtk::PositionType::Top, 656)
    );
}

#[test]
fn context_menu_keeps_a_positive_scrollable_height_in_a_small_view() {
    assert_eq!(
        context_menu_placement(20, 10.0),
        (gtk::PositionType::Bottom, 1)
    );
}

#[test]
fn move_to_trash_hides_only_for_a_confirmed_unsupported_location() {
    assert!(!move_to_trash_is_visible(false, Some(false)));
}

#[test]
fn move_to_trash_shows_for_a_confirmed_supported_location() {
    assert!(move_to_trash_is_visible(false, Some(true)));
}

#[test]
fn move_to_trash_defaults_to_visible_before_the_check_resolves() {
    // `None` covers both "the load hasn't finished yet" and "the check itself
    // couldn't be answered" -- neither should ever hide the only delete option.
    assert!(move_to_trash_is_visible(false, None));
}

#[test]
fn move_to_trash_stays_visible_inside_trash_regardless_of_can_trash() {
    // Inside Trash this button is really "Permanently delete" under a shared
    // label; an ordinary location's Trash support is irrelevant there.
    assert!(move_to_trash_is_visible(true, Some(false)));
    assert!(move_to_trash_is_visible(true, None));
}

#[test]
fn permanently_delete_hides_only_for_a_confirmed_unsupported_location() {
    assert!(!permanently_delete_is_visible(false, Some(false)));
}

#[test]
fn permanently_delete_shows_for_a_confirmed_supported_location() {
    assert!(permanently_delete_is_visible(false, Some(true)));
}

#[test]
fn permanently_delete_defaults_to_visible_before_the_check_resolves() {
    assert!(permanently_delete_is_visible(false, None));
}

#[test]
fn permanently_delete_hides_inside_trash_regardless_of_can_delete() {
    assert!(!permanently_delete_is_visible(true, Some(true)));
    assert!(!permanently_delete_is_visible(true, None));
}

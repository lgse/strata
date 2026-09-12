// SPDX-License-Identifier: MIT

mod actions;
mod menus;
mod open_with;

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
        (gtk::PositionType::Bottom, 768)
    );
    assert_eq!(
        context_menu_placement(800, 680.0),
        (gtk::PositionType::Top, 768)
    );
}

#[test]
fn context_menu_uses_full_height_instead_of_scrolling_one_side() {
    assert_eq!(
        context_menu_placement(800, 400.0),
        (gtk::PositionType::Bottom, 768)
    );
    assert_eq!(
        context_menu_placement(800, 401.0),
        (gtk::PositionType::Top, 768)
    );
}

#[test]
fn context_menu_anchor_stays_on_the_click_when_the_menu_fits() {
    assert_eq!(
        shifted_anchor_y(gtk::PositionType::Bottom, 800, 120, 400),
        120
    );
    assert_eq!(shifted_anchor_y(gtk::PositionType::Top, 800, 680, 400), 680);
}

#[test]
fn context_menu_anchor_shifts_to_use_space_on_the_other_side() {
    assert_eq!(
        shifted_anchor_y(gtk::PositionType::Bottom, 800, 700, 300),
        484
    );
    assert_eq!(shifted_anchor_y(gtk::PositionType::Top, 800, 200, 300), 316);
}

#[test]
fn context_menu_anchor_clamps_when_the_menu_exceeds_the_view() {
    assert_eq!(
        shifted_anchor_y(gtk::PositionType::Bottom, 800, 700, 900),
        CONTEXT_MENU_EDGE_MARGIN
    );
    assert_eq!(
        shifted_anchor_y(gtk::PositionType::Top, 800, 100, 900),
        800 - CONTEXT_MENU_EDGE_MARGIN
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
fn delete_actions_follow_location_and_resolved_capabilities() {
    // In Trash the shared trash action becomes permanent deletion. Unknown
    // capabilities elsewhere must not hide the only available delete action.
    for (in_trash, capability, trash_action, permanent_action) in [
        (false, None, true, true),
        (false, Some(false), false, false),
        (false, Some(true), true, true),
        (true, None, true, false),
        (true, Some(false), true, false),
        (true, Some(true), true, false),
    ] {
        assert_eq!(
            move_to_trash_is_visible(in_trash, capability),
            trash_action,
            "trash action: in_trash={in_trash}, capability={capability:?}"
        );
        assert_eq!(
            permanently_delete_is_visible(in_trash, capability),
            permanent_action,
            "permanent action: in_trash={in_trash}, capability={capability:?}"
        );
    }
}

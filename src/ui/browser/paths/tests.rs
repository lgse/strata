// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::{FileEntry, Location};
use crate::ui::browser::PinStatus;
use gtk::glib;

#[test]
fn only_the_trash_root_uses_the_aggregate_properties_size() {
    assert!(is_trash_root(&Location::uri("trash:///")));
    assert!(!is_trash_root(&Location::uri("trash:///photo.png")));
    assert!(!is_trash_root(&Location::local(
        "/home/user/.local/share/Trash"
    )));
}

#[test]
fn trash_locations_include_the_root_and_descendants() {
    assert!(is_trash_location(&Location::uri("trash:///")));
    assert!(is_trash_location(&Location::uri("trash:///folder")));
    assert!(!is_trash_location(&Location::local(
        "/home/example/.local/share/Trash"
    )));
}

#[test]
fn properties_paths_abbreviate_the_home_directory() {
    let home = glib::home_dir();

    assert_eq!(compact_display_path(&Location::local(&home)), "~");
    assert_eq!(
        compact_display_path(&Location::local(home.join("Documents/report.txt"))),
        "~/Documents/report.txt"
    );
    assert_eq!(
        compact_display_path(&Location::uri("trash:///example")),
        "trash:///example"
    );
}

#[test]
fn pinning_requires_an_available_non_trash_directory() {
    let entry = |location, kind| FileEntry {
        location,
        native_name: "item".into(),
        thumbnail_path: None,
        display_name: "item".into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
    };
    let directory = entry(
        Location::local("/fixture/folder"),
        crate::model::EntryKind::Directory,
    );
    let file = entry(
        Location::local("/fixture/file"),
        crate::model::EntryKind::File,
    );
    let trash_directory = entry(
        Location::uri("trash:///deleted-folder"),
        crate::model::EntryKind::Directory,
    );

    assert!(can_pin_entry(&directory, PinStatus::Available));
    assert!(!can_pin_entry(&directory, PinStatus::Pinned));
    assert!(!can_pin_entry(&directory, PinStatus::Unavailable));
    assert!(!can_pin_entry(&file, PinStatus::Available));
    assert!(!can_pin_entry(&trash_directory, PinStatus::Available));
}

#[test]
fn properties_offers_unpin_for_an_already_pinned_directory() {
    let folder = Location::local("/fixture/folder");

    assert_eq!(
        pin_action_for(&folder, true, PinStatus::Available),
        Some(PinAction::Pin)
    );
    assert_eq!(
        pin_action_for(&folder, true, PinStatus::Pinned),
        Some(PinAction::Unpin)
    );
    assert_eq!(PinAction::Pin.label(), "Pin");
    assert_eq!(PinAction::Unpin.label(), "Unpin");
}

#[test]
fn properties_hides_the_pin_control_where_pinning_is_impossible() {
    let folder = Location::local("/fixture/folder");

    assert_eq!(pin_action_for(&folder, true, PinStatus::Unavailable), None);
    assert_eq!(pin_action_for(&folder, false, PinStatus::Available), None);
    assert_eq!(
        pin_action_for(
            &Location::uri("trash:///folder"),
            true,
            PinStatus::Available
        ),
        None
    );
}

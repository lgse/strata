// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    BrowserMode, ClickActivation, ClickCount, EXPLORER_COLUMN_MIN_WIDTHS, EXPLORER_COLUMN_WIDTHS,
    MAX_GRID_THUMBNAIL_SIZE, MIN_GRID_THUMBNAIL_SIZE, SourceIndexMap, compare_type_groups,
    explorer_column_width, first_model_type_group, grid_card_extent, grid_card_icon_slot,
    metadata_fill_position, scroll_delta_for_unit, should_activate_pointer_click,
    type_group_for_first_intersecting, type_group_sorter, type_groups_of, value_type_group,
};
use crate::model::{EntryKind, FileEntry, Location, MetadataValue};
use gtk::{gio, prelude::*};
use std::process::Command;

/// Model values as the panes store them: kind, hidden flag, then the display name.
fn value(kind: char, name: &str) -> String {
    format!("{kind}v\t{name}")
}

#[test]
fn explorer_columns_have_usable_minimum_widths() {
    for (index, minimum) in EXPLORER_COLUMN_MIN_WIDTHS.into_iter().enumerate() {
        assert_eq!(explorer_column_width(index, minimum - 1), minimum);
        assert_eq!(explorer_column_width(index, minimum + 1), minimum + 1);
    }
}

#[test]
fn explorer_default_widths_respect_column_minimums() {
    for (default, minimum) in EXPLORER_COLUMN_WIDTHS
        .into_iter()
        .zip(EXPLORER_COLUMN_MIN_WIDTHS)
    {
        assert!(default >= minimum);
    }
}

#[test]
fn stored_click_counts_reject_unsupported_values() {
    assert_eq!(ClickCount::from_stored(1), Some(ClickCount::One));
    assert_eq!(ClickCount::from_stored(2), Some(ClickCount::Two));
    assert_eq!(ClickCount::from_stored(0), None);
    assert_eq!(ClickCount::from_stored(3), None);
}

#[test]
fn click_activation_defaults_follow_view_conventions() {
    assert_eq!(
        ClickActivation::default_for(BrowserMode::Columns),
        ClickActivation {
            files: ClickCount::Two,
            folders: ClickCount::One,
        }
    );
    for mode in [BrowserMode::Grid, BrowserMode::Explorer] {
        assert_eq!(
            ClickActivation::default_for(mode),
            ClickActivation {
                files: ClickCount::Two,
                folders: ClickCount::Two,
            }
        );
    }
}

#[test]
fn single_click_activation_distinguishes_files_and_folders() {
    let activation = ClickActivation {
        files: ClickCount::Two,
        folders: ClickCount::One,
    };

    assert!(should_activate_pointer_click(1, true, activation));
    assert!(!should_activate_pointer_click(1, false, activation));
    assert!(!should_activate_pointer_click(2, true, activation));
}

#[test]
fn alternate_modes_request_missing_metadata_for_bound_entries() {
    let mut entry = FileEntry {
        location: Location::local("/fixture/photo.jpg"),
        native_name: "photo.jpg".into(),
        display_name: "photo.jpg".into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        is_hidden: false,
    };

    assert_eq!(metadata_fill_position(Some(7), &entry, false), Some(7));
    assert_eq!(metadata_fill_position(None, &entry, false), None);

    entry.size = MetadataValue::Known(100);
    assert_eq!(metadata_fill_position(Some(7), &entry, false), Some(7));
    entry.modified_unix_seconds = MetadataValue::Known(1);
    assert_eq!(metadata_fill_position(Some(7), &entry, false), None);
    assert_eq!(metadata_fill_position(Some(7), &entry, true), Some(7));
    entry.mode = MetadataValue::Known(0o100644);
    assert_eq!(metadata_fill_position(Some(7), &entry, true), None);
}

#[test]
fn grid_cards_keep_a_uniform_icon_slot_and_two_line_label() {
    assert_eq!(grid_card_icon_slot(26), MIN_GRID_THUMBNAIL_SIZE);
    assert_eq!(grid_card_icon_slot(128), 128);
    assert_eq!(grid_card_icon_slot(512), MAX_GRID_THUMBNAIL_SIZE);
    assert_eq!(grid_card_extent(26), grid_card_extent(64));
    assert_eq!(grid_card_extent(64), (156, 107));
    assert_eq!(grid_card_extent(128), (156, 171));
    assert_eq!(grid_card_extent(256), (256, 299));
    assert_eq!(grid_card_extent(512), grid_card_extent(256));
}

#[test]
fn grid_scroll_maps_a_wheel_notch_from_page_size() {
    let wheel = scroll_delta_for_unit(1.0, 1000.0, gtk::gdk::ScrollUnit::Wheel);
    assert!((wheel - 100.0).abs() < 1e-9);
    assert!(scroll_delta_for_unit(1.0, 8000.0, gtk::gdk::ScrollUnit::Wheel) > wheel);
    assert_eq!(
        scroll_delta_for_unit(4.0, 100.0, gtk::gdk::ScrollUnit::Surface),
        10.0
    );
    assert_eq!(
        scroll_delta_for_unit(1.0, 50.0, gtk::gdk::ScrollUnit::Surface),
        scroll_delta_for_unit(1.0, 999.0, gtk::gdk::ScrollUnit::Surface)
    );
}

#[test]
fn folders_lead_the_groups_and_the_rest_are_alphabetical() {
    let mut groups = vec!["Zip archive", "Folder", "JSON document", "audio"];
    groups.sort_by(|left, right| compare_type_groups(left, right));

    assert_eq!(groups, ["Folder", "audio", "JSON document", "Zip archive"]);
}

#[test]
fn the_inline_new_entry_row_sorts_ahead_of_every_group() {
    assert!(compare_type_groups("", "Folder").is_lt());
    assert!(compare_type_groups("", "JSON document").is_lt());
    assert_eq!(value_type_group(""), "");
}

#[test]
fn every_loaded_type_appears_once_with_folders_first() {
    let values = [
        value('f', "notes.json"),
        value('d', "projects"),
        value('f', "data.json"),
        value('d', "archive"),
    ];

    let groups = type_groups_of(values.iter());

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0], "Folder");
    assert_eq!(groups[1], value_type_group(&value('f', "notes.json")));
}

#[test]
fn entries_of_one_type_share_a_group() {
    assert_eq!(
        value_type_group(&value('f', "notes.md")),
        value_type_group(&value('f', "README.md"))
    );
    assert_ne!(
        value_type_group(&value('f', "notes.md")),
        value_type_group(&value('f', "notes.json"))
    );
}

#[test]
fn type_group_sort_clusters_mixed_entries_and_keeps_placeholder_first() {
    let mut values = [
        value('f', "notes.json"),
        String::new(),
        value('d', "projects"),
        value('f', "data.json"),
        value('d', "archive"),
        value('f', "README.md"),
    ];
    values.sort_by(|left, right| {
        compare_type_groups(&value_type_group(left), &value_type_group(right))
    });

    assert_eq!(values[0], "");
    assert_eq!(value_type_group(&values[1]), "Folder");
    assert_eq!(value_type_group(&values[2]), "Folder");
    let json = value_type_group(&value('f', "notes.json"));
    let markdown = value_type_group(&value('f', "README.md"));
    assert_eq!(value_type_group(&values[3]), json);
    assert_eq!(value_type_group(&values[4]), json);
    assert_eq!(value_type_group(&values[5]), markdown);
    assert!(compare_type_groups(&json, &markdown).is_lt());
}

#[test]
fn the_sticky_heading_uses_the_first_card_intersecting_the_viewport() {
    let json = value('f', "notes.json");
    let folder = value('d', "projects");
    let markdown = value('f', "README.md");
    let json_group = value_type_group(&json);
    assert_eq!(
        type_group_for_first_intersecting(
            [
                (0, String::new(), 0.0, 40.0),
                (1, folder, -80.0, 40.0),
                (12, markdown, 35.0, 40.0),
                (8, json.clone(), -10.0, 40.0),
            ]
            .into_iter(),
            100.0,
        )
        .as_deref(),
        Some(json_group.as_str())
    );
    assert_eq!(
        type_group_for_first_intersecting(
            [(0, String::new(), 0.0, 40.0), (1, json, 100.0, 40.0)].into_iter(),
            100.0,
        ),
        None
    );
}

const TYPE_GROUP_SORT_GTK_CHILD: &str = "STRATA_TYPE_GROUP_SORT_GTK_CHILD";
const TYPE_GROUP_SORT_TEST: &str =
    "ui::browser_modes::tests::type_group_sorter_clusters_a_flattened_grid_model";

fn run_type_group_sorter_checks() {
    let source = gtk::StringList::new(&[
        &value('f', "notes.json"),
        &value('d', "projects"),
        &value('f', "data.json"),
        &value('d', "archive"),
        &value('d', "alpha"),
        &value('d', "beta"),
        &value('d', "gamma"),
        &value('f', "README.md"),
    ]);
    let placeholder = gtk::StringList::new(&[""]);
    let stacked = gio::ListStore::new::<gio::ListModel>();
    stacked.append(&placeholder.clone().upcast::<gio::ListModel>());
    stacked.append(&source.clone().upcast::<gio::ListModel>());
    let flattened = gtk::FlattenListModel::new(Some(stacked));
    let sorter = type_group_sorter();
    let sorted = gtk::SortListModel::new(Some(flattened), Some(sorter.clone()));
    sorted.set_section_sorter(Some(&sorter));

    let groups: Vec<String> = (0..sorted.n_items())
        .filter_map(|index| sorted.item(index).map(|item| super::model_value(&item)))
        .map(|value| value_type_group(&value))
        .collect();
    assert_eq!(groups[0], "");
    assert_eq!(&groups[1..6], ["Folder"; 5]);
    let json = value_type_group(&value('f', "notes.json"));
    let markdown = value_type_group(&value('f', "README.md"));
    assert_eq!(&groups[6..8], [json.as_str(), json.as_str()]);
    assert_eq!(groups[8], markdown);
    assert_eq!(sorted.section(1), (1, 6));
    assert_eq!(sorted.section(5), (1, 6));
    assert_eq!(sorted.section(6), (6, 8));
    assert_eq!(first_model_type_group(sorted.upcast_ref()), "Folder");
}

#[test]
fn type_group_sorter_clusters_a_flattened_grid_model() {
    if std::env::var_os(TYPE_GROUP_SORT_GTK_CHILD).is_some() {
        if gtk::init().is_err() {
            return;
        }
        run_type_group_sorter_checks();
        return;
    }

    let status = Command::new(std::env::current_exe().expect("test executable should exist"))
        .args(["--exact", TYPE_GROUP_SORT_TEST])
        .env(TYPE_GROUP_SORT_GTK_CHILD, "1")
        .status()
        .expect("isolated GTK grouping test should start");
    assert!(status.success(), "isolated GTK grouping test failed");
}

const GTK_CHILD: &str = "STRATA_SOURCE_INDEX_MAP_GTK_CHILD";
const SOURCE_INDEX_TEST: &str =
    "ui::browser_modes::tests::source_index_map_tracks_filter_sort_and_placeholder";

fn run_source_index_map_checks() {
    let source = gtk::StringList::new(&["fv\talpha", "dh\t.secret", "fv\tneedle"]);
    let map = SourceIndexMap::watch(&source);
    assert_eq!(map.of_view_position(&source, 0), Some(0));
    assert_eq!(map.of_view_position(&source, 2), Some(2));

    source.append("fv\tlate");
    assert_eq!(map.of_view_position(&source, 3), Some(3));

    source.splice(1, 0, &["fv\tmiddle"]);
    assert_eq!(
        map.of_item(&source.item(1).expect("inserted item")),
        Some(1)
    );
    assert_eq!(map.of_item(&source.item(4).expect("shifted item")), Some(4));

    let hide_hidden = gtk::CustomFilter::new(|item| {
        item.downcast_ref::<gtk::StringObject>()
            .is_some_and(|value| value.string().as_bytes().get(1) != Some(&b'h'))
    });
    let visible = gtk::FilterListModel::new(Some(source.clone()), Some(hide_hidden));
    assert_eq!(map.of_view_position(&visible, 0), Some(0));
    assert_eq!(map.of_view_position(&visible, 1), Some(1));
    assert_eq!(map.of_view_position(&visible, 3), Some(4));

    let needle = gtk::CustomFilter::new(|item| {
        item.downcast_ref::<gtk::StringObject>()
            .is_some_and(|value| value.string().contains("needle"))
    });
    let matches = gtk::FilterListModel::new(Some(source.clone()), Some(needle));
    assert_eq!(map.of_view_position(&matches, 0), Some(3));

    let sorter = gtk::CustomSorter::new(|left, right| {
        let left = left
            .downcast_ref::<gtk::StringObject>()
            .map(|value| value.string())
            .unwrap_or_default();
        let right = right
            .downcast_ref::<gtk::StringObject>()
            .map(|value| value.string())
            .unwrap_or_default();
        right.cmp(&left).into()
    });
    let sorted = gtk::SortListModel::new(Some(source.clone()), Some(sorter));
    let first_sorted = sorted.item(0).expect("sorted model should have rows");
    let mapped = map.of_item(&first_sorted);
    assert!(mapped.is_some());
    assert_ne!(
        mapped,
        Some(0),
        "reverse sort should move the first source item off view index 0"
    );
    assert_eq!(map.of_view_position(&sorted, 0), mapped);

    let placeholder = gtk::StringList::new(&["creating"]);
    let stacked = gio::ListStore::new::<gio::ListModel>();
    stacked.append(&placeholder.clone().upcast::<gio::ListModel>());
    stacked.append(&source.clone().upcast::<gio::ListModel>());
    let flattened = gtk::FlattenListModel::new(Some(stacked));
    assert!(
        map.of_view_position(&flattened, 0).is_none(),
        "the inline placeholder is not a source entry"
    );
    assert_eq!(map.of_view_position(&flattened, 1), Some(0));

    assert_eq!(
        super::view_position_for_source(&source, Some(source.upcast_ref()), 4),
        Some(4)
    );
    assert_eq!(
        super::view_position_for_source(&source, Some(visible.upcast_ref()), 3),
        Some(2)
    );
    assert_eq!(
        super::view_position_for_source(&source, Some(flattened.upcast_ref()), 0),
        Some(1)
    );

    let source = gtk::StringList::new(&["fv\talpha"]);
    let weak = source.downgrade();
    let map = SourceIndexMap::watch(&source);
    drop(source);
    drop(map);
    assert!(
        weak.upgrade().is_none(),
        "watching must not pin the StringList after the pane drops"
    );
}

mod skeletons;

#[test]
fn source_index_map_tracks_filter_sort_and_placeholder() {
    if std::env::var_os(GTK_CHILD).is_some() {
        if gtk::init().is_err() {
            return;
        }
        run_source_index_map_checks();
        return;
    }

    let status = Command::new(std::env::current_exe().expect("test executable should exist"))
        .args(["--exact", SOURCE_INDEX_TEST])
        .env(GTK_CHILD, "1")
        .status()
        .expect("isolated GTK mapping test should start");
    assert!(status.success(), "isolated GTK mapping test failed");
}

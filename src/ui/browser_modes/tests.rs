// SPDX-License-Identifier: MIT

use super::{
    BrowserDensity, BrowserMode, ClickActivation, ClickCount, LIST_COLUMN_MIN_WIDTHS,
    SourceIndexMap, compare_type_groups, list_column_width, metadata_fill_position,
    should_activate_filtered_pointer, should_activate_pointer_click, type_group_sorter,
    type_groups_of, value_type_group,
};
use crate::model::{EntryKind, FileEntry, Location, MetadataValue};
use crate::test_support::gtk_test;
use gtk::{gio, prelude::*};
use std::path::PathBuf;
use std::process::Command;
use std::{cell::RefCell, collections::HashSet};

impl super::ModeViews {
    pub(in crate::ui) fn assert_saved_preferences(&self, manager: &crate::ui::theme::ThemeManager) {
        assert_eq!(self.density, manager.browser_density());
        assert_eq!(self.group_by_type, manager.group_by_type());
        assert_eq!(
            self.single_click_previews.get(),
            manager.single_click_previews()
        );
        assert_eq!(
            self.icons_click_activation.get(),
            manager.click_activation(BrowserMode::Icons)
        );
        assert_eq!(
            self.list_click_activation.get(),
            manager.click_activation(BrowserMode::List)
        );
    }
}

#[test]
fn pointer_controls_cover_navigation_and_pane_actions() {
    gtk_test(
        "ui::browser_modes::tests::pointer_controls_cover_navigation_and_pane_actions",
        || {
            let browser =
                crate::app::Browser::new(std::rc::Rc::new(crate::adapters::LocalFileSource));
            let navigation = super::list_navigation(&browser);
            let mut child = navigation.first_child();
            let mut count = 0;
            while let Some(button) = child {
                assert_eq!(
                    button.cursor().and_then(|cursor| cursor.name()).as_deref(),
                    Some("pointer")
                );
                count += 1;
                child = button.next_sibling();
            }
            assert_eq!(count, 3);
            let headings = super::list_headings(&browser, 0, super::ListColumnLayout::new());
            let mut child = headings.first_child();
            let mut index = 0;
            while let Some(cell) = child {
                let button = cell
                    .first_child()
                    .expect("heading overlay")
                    .downcast::<gtk::Overlay>()
                    .expect("overlay")
                    .child()
                    .expect("heading button");
                assert_eq!(
                    button.cursor().and_then(|cursor| cursor.name()).as_deref(),
                    if index == 1 { None } else { Some("pointer") }
                );
                index += 1;
                child = cell.next_sibling();
            }
            assert_eq!(index, 5);
            let controls = super::icons_controls(&browser, 0, 128);
            assert_eq!(controls.thumbnail_scale.adjustment().lower(), 32.0);
            controls.thumbnail_scale.set_value(32.0);
            assert_eq!(controls.thumbnail_scale.value(), 32.0);
            let mut child = controls.actions.first_child();
            let mut count = 0;
            while let Some(button) = child {
                assert_eq!(button.valign(), gtk::Align::Center);
                assert_eq!(
                    button.cursor().and_then(|cursor| cursor.name()).as_deref(),
                    Some("pointer")
                );
                count += 1;
                child = button.next_sibling();
            }
            assert_eq!(count, 6);
        },
    );
}

/// Model values as the panes store them: kind, hidden flag, then the display name.
fn value(kind: char, name: &str) -> String {
    format!("{kind}v\t{name}")
}

#[test]
fn list_columns_have_usable_minimum_widths() {
    for (index, minimum) in LIST_COLUMN_MIN_WIDTHS.into_iter().enumerate() {
        assert_eq!(list_column_width(index, minimum - 1), minimum);
        assert_eq!(list_column_width(index, minimum + 1), minimum + 1);
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
    for mode in [BrowserMode::Icons, BrowserMode::List] {
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
fn type_grouping_is_list_only() {
    assert!(!BrowserMode::Columns.supports_type_grouping());
    assert!(!BrowserMode::Icons.supports_type_grouping());
    assert!(BrowserMode::List.supports_type_grouping());
}

#[test]
fn filtered_activation_ignores_click_preferences() {
    let query = RefCell::new("report".to_owned());
    assert!(should_activate_filtered_pointer(1, &query));
    assert!(!should_activate_filtered_pointer(2, &query));
    query.replace(String::new());
    assert!(!should_activate_filtered_pointer(1, &query));
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
        thumbnail_path: None,
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
fn icons_columns_follow_viewport_width() {
    assert_eq!(
        super::icons_columns_for_width(800, 120, BrowserDensity::Compact),
        6
    );
    assert_eq!(
        super::icons_columns_for_width(120, 120, BrowserDensity::Compact),
        1
    );
    assert_eq!(
        super::icons_columns_for_width(80, 120, BrowserDensity::Compact),
        1
    );
    assert_eq!(
        super::icons_columns_for_width(8000, 120, BrowserDensity::Compact),
        20
    );
    assert_eq!(
        super::icons_columns_for_width(8000, 120, BrowserDensity::Airy),
        16
    );
}

#[test]
fn ungrouped_icons_reflow_when_the_preview_split_closes() {
    gtk_test(
        "ui::browser_modes::tests::ungrouped_icons_reflow_when_the_preview_split_closes",
        || {
            let names: Vec<String> = (0..24).map(|index| format!("item-{index}")).collect();
            let labels: Vec<&str> = names.iter().map(String::as_str).collect();
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(|_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
                card.set_size_request(80, 80);
                item.set_child(Some(&card));
            });
            let grid = gtk::GridView::new(
                Some(gtk::NoSelection::new(Some(gtk::StringList::new(&labels)))),
                Some(factory),
            );
            grid.set_min_columns(1);
            grid.set_max_columns(20);
            grid.set_hexpand(true);
            grid.set_vexpand(true);
            let scroll = gtk::ScrolledWindow::builder()
                .child(&grid)
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .hexpand(true)
                .vexpand(true)
                .build();
            let grid_for_pin = grid.clone();
            super::after_icons_viewport_width_changes(&scroll, move |width| {
                super::pin_ungrouped_grid_columns(
                    &grid_for_pin,
                    super::icons_columns_for_width(width, 80, BrowserDensity::Compact),
                );
            });
            let preview = gtk::Box::new(gtk::Orientation::Vertical, 0);
            preview.set_size_request(360, -1);
            let split = gtk::Paned::new(gtk::Orientation::Horizontal);
            split.set_resize_start_child(true);
            split.set_resize_end_child(false);
            split.set_shrink_start_child(false);
            split.set_shrink_end_child(true);
            split.set_start_child(Some(&scroll));
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(400)
                .child(&split)
                .build();
            window.present();
            drain_gtk();
            let open = first_row_columns(&grid);
            split.set_end_child(Some(&preview));
            drain_gtk();
            let with_preview = first_row_columns(&grid);
            split.set_end_child(None::<&gtk::Widget>);
            drain_gtk();
            let closed = first_row_columns(&grid);
            window.close();
            assert!(
                with_preview < open,
                "opening the preview pane should drop columns ({with_preview} with preview, {open} open)"
            );
            assert_eq!(
                closed, open,
                "closing the preview pane should restore the grid ({closed} after close, {open} before preview)"
            );
        },
    );
}

fn drain_gtk() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..64 {
        while context.iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
}

fn first_row_columns(view: &impl IsA<gtk::Widget>) -> usize {
    let view = view.as_ref();
    let mut tops = Vec::new();
    let mut child = view.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        let Some(bounds) = widget.compute_bounds(view) else {
            continue;
        };
        if bounds.height() <= 0.0 {
            continue;
        }
        tops.push(bounds.y().round() as i32);
    }
    let Some(min_y) = tops.iter().copied().min() else {
        return 0;
    };
    tops.iter().filter(|y| (*y - min_y).abs() <= 1).count()
}

#[test]
fn folders_lead_the_groups_and_the_rest_are_alphabetical() {
    let mut groups = vec!["Zip archive", "Folder", "JSON document", "audio"];
    groups.sort_by(|left, right| compare_type_groups(left, right));

    assert_eq!(groups, ["Folder", "audio", "JSON document", "Zip archive"]);
}

#[test]
fn empty_model_values_sort_before_known_groups() {
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
fn type_group_sorter_clusters_mime_types_and_keeps_source_order_inside_a_group() {
    gtk_test(
        "ui::browser_modes::tests::type_group_sorter_clusters_mime_types_and_keeps_source_order_inside_a_group",
        || {
            let source = gtk::StringList::new(&[
                &value('f', "notes.json"),
                &value('d', "projects"),
                &value('f', "data.json"),
                &value('f', "readme.md"),
            ]);
            let sorted = gtk::SortListModel::new(Some(source), Some(type_group_sorter()));
            let names: Vec<String> = (0..sorted.n_items())
                .filter_map(|index| {
                    let value = sorted.item(index)?.downcast::<gtk::StringObject>().ok()?;
                    Some(
                        value
                            .string()
                            .split_once('\t')
                            .map(|(_, name)| name.to_string())
                            .unwrap_or_default(),
                    )
                })
                .collect();
            assert_eq!(names.first().map(String::as_str), Some("projects"));
            let json: Vec<_> = names
                .iter()
                .enumerate()
                .filter(|(_, name)| name.ends_with(".json"))
                .collect();
            assert_eq!(json.len(), 2);
            assert_eq!(json[1].0, json[0].0 + 1, "same type stays together");
            assert_eq!(json[0].1.as_str(), "notes.json");
            assert_eq!(json[1].1.as_str(), "data.json");
            assert!(names.iter().any(|name| name == "readme.md"));
        },
    );
}

const GTK_CHILD: &str = "STRATA_SOURCE_INDEX_MAP_GTK_CHILD";
const SOURCE_INDEX_TEST: &str =
    "ui::browser_modes::tests::source_index_map_tracks_filter_sort_and_non_source_items";
const LIST_ROW_GTK_CHILD: &str = "STRATA_LIST_ROW_GTK_CHILD";
const LIST_ROW_TEST: &str = "ui::browser_modes::tests::list_bind_can_read_the_rename_field";

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

    let prefix = gtk::StringList::new(&["decoration"]);
    let stacked = gio::ListStore::new::<gio::ListModel>();
    stacked.append(&prefix.clone().upcast::<gio::ListModel>());
    stacked.append(&source.clone().upcast::<gio::ListModel>());
    let flattened = gtk::FlattenListModel::new(Some(stacked));
    assert!(
        map.of_view_position(&flattened, 0).is_none(),
        "the synthetic prefix is not a source entry"
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

    let positions = |view: &gio::ListModel| super::PanePositions {
        index: map.clone(),
        view: view.clone(),
    };
    let unfiltered = positions(source.upcast_ref());
    assert_eq!(unfiltered.view_position(4), Some(4));
    assert_eq!(unfiltered.source_position(4), Some(4));
    assert_eq!(
        positions(flattened.upcast_ref()).view_position(0),
        Some(1),
        "a leading non-source item shifts every row by one"
    );
    assert_eq!(positions(visible.upcast_ref()).view_position(3), Some(2));
    assert_eq!(
        positions(visible.upcast_ref()).view_position(2),
        None,
        "an anchor the filter hides has no row to range from"
    );
    let reverse_sorted = positions(sorted.upcast_ref());
    let anchor_row = reverse_sorted
        .view_position(0)
        .expect("the first source entry has a sorted row");
    assert_ne!(anchor_row, 0, "reverse sort moves that entry off row 0");
    assert_eq!(
        reverse_sorted.source_position(anchor_row),
        Some(0),
        "the range anchor follows its entry through a re-sort"
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

#[test]
fn list_bind_can_read_the_rename_field() {
    if std::env::var_os(LIST_ROW_GTK_CHILD).is_some() {
        if gtk::init().is_err() {
            return;
        }
        let row = super::assemble_list_row();
        let (_, name, field, _, _, _, _) =
            super::list_row_parts(&row).expect("bind and settle walk this row");
        assert!(name.has_css_class("alternate-rename-label"));
        assert!(field.has_css_class("inline-rename"));
        return;
    }

    let status = Command::new(std::env::current_exe().expect("test executable should exist"))
        .args(["--exact", LIST_ROW_TEST])
        .env(LIST_ROW_GTK_CHILD, "1")
        .status()
        .expect("isolated GTK list row test should start");
    assert!(status.success(), "isolated GTK list row test failed");
}

#[test]
fn source_index_map_tracks_filter_sort_and_non_source_items() {
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

#[test]
fn icons_scrolling_bind_still_requests_thumbnail_and_settle_fills_chrome() {
    gtk_test(
        "ui::browser_modes::tests::icons_scrolling_bind_still_requests_thumbnail_and_settle_fills_chrome",
        || {
            crate::ui::theme::ThemeManager::shared();
            crate::ui::thumbnail::hold_thumbnail_workers();
            let path = PathBuf::from("/fixture/icons-scroll.png");
            let entry = FileEntry {
                location: Location::local(&path),
                thumbnail_path: None,
                native_name: "icons-scroll.png".into(),
                display_name: "icons-scroll.png".into(),
                kind: EntryKind::File,
                size: MetadataValue::Known(1),
                modified_unix_seconds: MetadataValue::Known(1),
                mode: MetadataValue::Known(0o100644),
                is_hidden: false,
            };
            let card = crate::ui::icons_cell::new_card(64);
            super::apply_icons_entry(None, &card, &entry, &HashSet::new(), 64, true, None);
            assert!(crate::ui::icons_cell::rename_field(&card).is_none());
            let (icon, label) = crate::ui::icons_cell::parts(&card).expect("icons card");
            assert!(label.tooltip_text().is_none());
            assert!(!card.has_css_class("cut"));
            let context = gtk::glib::MainContext::default();
            for _ in 0..64 {
                if !context.iteration(false) {
                    break;
                }
            }
            assert!(
                !crate::ui::thumbnail::has_pending_thumbnail(&path),
                "scrolling bind must not enqueue thumbnail work"
            );
            crate::ui::thumbnail::set_thumbnail_or_icon(
                &icon,
                &entry,
                crate::assets::icons::PICTURES,
                64,
                64,
            );
            for _ in 0..64 {
                if !context.iteration(false) {
                    break;
                }
            }
            assert!(crate::ui::thumbnail::has_pending_thumbnail(&path));
            let job = crate::ui::thumbnail::pending_thumbnail_id(&path);
            let mut cuts = HashSet::new();
            cuts.insert(entry.location.clone());
            super::refresh_icons_card_chrome(None, &card, &icon, &label, &entry, &cuts);
            assert_eq!(label.tooltip_text().as_deref(), Some("icons-scroll.png"));
            assert!(card.has_css_class("cut"));
            assert_eq!(crate::ui::thumbnail::pending_thumbnail_id(&path), job);
            crate::ui::thumbnail::clear_thumbnail_runtime();
        },
    );
}

mod column_widths;
mod rename;

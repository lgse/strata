// SPDX-License-Identifier: MIT

use super::{
    BrowserDensity, BrowserMode, ClickActivation, ClickCount, LIST_COLUMN_MIN_WIDTHS,
    SourceIndexMap, compare_type_groups, compare_type_groups_for_preferences, list_column_width,
    metadata_fill_position, should_activate_filtered_pointer, should_activate_pointer_click,
    type_group_sorter, type_groups_of, value_type_group,
};
use crate::model::{EntryKind, FileEntry, Location, MetadataValue, SortDirection, SortKey};
use crate::test_support::gtk_test;
use gtk::{gio, prelude::*};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

impl super::ModeViews {
    pub(in crate::ui) fn assert_saved_preferences(
        &self,
        manager: &crate::ui::preferences::PreferenceManager,
    ) {
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
        assert_eq!(
            self.icons_thumbnail_size.get(),
            manager.icons_thumbnail_size()
        );
        for pane in &self.icons_panes {
            assert_eq!(
                pane.thumbnail_scale
                    .as_ref()
                    .expect("Icons size control")
                    .value(),
                f64::from(manager.icons_thumbnail_size())
            );
        }
    }

    pub(in crate::ui) fn active_filter_placeholder(&self) -> Option<String> {
        let pane = self
            .browser
            .active_depth()
            .and_then(|depth| self.panes_at(depth).into_iter().next())?;
        pane.filter_entry
            .as_ref()?
            .placeholder_text()
            .map(|text| text.to_string())
    }
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
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    };

    assert_eq!(
        metadata_fill_position(Some(7), &entry, false, false),
        Some(7)
    );
    assert_eq!(metadata_fill_position(None, &entry, false, true), None);

    entry.size = MetadataValue::Known(100);
    entry.modified_unix_seconds = MetadataValue::Known(1);
    assert_eq!(metadata_fill_position(Some(7), &entry, false, false), None);
    assert_eq!(
        metadata_fill_position(Some(7), &entry, false, true),
        Some(7)
    );
    entry.image_dimensions = MetadataValue::Unavailable;
    entry.native_name = "movie.mp4".into();
    assert_eq!(
        metadata_fill_position(Some(7), &entry, false, true),
        Some(7)
    );
    entry.duration_seconds = MetadataValue::Unavailable;
    assert_eq!(
        metadata_fill_position(Some(7), &entry, true, false),
        Some(7)
    );
    entry.mode = MetadataValue::Known(0o100644);
    assert_eq!(metadata_fill_position(Some(7), &entry, true, false), None);
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
    let mut groups = vec!["Zip archive", "Folder", "Other", "JSON document", "audio"];
    groups.sort_by(|left, right| compare_type_groups(left, right));

    assert_eq!(
        groups,
        ["Folder", "audio", "JSON document", "Zip archive", "Other"]
    );
}

#[test]
fn type_groups_follow_type_direction_and_folder_preference() {
    for folders_first in [false, true] {
        for sort_direction in [SortDirection::Ascending, SortDirection::Descending] {
            let preferences = crate::model::ViewPreferences {
                sort_key: SortKey::Type,
                sort_direction,
                folders_first,
                ..Default::default()
            };
            let mut groups = ["Other", "Type 10", "Folder", "Audio", "Type 2"];
            groups.sort_by(|left, right| {
                compare_type_groups_for_preferences(left, right, preferences)
            });
            let mut expected = vec!["Audio", "Folder", "Type 2", "Type 10", "Other"];
            if sort_direction == SortDirection::Descending {
                expected.reverse();
            }
            if folders_first {
                expected.retain(|label| *label != "Folder");
                expected.insert(0, "Folder");
            }
            assert_eq!(groups.as_slice(), expected);
        }
    }
}

#[test]
fn empty_model_values_sort_before_known_groups() {
    assert!(compare_type_groups("", "Folder").is_lt());
    assert!(compare_type_groups("", "JSON document").is_lt());
    assert!(compare_type_groups("", "Other").is_lt());
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
            let sorted =
                gtk::SortListModel::new(Some(source), Some(type_group_sorter(Default::default)));
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

    let sorted_index = SourceIndexMap::watch(&sorted);
    let visible_index = SourceIndexMap::watch(&visible);
    let removed = source.item(0).expect("removed item");
    source.remove(0);
    assert_eq!(sorted_index.of_item(&removed), None);
    assert_eq!(visible_index.of_item(&removed), None);
    source.append("fh\t.hidden");
    let hidden = source.item(source.n_items() - 1).expect("hidden item");
    assert_eq!(visible_index.of_item(&hidden), None);
    visible.set_filter(None::<&gtk::Filter>);
    sorted.set_sorter(None::<&gtk::Sorter>);
    for (model, index) in [
        (sorted.upcast_ref::<gio::ListModel>(), &sorted_index),
        (visible.upcast_ref::<gio::ListModel>(), &visible_index),
    ] {
        for position in 0..model.n_items() {
            assert_eq!(
                index.of_item(&model.item(position).expect("view item")),
                Some(position as usize)
            );
        }
    }

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
fn icons_hover_only_tracks_thumbnail_or_caption_content() {
    gtk_test(
        "ui::browser_modes::tests::icons_hover_only_tracks_thumbnail_or_caption_content",
        || {
            let card = crate::ui::icons_cell::new_card(64);
            let (icon, label) = crate::ui::icons_cell::parts(&card).expect("icons card");
            label.set_text(Some("sample.png"));
            let window = gtk::Window::builder().child(&card).build();
            window.present();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while icon.width() == 0 {
                assert!(std::time::Instant::now() < deadline);
                gtk::glib::MainContext::default().iteration(true);
            }
            let bounds = icon.compute_bounds(&card).expect("icon bounds");
            let y = f64::from(bounds.center().y());
            super::set_icons_content_hover(&card, 2.0, y);
            assert!(!card.has_css_class("content-hover"));
            super::set_icons_content_hover(&card, f64::from(bounds.center().x()), y);
            assert!(card.has_css_class("content-hover"));
            window.close();
        },
    );
}

#[test]
fn continuous_scroll_refreshes_once_per_frame_without_waiting_for_settle() {
    gtk_test(
        "ui::browser_modes::tests::continuous_scroll_refreshes_once_per_frame_without_waiting_for_settle",
        || {
            use std::{
                cell::Cell,
                rc::Rc,
                time::{Duration, Instant},
            };
            let scroll = gtk::ScrolledWindow::new();
            let window = gtk::Window::builder().child(&scroll).build();
            window.present();
            let scrolling = Rc::new(Cell::new(false));
            let frames = Rc::new(RefCell::new(HashSet::new()));
            let frames_for_refresh = frames.clone();
            let scroll_for_refresh = scroll.downgrade();
            super::install_scroll_refresh(&scroll, scrolling.clone(), None, move || {
                let clock = scroll_for_refresh
                    .upgrade()
                    .expect("scroller")
                    .frame_clock()
                    .expect("frame clock");
                assert!(
                    frames_for_refresh
                        .borrow_mut()
                        .insert(clock.frame_counter()),
                    "one refresh per frame"
                );
            });
            let tick = scroll.add_tick_callback(|scroll, _| {
                for _ in 0..4 {
                    scroll
                        .vadjustment()
                        .emit_by_name::<()>("value-changed", &[]);
                }
                gtk::glib::ControlFlow::Continue
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            while frames.borrow().len() < 3 {
                assert!(
                    Instant::now() < deadline,
                    "scrolling postponed presentation updates"
                );
                gtk::glib::MainContext::default().iteration(false);
            }
            tick.remove();
            assert!(!scrolling.get());
            window.destroy();
        },
    );
}

#[test]
fn icons_scrolling_bind_still_requests_thumbnail_and_settle_fills_chrome() {
    gtk_test(
        "ui::browser_modes::tests::icons_scrolling_bind_still_requests_thumbnail_and_settle_fills_chrome",
        || {
            crate::ui::preferences::PreferenceManager::shared();
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
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            };
            let card = crate::ui::icons_cell::new_card(64);
            super::apply_icons_entry(
                None,
                &card,
                &entry,
                &super::ClipboardMarks::default(),
                64,
                true,
                None,
            );
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
                crate::ui::thumbnail::has_pending_thumbnail(&path),
                "scrolling bind must admit viewport-prioritized thumbnail work"
            );
            let initial_job = crate::ui::thumbnail::pending_thumbnail_id(&path);
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
            assert_eq!(
                job, initial_job,
                "an unchanged row must retain its in-flight thumbnail"
            );
            let locations = std::slice::from_ref(&entry.location);
            let marks = super::ClipboardMarks::from_locations(locations, &[]);
            super::refresh_icons_card_chrome(None, &card, &icon, &label, &entry, &marks);
            assert_eq!(label.tooltip_text().as_deref(), Some("icons-scroll.png"));
            assert!(card.has_css_class("cut"));
            assert!(!card.has_css_class("copied"));
            let marks = super::ClipboardMarks::from_locations(locations, locations);
            super::refresh_icons_card_chrome(None, &card, &icon, &label, &entry, &marks);
            assert!(card.has_css_class("cut"), "cut wins over copy");
            assert!(!card.has_css_class("copied"));
            let marks = super::ClipboardMarks::from_locations(&[], locations);
            super::refresh_icons_card_chrome(None, &card, &icon, &label, &entry, &marks);
            assert!(!card.has_css_class("cut"));
            assert!(card.has_css_class("copied"));
            assert_eq!(icon.opacity(), 1.0);
            assert_eq!(crate::ui::thumbnail::pending_thumbnail_id(&path), job);
            crate::ui::thumbnail::clear_thumbnail_runtime();
        },
    );
}

#[test]
fn icons_entry_displays_item_info_for_images_folders_and_files() {
    gtk_test(
        "ui::browser_modes::tests::icons_entry_displays_item_info_for_images_folders_and_files",
        || {
            let mut entry = FileEntry {
                location: Location::local("/fixture/photo.png"),
                thumbnail_path: None,
                native_name: "photo.png".into(),
                display_name: "photo.png".into(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Known(1),
                mode: MetadataValue::Known(0o100644),
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            };
            let card = crate::ui::icons_cell::new_card(64);
            let details = crate::ui::icons_cell::details_label(&card).expect("details label");

            super::apply_icons_entry(
                None,
                &card,
                &entry,
                &super::ClipboardMarks::default(),
                64,
                false,
                None,
            );
            assert!(details.text().is_empty());

            entry.image_dimensions = MetadataValue::Known((1920, 1080));
            super::apply_icons_entry(
                None,
                &card,
                &entry,
                &super::ClipboardMarks::default(),
                64,
                false,
                None,
            );
            assert!(details.is_visible());
            assert_eq!(details.text().as_str(), "1920×1080");

            entry.image_dimensions = MetadataValue::Unavailable;
            entry.size = MetadataValue::Known(2048);
            super::apply_icons_entry(
                None,
                &card,
                &entry,
                &super::ClipboardMarks::default(),
                64,
                false,
                None,
            );
            assert_eq!(details.text().as_str(), "2 kB");

            entry.location = Location::local("/fixture/movie.mp4");
            entry.native_name = "movie.mp4".into();
            entry.display_name = "movie.mp4".into();
            entry.duration_seconds = MetadataValue::Known(83);
            super::apply_icons_entry(
                None,
                &card,
                &entry,
                &super::ClipboardMarks::default(),
                64,
                false,
                None,
            );
            assert_eq!(details.text().as_str(), "1:23");

            let mut folder_entry = FileEntry {
                location: Location::local("/fixture/docs"),
                thumbnail_path: None,
                native_name: "docs".into(),
                display_name: "docs".into(),
                kind: EntryKind::Directory,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Known(1),
                mode: MetadataValue::Known(0o100755),
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                image_dimensions: MetadataValue::Unavailable,
                child_count: MetadataValue::Known(12),
                duration_seconds: MetadataValue::Unknown,
            };
            super::apply_icons_entry(
                None,
                &card,
                &folder_entry,
                &super::ClipboardMarks::default(),
                64,
                false,
                None,
            );
            assert_eq!(details.text().as_str(), "12 items");

            folder_entry.child_count = MetadataValue::Known(1);
            super::apply_icons_entry(
                None,
                &card,
                &folder_entry,
                &super::ClipboardMarks::default(),
                64,
                false,
                None,
            );
            assert_eq!(details.text().as_str(), "1 item");

            folder_entry.child_count = MetadataValue::Known(0);
            super::apply_icons_entry(
                None,
                &card,
                &folder_entry,
                &super::ClipboardMarks::default(),
                64,
                false,
                None,
            );
            assert_eq!(details.text().as_str(), "No items");

            folder_entry.child_count = MetadataValue::Unknown;
            super::apply_icons_entry(
                None,
                &card,
                &folder_entry,
                &super::ClipboardMarks::default(),
                64,
                true,
                None,
            );
            assert!(
                details.text().is_empty(),
                "recycled cards clear old details"
            );
        },
    );
}

mod column_widths;
mod rename;

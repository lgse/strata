// SPDX-License-Identifier: MIT

use super::*;

fn item_name(item: &glib::Object) -> String {
    item.downcast_ref::<gtk::StringObject>()
        .map(|item| item.string().into())
        .unwrap_or_default()
}

#[test]
fn view_positions_map_to_the_source_items_they_show() {
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::view_positions_map_to_the_source_items_they_show",
        || {
            let source = gtk::StringList::new(&["alpha", ".hidden", "bravo", "charlie"]);
            let source_index = SourceIndexMap::watch(&source);
            let show_hidden = Rc::new(Cell::new(false));
            let filter = gtk::CustomFilter::new({
                let show_hidden = show_hidden.clone();
                move |item| show_hidden.get() || !item_name(item).starts_with('.')
            });
            let filtered = gtk::FilterListModel::new(Some(source.clone()), Some(filter.clone()));
            let descending =
                gtk::CustomSorter::new(|left, right| item_name(right).cmp(&item_name(left)).into());
            let view = gtk::SortListModel::new(Some(filtered), Some(descending))
                .upcast::<gio::ListModel>();
            let view_index = SourceIndexMap::watch(&view);
            let shown = |positions: std::ops::Range<u32>| -> Vec<String> {
                view_index
                    .sources_of(&source_index, &view, positions)
                    .into_iter()
                    .map(|position| {
                        let position = position.expect("every shown item has a source");
                        source.string(position as u32).unwrap_or_default().into()
                    })
                    .collect()
            };

            assert_eq!(shown(0..3), ["charlie", "bravo", "alpha"]);
            // A filtered-out arrival moves source positions without touching the view.
            source.splice(0, 0, &[".cache"]);
            assert_eq!(shown(0..3), ["charlie", "bravo", "alpha"]);
            source.append("delta");
            assert_eq!(shown(0..4), ["delta", "charlie", "bravo", "alpha"]);
            show_hidden.set(true);
            filter.changed(gtk::FilterChange::LessStrict);
            assert_eq!(
                shown(0..6),
                ["delta", "charlie", "bravo", "alpha", ".hidden", ".cache"]
            );
        },
    );
}

#[test]
fn scroll_refresh_waits_for_scrolling_to_stop() {
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::scroll_refresh_waits_for_scrolling_to_stop",
        || {
            let context = glib::MainContext::default();
            let scroll = gtk::ScrolledWindow::new();
            let adjustment = gtk::Adjustment::new(0.0, 0.0, 10_000.0, 1.0, 10.0, 100.0);
            scroll.set_vadjustment(Some(&adjustment));
            let scrolling = Rc::new(Cell::new(false));
            let last_scroll = Rc::new(Cell::new(std::time::Instant::now()));
            let refreshes = Rc::new(RefCell::new(Vec::new()));
            install_scroll_refresh(&scroll, scrolling.clone(), {
                let last_scroll = last_scroll.clone();
                let refreshes = refreshes.clone();
                move || refreshes.borrow_mut().push(last_scroll.get().elapsed())
            });

            for step in 1..=20 {
                last_scroll.set(std::time::Instant::now());
                adjustment.set_value(f64::from(step) * 10.0);
                assert!(scrolling.get());
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while scrolling.get() {
                assert!(std::time::Instant::now() < deadline, "scroll never settled");
                context.iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }

            assert!(!refreshes.borrow().is_empty());
            assert!(
                refreshes
                    .borrow()
                    .iter()
                    .all(|still| *still >= ScrollSettle::DELAY),
                "refreshed before scrolling was still: {:?}",
                refreshes.borrow()
            );
        },
    );
}

fn typed_entry(name: &str, kind: crate::model::EntryKind) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.into(),
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    }
}

#[test]
fn type_groups_order_chunked_rows_and_follow_sort_preference_changes() {
    use crate::model::{EntryKind, ViewPreferences};
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::type_groups_order_chunked_rows_and_follow_sort_preference_changes",
        || {
            let preferences = Rc::new(Cell::new(Some(ViewPreferences::default())));
            let sorter = TypeGroupSorter::new({
                let preferences = preferences.clone();
                move || preferences.get()
            });
            let source = gtk::StringList::new(&[]);
            let view = gtk::SortListModel::new(Some(source.clone()), Some(sorter.sorter.clone()));
            let publish = |position: u32, entries: &[(&str, EntryKind)]| {
                let values: Vec<_> = entries
                    .iter()
                    .map(|(name, kind)| {
                        super::super::browser::entry_model_value(&typed_entry(name, *kind))
                    })
                    .collect();
                let values: Vec<_> = values.iter().map(String::as_str).collect();
                source.splice(position, 0, &values);
            };
            let shown = || -> Vec<String> {
                (0..view.n_items())
                    .filter_map(|position| view.item(position))
                    .map(|item| {
                        let value = item_name(&item);
                        value
                            .split_once('\t')
                            .map_or(value.clone(), |(_, name)| name.to_owned())
                    })
                    .collect()
            };

            // Rows arrive in filename order over two splices, as publication delivers them.
            publish(
                0,
                &[
                    ("a.png", EntryKind::File),
                    ("b", EntryKind::Directory),
                    ("c.jpg", EntryKind::File),
                ],
            );
            publish(
                3,
                &[
                    ("d", EntryKind::Directory),
                    ("e.png", EntryKind::File),
                    ("socket", EntryKind::Other),
                ],
            );
            assert_eq!(shown(), ["b", "d", "c.jpg", "a.png", "e.png", "socket"]);

            sorter.refresh(preferences.get());
            assert_eq!(shown(), ["b", "d", "c.jpg", "a.png", "e.png", "socket"]);

            preferences.set(Some(ViewPreferences {
                sort_key: SortKey::Type,
                sort_direction: SortDirection::Descending,
                ..ViewPreferences::default()
            }));
            sorter.refresh(preferences.get());
            assert_eq!(shown(), ["b", "d", "socket", "a.png", "e.png", "c.jpg"]);
        },
    );
}

#[test]
fn density_slider_positions_snap_to_presets_and_clamp_to_the_maximum() {
    for (px, density) in [
        (0, BrowserDensity::Custom(0)),
        (4, BrowserDensity::Compact),
        (5, BrowserDensity::Custom(5)),
        (8, BrowserDensity::Airy),
        (200, BrowserDensity::Custom(16)),
    ] {
        assert_eq!(BrowserDensity::from_px(px), density, "content {px} px");
        assert_eq!(density.px(), px.min(16));
    }
    for (px, density) in [
        (0, FrameDensity::Compact),
        (3, FrameDensity::Custom(3)),
        (4, FrameDensity::Airy),
        (200, FrameDensity::Custom(12)),
    ] {
        assert_eq!(FrameDensity::from_px(px), density, "frame {px} px");
        assert_eq!(density.px(), px.min(12));
    }
}

#[test]
fn icons_column_cap_follows_the_slider_side_of_airy() {
    for (px, columns) in [(0, 20), (4, 20), (7, 20), (8, 16), (16, 16)] {
        assert_eq!(
            density_icons_columns(BrowserDensity::from_px(px)),
            columns,
            "{px} px"
        );
    }
}

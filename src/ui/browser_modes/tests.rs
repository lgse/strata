// SPDX-License-Identifier: MIT

use super::*;
use gtk::gdk;

mod counting_model;

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
            let removed = source.item(3).expect("bravo");
            source.splice(3, 1, &["bravo.json"]);
            assert_eq!(view_index.of_item(&removed), None);
            assert_eq!(
                shown(0..6),
                [
                    "delta",
                    "charlie",
                    "bravo.json",
                    "alpha",
                    ".hidden",
                    ".cache"
                ]
            );
            let ascending =
                gtk::CustomSorter::new(|left, right| item_name(left).cmp(&item_name(right)).into());
            view.downcast_ref::<gtk::SortListModel>()
                .expect("sorted view")
                .set_sorter(Some(&ascending));
            assert_eq!(
                shown(0..6),
                [
                    ".cache",
                    ".hidden",
                    "alpha",
                    "bravo.json",
                    "charlie",
                    "delta"
                ]
            );
            show_hidden.set(false);
            filter.changed(gtk::FilterChange::MoreStrict);
            assert_eq!(shown(0..4), ["alpha", "bravo.json", "charlie", "delta"]);
            for position in 0..view.n_items() {
                let item = view.item(position).expect("shown item");
                assert_eq!(view_index.of_item(&item), Some(position as usize));
            }
            source.splice(0, source.n_items(), &[]);
            assert_eq!(view.n_items(), 0);
            source.append("fresh");
            assert_eq!(shown(0..1), ["fresh"]);
        },
    );
}

#[test]
fn index_does_not_keep_a_closed_model_alive_or_resolve_its_old_positions() {
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::index_does_not_keep_a_closed_model_alive_or_resolve_its_old_positions",
        || {
            let model = gtk::StringList::new(&["old"]);
            let item = model.item(0).expect("old item");
            let weak = model.downgrade();
            let index = SourceIndexMap::watch(&model);
            assert_eq!(index.of_item(&item), Some(0));
            drop(model);
            assert!(weak.upgrade().is_none());
            assert_eq!(index.of_item(&item), None);
        },
    );
}

#[test]
fn chunked_type_groups_do_not_eagerly_read_replaced_ranges() {
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::chunked_type_groups_do_not_eagerly_read_replaced_ranges",
        || {
            for total in [512, 4096] {
                let source = gtk::StringList::new(&[]);
                let source_index = SourceIndexMap::watch(&source);
                let sorter =
                    TypeGroupSorter::new(|| Some(crate::model::ViewPreferences::default()));
                let sorted =
                    gtk::SortListModel::new(Some(source.clone()), Some(sorter.sorter.clone()));
                sorted.set_section_sorter(Some(&sorter.sorter));
                let counted = counting_model::CountingModel::new(&sorted);
                let view = counted.clone().upcast::<gio::ListModel>();
                let view_index = SourceIndexMap::watch(&view);
                for start in (0..total).step_by(128) {
                    let values: Vec<_> = (start..start + 128)
                        .map(|i| {
                            let extension = ["png", "txt", "json", "csv"][i % 4];
                            super::super::browser::entry_model_value(&typed_entry(
                                &format!("{i:06}.{extension}"),
                                crate::model::EntryKind::File,
                            ))
                        })
                        .collect();
                    let values: Vec<_> = values.iter().map(String::as_str).collect();
                    source.splice(start as u32, 0, &values);
                }
                let mapped = view_index.sources_of(&source_index, &view, 0..total as u32);
                assert!(mapped.iter().all(Option::is_some));
                for (position, source_position) in mapped.into_iter().enumerate() {
                    assert_eq!(
                        source.item(source_position.expect("mapped item") as u32),
                        sorted.item(position as u32)
                    );
                }
                assert!(
                    counted.reads() <= total * 2,
                    "{total} published rows caused {} view reads",
                    counted.reads()
                );
            }
        },
    );
}

#[test]
fn single_row_selection_does_not_scan_the_whole_view() {
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::single_row_selection_does_not_scan_the_whole_view",
        || {
            let values: Vec<_> = (0..4096).map(|i| format!("{i:06}")).collect();
            let values: Vec<_> = values.iter().map(String::as_str).collect();
            let source = gtk::StringList::new(&values);
            let source_index = SourceIndexMap::watch(&source);
            let sorter =
                gtk::CustomSorter::new(|left, right| item_name(right).cmp(&item_name(left)).into());
            let sorted = gtk::SortListModel::new(Some(source.clone()), Some(sorter));
            let counted = counting_model::CountingModel::new(&sorted);
            let view = counted.clone().upcast::<gio::ListModel>();
            let view_index = SourceIndexMap::watch(&view);
            source.append("zz");
            let before = counted.reads();
            assert_eq!(
                view_index.sources_of(&source_index, &view, std::iter::once(0)),
                [Some(4096)]
            );
            assert!(
                counted.reads() - before <= 2,
                "one selected row caused {} view reads",
                counted.reads() - before
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

pub(super) fn typed_entry(name: &str, kind: crate::model::EntryKind) -> FileEntry {
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
        is_hidden: false,
        recent_uri: None,
    }
}

#[test]
fn filtered_file_clicks_never_open_chooser_or_trashed_files() {
    use ClickCount::{One, Two};
    use FilteredFileClick::{Ignore, Open, Preview};
    let local = typed_entry("note.txt", crate::model::EntryKind::File);
    let trashed = FileEntry {
        location: Location::uri("trash:///note.txt"),
        ..local.clone()
    };
    for (entry, chooser, previews, files, expected) in [
        (&local, false, true, One, Open),
        (&local, false, true, Two, Preview),
        (&local, false, false, Two, Ignore),
        (&local, true, true, One, Preview),
        (&trashed, false, true, One, Preview),
        (&trashed, false, false, One, Ignore),
    ] {
        assert_eq!(
            filtered_file_click(entry, chooser, previews, files),
            expected,
            "{:?} chooser={chooser} previews={previews} files={files:?}",
            entry.location
        );
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

fn collection_with_five_items(grid: bool) -> (gtk::Window, gtk::Widget, gtk::MultiSelection) {
    let model = gtk::StringList::new(&["a", "b", "c", "d", "e"]);
    let selection = gtk::MultiSelection::new(Some(model));
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, object| {
        let item = object.downcast_ref::<gtk::ListItem>().expect("list item");
        let label = gtk::Label::new(Some("Item"));
        label.set_size_request(80, 40);
        item.set_child(Some(&label));
    });
    let view: gtk::Widget = if grid {
        gtk::GridView::builder()
            .model(&selection)
            .factory(&factory)
            .min_columns(5)
            .max_columns(5)
            .build()
            .upcast()
    } else {
        gtk::ListView::new(Some(selection.clone()), Some(factory)).upcast()
    };
    let window = gtk::Window::builder()
        .child(&view)
        .default_width(600)
        .default_height(400)
        .build();
    window.present();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while view.width() == 0 {
        glib::MainContext::default().iteration(false);
        assert!(std::time::Instant::now() < deadline, "view never allocated");
    }
    assert!(view.grab_focus());
    (window, view, selection)
}

fn press_shift_arrow(view: &gtk::Widget, key: gdk::Key) {
    let controllers = view.observe_controllers();
    let shortcut = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::ShortcutController>()
        })
        .flat_map(|controller| {
            (0..controller.n_items())
                .filter_map(move |index| controller.item(index).and_downcast::<gtk::Shortcut>())
        })
        .find(|shortcut| {
            shortcut
                .trigger()
                .and_downcast::<gtk::KeyvalTrigger>()
                .is_some_and(|trigger| {
                    trigger.keyval() == key && trigger.modifiers() == gdk::ModifierType::SHIFT_MASK
                })
        })
        .expect("the collection binds Shift+arrow");
    assert!(shortcut.action().expect("binding action").activate(
        gtk::ShortcutActionFlags::EXCLUSIVE,
        view,
        shortcut.arguments().as_ref(),
    ));
}

#[test]
fn control_click_moves_the_shift_range_origin_to_the_clicked_row() {
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::control_click_moves_the_shift_range_origin_to_the_clicked_row",
        || {
            for (grid, key) in [(false, gdk::Key::Down), (true, gdk::Key::Right)] {
                for (moved, extended) in [(false, vec![0, 1, 2, 3]), (true, vec![2, 3])] {
                    let (window, view, selection) = collection_with_five_items(grid);
                    let syncing = Cell::new(false);
                    view.activate_action(
                        "list.select-item",
                        Some(&(0u32, false, false).to_variant()),
                    )
                    .expect("select the first row");
                    if let Some(list) = view.downcast_ref::<gtk::ListView>() {
                        list.scroll_to(2, gtk::ListScrollFlags::FOCUS, None);
                    } else if let Some(grid) = view.downcast_ref::<gtk::GridView>() {
                        grid.scroll_to(2, gtk::ListScrollFlags::FOCUS, None);
                    }
                    let both = gtk::Bitset::new_empty();
                    both.add(0);
                    both.add(2);
                    selection.set_selection(&both, &gtk::Bitset::new_range(0, 5));
                    if moved {
                        move_range_origin(&view, &selection, &syncing, 2);
                    }
                    assert_eq!(
                        bitset_positions(&selection.selection()),
                        [0, 2],
                        "grid {grid} moved {moved}"
                    );

                    press_shift_arrow(&view, key);
                    assert_eq!(
                        bitset_positions(&selection.selection()),
                        extended,
                        "grid {grid} moved {moved}"
                    );
                    window.close();
                }
            }
        },
    );
}

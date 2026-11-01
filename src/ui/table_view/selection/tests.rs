// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn selection_highlight_shrinks_and_clears_without_erasing_markup() {
    crate::test_support::gtk_test(
        "ui::table_view::selection::tests::selection_highlight_shrinks_and_clears_without_erasing_markup",
        || {
            for markup in [false, true] {
                let label = gtk::Label::new(None);
                let fresh = gtk::Label::new(None);
                for target in [&label, &fresh] {
                    target.set_selectable(true);
                    if markup {
                        target.set_markup("<b>bold</b> &amp; plain");
                    } else {
                        target.set_text("plain <&> text");
                    }
                }
                let original_text = label.text();
                let original_attributes =
                    label.layout().attributes().map(|attrs| attrs.to_string());
                let full = gtk::pango::AttrList::new();
                full.insert(gtk::pango::AttrColor::new_background(0, 65535, 0));
                replace_highlight(&label, Some(&full));
                assert_ne!(
                    label.layout().attributes().map(|attrs| attrs.to_string()),
                    original_attributes
                );
                let partial = gtk::pango::AttrList::new();
                let mut background = gtk::pango::AttrColor::new_background(0, 65535, 0);
                background.set_start_index(1);
                background.set_end_index(3);
                partial.insert(background);
                fresh.set_attributes(Some(&partial));
                replace_highlight(&label, Some(&partial));
                assert_eq!(
                    label.layout().attributes().map(|attrs| attrs.to_string()),
                    fresh.layout().attributes().map(|attrs| attrs.to_string())
                );
                replace_highlight(&label, None);
                assert_eq!(
                    label.layout().attributes().map(|attrs| attrs.to_string()),
                    original_attributes
                );
                assert_eq!(label.text(), original_text);
            }
        },
    );
}

#[test]
fn retained_selections_follow_live_theme_in_two_windows_and_rebuilt_tables() {
    crate::test_support::gtk_test(
        "ui::table_view::selection::tests::retained_selections_follow_live_theme_in_two_windows_and_rebuilt_tables",
        || {
            let themes = crate::ui::theme::ThemeManager::shared();
            let context = glib::MainContext::default();
            let fixture = || {
                let table = TableState::new(vec![vec![DocumentTableCellLayout {
                    text: "value".into(),
                    header: false,
                    spans: Vec::new(),
                }]]);
                let selection = Selection::new(table.clone());
                selection.range.set(Some(Range {
                    anchor: Point {
                        row: 0,
                        column: 0,
                        offset: 0,
                    },
                    focus: Point {
                        row: 0,
                        column: 0,
                        offset: 5,
                    },
                }));
                let factory = gtk::SignalListItemFactory::new();
                factory.connect_setup(|_, item| {
                    item.downcast_ref::<gtk::ListItem>()
                        .expect("list item")
                        .set_child(Some(&gtk::Label::new(Some("value"))));
                });
                let for_bind = selection.clone();
                factory.connect_bind(move |_, item| {
                    let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                    let label = item.child().and_downcast::<gtk::Label>().expect("cell");
                    for_bind.bind(&label, item, 0);
                });
                let view =
                    gtk::ColumnView::new(Some(gtk::NoSelection::new(Some(table.model.clone()))));
                view.append_column(&gtk::ColumnViewColumn::new(Some("Field"), Some(factory)));
                let scroll = gtk::ScrolledWindow::builder().child(&view).build();
                selection.install(&view, &scroll);
                let window = gtk::Window::builder().child(&scroll).build();
                window.present();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                let cell = loop {
                    while context.pending() {
                        context.iteration(false);
                    }
                    if let Some(cell) = descendant(window.upcast_ref(), &|label: &gtk::Label| {
                        label.text() == "value"
                    }) {
                        break cell;
                    }
                    assert!(std::time::Instant::now() < deadline, "selected cell bound");
                    std::thread::sleep(std::time::Duration::from_millis(2));
                };
                (window, cell, selection)
            };
            let colors = |label: &gtk::Label| {
                let mut colors = label
                    .layout()
                    .attributes()
                    .into_iter()
                    .flat_map(|attributes| attributes.attributes())
                    .filter_map(|attribute| {
                        attribute
                            .downcast_ref::<gtk::pango::AttrColor>()
                            .map(|color| color.color().to_str().to_string())
                    })
                    .collect::<Vec<_>>();
                colors.sort();
                colors
            };
            let mut windows = vec![fixture(), fixture()];
            let original = colors(&windows[0].1);
            assert!(
                !original.is_empty(),
                "selection is highlighted before Settings opens"
            );
            assert_eq!(colors(&windows[1].1), original);
            let mut preview = themes.starter_tokens();
            preview.accent = "#ff0000".into();
            preview.background = "#0000ff".into();
            themes.preview(&preview).expect("valid live theme preview");
            while context.pending() {
                context.iteration(false);
            }
            let changed = colors(&windows[0].1);
            assert_ne!(
                changed, original,
                "theme preview recolors the retained selection"
            );
            assert_eq!(colors(&windows[1].1), changed, "both windows update");
            windows[0].0.close();
            windows[0] = fixture();
            assert_eq!(
                colors(&windows[0].1),
                changed,
                "rebuilt tables use the active theme"
            );
            themes.cancel_preview();
            while context.pending() {
                context.iteration(false);
            }
            for (window, label, selection) in windows {
                assert_eq!(
                    colors(&label),
                    original,
                    "cancelling restores the selection colors"
                );
                assert_eq!(selection.copy_text().as_deref(), Some("value"));
                window.close();
            }
        },
    );
}

#[test]
fn selection_copies_sorted_rows_partial_unicode_and_ragged_cells() {
    crate::test_support::gtk_test(
        "ui::table_view::selection::tests::selection_copies_sorted_rows_partial_unicode_and_ragged_cells",
        || {
            let cell = |text: &str| DocumentTableCellLayout {
                text: text.into(),
                header: false,
                spans: Vec::new(),
            };
            let table = TableState::new(vec![
                vec![cell("beta"), cell("long value")],
                vec![cell("alpha"), cell("é🙂文")],
                vec![cell("gamma")],
            ]);
            let widget = table.widget();
            let selection = Selection::new(table);
            for (anchor, focus, expected) in [
                (
                    Point {
                        row: 0,
                        column: 1,
                        offset: 1,
                    },
                    Point {
                        row: 1,
                        column: 1,
                        offset: 4,
                    },
                    "🙂文\nbeta\tlong",
                ),
                (
                    Point {
                        row: 1,
                        column: 1,
                        offset: 4,
                    },
                    Point {
                        row: 0,
                        column: 1,
                        offset: 1,
                    },
                    "🙂文\nbeta\tlong",
                ),
                (
                    Point {
                        row: 0,
                        column: 0,
                        offset: 2,
                    },
                    Point {
                        row: 2,
                        column: 1,
                        offset: 0,
                    },
                    "pha\té🙂文\nbeta\tlong value\ngamma\t",
                ),
            ] {
                selection.range.set(Some(Range { anchor, focus }));
                assert_eq!(selection.copy_text().as_deref(), Some(expected));
            }
            selection.clear();
            assert!(selection.copy_text().is_none());
            drop(widget);
            let sparse = Selection::new(TableState::new(vec![
                vec![cell("a")],
                vec![],
                vec![cell("z"), cell("tail")],
            ]));
            sparse.range.set(Some(Range {
                anchor: Point {
                    row: 0,
                    column: 0,
                    offset: 0,
                },
                focus: Point {
                    row: 2,
                    column: 1,
                    offset: 0,
                },
            }));
            assert_eq!(sparse.copy_text().as_deref(), Some("a\nz\ttail\n\t"));
        },
    );
}

#[test]
fn select_all_and_unselect_all_clear_the_cell_range_exactly() {
    crate::test_support::gtk_test(
        "ui::table_view::selection::tests::select_all_and_unselect_all_clear_the_cell_range_exactly",
        || {
            use gtk::gdk::{Key, ModifierType as Modifiers};
            let table = TableState::new(vec![vec![DocumentTableCellLayout {
                text: "value".into(),
                header: false,
                spans: Vec::new(),
            }]]);
            let view = gtk::ColumnView::new(None::<gtk::NoSelection>);
            let scroll = gtk::ScrolledWindow::builder().child(&view).build();
            let selection = Selection::new(table);
            selection.install(&view, &scroll);
            let keys = view
                .observe_controllers()
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
                .expect("selection key controller");
            let point = |offset| Point {
                row: 0,
                column: 0,
                offset,
            };
            for (key, modifiers, clears) in [
                (Key::a, Modifiers::CONTROL_MASK, true),
                (
                    Key::A,
                    Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK,
                    true,
                ),
                (Key::a, Modifiers::CONTROL_MASK | Modifiers::ALT_MASK, false),
            ] {
                selection.range.set(Some(Range {
                    anchor: point(0),
                    focus: point(5),
                }));
                assert!(
                    !keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]),
                    "{modifiers:?}+{key:?} passes on to the focused cell"
                );
                assert_eq!(
                    selection.copy_text().is_none(),
                    clears,
                    "{modifiers:?}+{key:?}"
                );
            }
        },
    );
}

#[test]
fn fallback_copy_and_select_all_run_the_focused_cells_own_actions() {
    crate::test_support::gtk_test(
        "ui::table_view::selection::tests::fallback_copy_and_select_all_run_the_focused_cells_own_actions",
        || {
            use gtk::gdk::{Key, ModifierType as Modifiers};
            let table = TableState::new(vec![vec![DocumentTableCellLayout {
                text: "value".into(),
                header: false,
                spans: Vec::new(),
            }]]);
            let window = gtk::Window::builder().child(&table.widget()).build();
            window.present();
            let context = glib::MainContext::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let cell = loop {
                while context.pending() {
                    context.iteration(false);
                }
                if let Some(cell) = descendant(window.upcast_ref(), &|label: &gtk::Label| {
                    label.text() == "value"
                }) {
                    break cell;
                }
                assert!(std::time::Instant::now() < deadline, "table cell bound");
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            cell.grab_focus();
            let view =
                descendant(window.upcast_ref(), &|_: &gtk::ColumnView| true).expect("table view");
            let keys = view
                .observe_controllers()
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
                .find(|controller| controller.propagation_phase() == gtk::PropagationPhase::Capture)
                .expect("cell selection key controller");
            // Without a Latin layout, these keys read as Ctrl+A and Ctrl+C by position.
            let press = |key: Key, keycode: u32| {
                keys.emit_by_name::<bool>(
                    "key-pressed",
                    &[&key, &keycode, &Modifiers::CONTROL_MASK],
                )
            };

            assert!(press(Key::Cyrillic_ef, 38), "Ctrl+A stays in the cell");
            assert_eq!(cell.selection_bounds(), Some((0, 5)));
            let clipboard = cell.clipboard();
            clipboard.set_text("before");
            assert!(press(Key::Cyrillic_es, 54), "Ctrl+C stays in the cell");
            assert_eq!(
                context
                    .block_on(clipboard.read_text_future())
                    .ok()
                    .flatten()
                    .as_deref(),
                Some("value")
            );
            window.close();
        },
    );
}

fn descendant<T: IsA<gtk::Widget> + glib::object::IsClass>(
    root: &gtk::Widget,
    accept: &impl Fn(&T) -> bool,
) -> Option<T> {
    if let Some(widget) = root.downcast_ref::<T>()
        && accept(widget)
    {
        return Some(widget.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = descendant(&widget, accept) {
            return Some(found);
        }
    }
    None
}

#[test]
fn unmapping_releases_the_window_dismissal_listener() {
    crate::test_support::gtk_test(
        "ui::table_view::selection::tests::unmapping_releases_the_window_dismissal_listener",
        || {
            let table = TableState::new(vec![vec![DocumentTableCellLayout {
                text: "value".into(),
                header: false,
                spans: Vec::new(),
            }]]);
            let view = gtk::ColumnView::new(None::<gtk::NoSelection>);
            let scroll = gtk::ScrolledWindow::builder().child(&view).build();
            let selection = Selection::new(table);
            selection.install(&view, &scroll);
            let window = gtk::Window::builder().child(&scroll).build();
            window.present();
            let context = glib::MainContext::default();
            while context.pending() {
                context.iteration(false);
            }
            selection.range.set(Some(Range {
                anchor: Point {
                    row: 0,
                    column: 0,
                    offset: 0,
                },
                focus: Point {
                    row: 0,
                    column: 0,
                    offset: 5,
                },
            }));
            assert_eq!(selection.copy_text().as_deref(), Some("value"));
            let controllers = window.observe_controllers();
            let listener = (0..controllers.n_items())
                .filter_map(|index| {
                    controllers
                        .item(index)
                        .and_downcast::<gtk::EventControllerLegacy>()
                })
                .find(|controller| controller.name().as_deref() == Some("table-selection-dismiss"))
                .expect("mapped table installs dismissal routing");
            window.set_child(None::<&gtk::Widget>);
            assert!(
                listener.widget().is_none(),
                "unmapping disconnects window listener"
            );
            window.close();
        },
    );
}

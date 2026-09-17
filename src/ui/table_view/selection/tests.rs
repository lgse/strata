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
                    offset: 4,
                },
            }));
            assert_eq!(sparse.copy_text().as_deref(), Some("a\n\nz\ttail"));
        },
    );
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

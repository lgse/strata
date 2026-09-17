// SPDX-License-Identifier: MIT

use super::*;

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
fn clicking_outside_clears_selection_and_unmapping_releases_the_listener() {
    crate::test_support::gtk_test(
        "ui::table_view::selection::tests::clicking_outside_clears_selection_and_unmapping_releases_the_listener",
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
                .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
                .find(|controller| controller.name().as_deref() == Some("table-selection-dismiss"))
                .expect("mapped table installs dismissal routing");
            listener.emit_by_name::<()>("pressed", &[&1i32, &0.0f64, &0.0f64]);
            assert!(selection.copy_text().is_none());
            window.set_child(None::<&gtk::Widget>);
            assert!(
                listener.widget().is_none(),
                "unmapping disconnects window listener"
            );
            window.close();
        },
    );
}

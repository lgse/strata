// SPDX-License-Identifier: MIT

use super::*;

fn cell(text: &str, header: bool) -> DocumentTableCellLayout {
    DocumentTableCellLayout {
        text: text.into(),
        header,
        spans: Vec::new(),
    }
}

#[test]
fn mixed_sort_keys_form_a_total_order() {
    let values = [
        "", "2", "10", "1x", "NaN", "inf", "-inf", "-5", "-0", "0", "alpha", "Alpha", "é", "É",
    ];
    for a in values {
        for b in values {
            let ab = SortKey::new(a).compare(&SortKey::new(b));
            assert_eq!(ab.reverse(), SortKey::new(b).compare(&SortKey::new(a)));
            for c in values {
                if ab != Ordering::Greater
                    && SortKey::new(b).compare(&SortKey::new(c)) != Ordering::Greater
                {
                    assert_ne!(SortKey::new(a).compare(&SortKey::new(c)), Ordering::Greater);
                }
            }
        }
    }
}

#[test]
fn secondary_columns_break_ties_without_reversing_equal_keys() {
    crate::test_support::gtk_test(
        "ui::table_view::tests::secondary_columns_break_ties_without_reversing_equal_keys",
        || {
            let state = TableState::new(vec![
                vec![cell("name", true), cell("group", true)],
                vec![cell("b", false), cell("1", false)],
                vec![cell("a", false), cell("1", false)],
                vec![cell("A", false), cell("1", false)],
                vec![cell("a", false), cell("2", false)],
                vec![cell("b", false), cell("2", false)],
            ]);
            for (columns, expected) in [
                (
                    [
                        (1, gtk::SortType::Ascending),
                        (0, gtk::SortType::Descending),
                    ],
                    "name\tgroup\nb\t1\na\t1\nA\t1\nb\t2\na\t2\n",
                ),
                (
                    [
                        (1, gtk::SortType::Descending),
                        (0, gtk::SortType::Ascending),
                    ],
                    "name\tgroup\na\t2\nb\t2\na\t1\nA\t1\nb\t1\n",
                ),
                (
                    [(1, gtk::SortType::Ascending), (0, gtk::SortType::Ascending)],
                    "name\tgroup\na\t1\nA\t1\nb\t1\na\t2\nb\t2\n",
                ),
                (
                    [
                        (1, gtk::SortType::Descending),
                        (0, gtk::SortType::Ascending),
                    ],
                    "name\tgroup\na\t2\nb\t2\na\t1\nA\t1\nb\t1\n",
                ),
            ] {
                state.apply_sort(&columns);
                assert_eq!(state.copy_text(), expected);
            }
        },
    );
}

#[test]
fn visiting_every_column_keeps_sort_keys_lazy_and_within_the_actual_cell_budget() {
    crate::test_support::gtk_test(
        "ui::table_view::tests::visiting_every_column_keeps_sort_keys_lazy_and_within_the_actual_cell_budget",
        || {
            let mut rows = vec![vec![cell("heading", true); 256]];
            rows.extend(
                (0..1000)
                    .rev()
                    .map(|index| vec![cell(&index.to_string(), false)]),
            );
            rows.push(
                (0..256)
                    .map(|column| cell(&format!("tail-{column}"), false))
                    .collect(),
            );
            let actual_cells = rows.iter().map(Vec::len).sum::<usize>();
            let body_cells = rows[1..].iter().map(Vec::len).sum::<usize>();
            let state = TableState::new(rows);
            assert!(
                state
                    .column_sort_keys
                    .iter()
                    .skip(1)
                    .all(|keys| keys.get().is_none()),
                "untouched columns must not parse or lowercase their text"
            );
            for column in 1..256 {
                state.apply_sort(&[(column, gtk::SortType::Ascending)]);
            }
            let allocated = state
                .column_sort_keys
                .iter()
                .filter_map(OnceCell::get)
                .map(|keys| match keys {
                    ColumnSortKeys::Dense(keys) => keys.len(),
                    ColumnSortKeys::Sparse(keys) => keys.len(),
                })
                .sum::<usize>();
            assert!(
                allocated <= 2 * actual_cells,
                "cache growth must stay bounded by actual cells, not rows times columns"
            );
            assert_eq!(
                allocated, body_cells,
                "the sparse columns need no missing-cell keys"
            );
            state.apply_sort(&[(255, gtk::SortType::Descending)]);
            assert!(
                state
                    .copy_text()
                    .lines()
                    .last()
                    .expect("descending sparse tail")
                    .starts_with("tail-0\ttail-1\t")
            );
            state.apply_sort(&[(255, gtk::SortType::Ascending)]);
            assert_eq!(state.copy_text().lines().count(), 1002);
            assert!(
                state
                    .copy_text()
                    .lines()
                    .nth(1)
                    .expect("sorted first row")
                    .starts_with("tail-0\ttail-1\t")
            );
        },
    );
}

#[test]
fn empty_header_only_and_empty_first_rows_can_sort_clear_and_rebuild() {
    crate::test_support::gtk_test(
        "ui::table_view::tests::empty_header_only_and_empty_first_rows_can_sort_clear_and_rebuild",
        || {
            for (rows, expected, natural) in [
                (vec![], "", ""),
                (vec![vec![]], "\n", "\n"),
                (vec![vec![cell("name", true)]], "name\n", "name\n"),
                (
                    vec![vec![], vec![cell("value", false)]],
                    "value\n\n",
                    "\nvalue\n",
                ),
            ] {
                let state = TableState::new(rows);
                let widget = state.widget();
                assert_eq!(state.copy_text(), expected);
                state.apply_sort(&[]);
                drop(state.widget());
                assert_eq!(state.copy_text(), natural);
                drop(widget);
            }
        },
    );
}

#[test]
fn row_objects_keep_identity_through_direction_changes_and_outlive_the_table() {
    crate::test_support::gtk_test(
        "ui::table_view::tests::row_objects_keep_identity_through_direction_changes_and_outlive_the_table",
        || {
            for header in [false, true] {
                let mut rows = Vec::new();
                if header {
                    rows.push(vec![cell("name", true)]);
                }
                rows.extend(["b", "a", "A", ""].map(|text| vec![cell(text, false)]));
                let state = TableState::new(rows);
                let row = state
                    .model
                    .item(0)
                    .expect("ascending first row")
                    .downcast::<glib::BoxedAnyObject>()
                    .expect("row index");
                assert_eq!(*row.borrow::<usize>(), usize::from(header) + 1);
                state.apply_sort(&[(0, gtk::SortType::Descending)]);
                assert_eq!(
                    state.copy_text(),
                    if header {
                        "name\n\nb\na\nA\n"
                    } else {
                        "\nb\na\nA\n"
                    }
                );
                assert_eq!(state.model.item(2), Some(row.clone().upcast()));
                assert!(state.model.item(4).is_none());
                assert!(state.model.item(u32::MAX).is_none());
                state.apply_sort(&[(0, gtk::SortType::Ascending)]);
                assert_eq!(state.model.item(0), Some(row.clone().upcast()));
                state.apply_sort(&[]);
                assert_eq!(state.model.item(1), Some(row.clone().upcast()));

                let model = state.model.clone();
                let weak_state = Rc::downgrade(&state);
                drop(state);
                assert!(weak_state.upgrade().is_none());
                assert_eq!(model.item(1), Some(row.clone().upcast()));
                let weak_row = row.downgrade();
                drop(row);
                drop(model);
                assert!(weak_row.upgrade().is_none());
            }
        },
    );
}

#[test]
fn sorting_and_rebuilds_preserve_ragged_rows_without_republishing_unchanged_items() {
    crate::test_support::gtk_test(
        "ui::table_view::tests::sorting_and_rebuilds_preserve_ragged_rows_without_republishing_unchanged_items",
        || {
            let state = TableState::new(vec![
                vec![cell("value", true)],
                vec![cell("1", false)],
                vec![cell("5", false)],
                vec![cell("10", false)],
                vec![cell("3", false)],
                vec![cell("2", false)],
                vec![],
            ]);
            let widget = state.widget();
            assert_eq!(
                state.copy_text(),
                "value\n1\n2\n3\n5\n10\n\n",
                "first column sorts on initial display"
            );
            let scroll = widget
                .last_child()
                .expect("table scroller")
                .downcast::<gtk::ScrolledWindow>()
                .expect("scrolled window");
            let view = scroll
                .child()
                .expect("table view")
                .downcast::<gtk::ColumnView>()
                .expect("column view");
            let column = view
                .columns()
                .item(0)
                .expect("sortable column")
                .downcast::<gtk::ColumnViewColumn>()
                .expect("column");
            view.sort_by_column(Some(&column), gtk::SortType::Descending);
            assert_eq!(state.copy_text(), "value\n\n10\n5\n3\n2\n1\n");

            let changes = Rc::new(RefCell::new(Vec::new()));
            let changes_for_signal = changes.clone();
            state
                .model
                .connect_items_changed(move |_, position, removed, added| {
                    changes_for_signal
                        .borrow_mut()
                        .push((position, removed, added));
                });
            let rebuilt = state.widget();
            assert_eq!(state.copy_text(), "value\n\n10\n5\n3\n2\n1\n");
            view.sort_by_column(Some(&column), gtk::SortType::Descending);
            assert!(
                changes.borrow().is_empty(),
                "rebuild and identical sort leave row bindings intact"
            );
            view.sort_by_column(None, gtk::SortType::Ascending);
            assert_eq!(state.copy_text(), "value\n1\n5\n10\n3\n2\n\n");
            changes.borrow_mut().clear();
            drop((rebuilt, state.widget()));
            assert!(
                changes.borrow().is_empty(),
                "unsorted tables rebuild without changing their model"
            );

            view.sort_by_column(Some(&column), gtk::SortType::Ascending);
            assert_eq!(
                *changes.borrow(),
                [(1, 4, 4)],
                "only the changed middle rows are rebound"
            );
            changes.borrow_mut().clear();
            view.sort_by_column(Some(&column), gtk::SortType::Ascending);
            assert!(
                changes.borrow().is_empty(),
                "an unchanged ascending sort is also a no-op"
            );
        },
    );
}

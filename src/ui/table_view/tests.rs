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
    let values = ["", "2", "10", "1x", "NaN", "-5", "alpha", "Alpha"];
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
fn maximally_ragged_tables_keep_sort_storage_proportional_to_present_values() {
    crate::test_support::gtk_test(
        "ui::table_view::tests::maximally_ragged_tables_keep_sort_storage_proportional_to_present_values",
        || {
            let mut rows = vec![
                vec![cell("header", true); 256],
                vec![cell("10", false); 256],
            ];
            rows.extend((0..99_488).map(|_| vec![cell("2", false)]));
            let state = TableState::new(rows);
            let widget = state.widget();
            assert_eq!(
                state.sort_keys.iter().map(Vec::len).sum::<usize>(),
                100_000,
                "sort caches must not materialize millions of absent ragged cells"
            );
            let scroll = widget
                .last_child()
                .and_downcast::<gtk::ScrolledWindow>()
                .expect("table scroller");
            let view = scroll
                .child()
                .and_downcast::<gtk::ColumnView>()
                .expect("table view");
            let column = view
                .columns()
                .item(255)
                .and_downcast::<gtk::ColumnViewColumn>()
                .expect("last column");
            view.sort_by_column(Some(&column), gtk::SortType::Ascending);
            assert_eq!(
                state.copy_text().lines().nth(1),
                Some(vec!["10"; 256].join("\t").as_str())
            );
            view.sort_by_column(Some(&column), gtk::SortType::Descending);
            assert_eq!(state.copy_text().lines().nth(1), Some("2"));
            assert_eq!(
                state.copy_text().lines().last(),
                Some(vec!["10"; 256].join("\t").as_str())
            );
        },
    );
}

#[test]
fn sorting_preserves_headers_full_copy_and_state_after_recycling() {
    crate::test_support::gtk_test(
        "ui::table_view::tests::sorting_preserves_headers_full_copy_and_state_after_recycling",
        || {
            let state = TableState::new(vec![
                vec![cell("value", true)],
                vec![cell("10", false)],
                vec![cell("2", false)],
                vec![],
            ]);
            let widget = state.widget();
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
            assert_eq!(
                state.copy_text(),
                "value\n2\n10\n\n",
                "first column sorts on initial display"
            );
            view.sort_by_column(Some(&column), gtk::SortType::Descending);
            assert_eq!(state.copy_text(), "value\n\n10\n2\n");
            let weak = view.downgrade();
            drop((column, view, scroll, widget));
            assert!(
                weak.upgrade().is_none(),
                "a recycled table must release its view"
            );
            let widget = state.widget();
            assert_eq!(state.copy_text(), "value\n\n10\n2\n");
            drop(widget);
            let weak = Rc::downgrade(&state);
            drop(state);
            assert!(weak.upgrade().is_none());
        },
    );
}

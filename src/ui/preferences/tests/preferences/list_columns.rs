// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn list_column_visibility_preserves_default_sort_and_publishes_once() {
    gtk_test(
        "ui::preferences::tests::preferences::list_columns::list_column_visibility_preserves_default_sort_and_publishes_once",
        || {
            let manager = PreferenceManager::shared();
            let snapshots = Rc::new(RefCell::new(Vec::new()));
            let observed = snapshots.clone();
            manager.observe(Rc::new(move || {
                let manager = PreferenceManager::shared();
                observed
                    .borrow_mut()
                    .push((manager.default_sort(), manager.list_hidden_columns()));
            }));

            for (sort_key, column, direction) in [
                (SortKey::Size, ListColumnId::Size, SortDirection::Ascending),
                (SortKey::Type, ListColumnId::Type, SortDirection::Descending),
                (
                    SortKey::Modified,
                    ListColumnId::Modified,
                    SortDirection::Descending,
                ),
            ] {
                manager.reset_list_columns();
                manager.set_default_sort(sort_key, direction);
                for (visible, hidden) in
                    [(false, vec![column]), (true, vec![]), (false, vec![column])]
                {
                    snapshots.borrow_mut().clear();
                    manager.set_list_column_visible(column, visible);

                    assert_eq!(manager.is_list_column_visible(column), visible);
                    assert_eq!(manager.default_sort(), (sort_key, direction));
                    assert_eq!(
                        snapshots.borrow().as_slice(),
                        &[((sort_key, direction), hidden.clone())],
                        "visibility must publish once without changing the sort"
                    );
                    crate::storage::writer::settle();
                    let saved = read_preferences().expect("saved preferences");
                    assert_eq!(saved.list_hidden_columns, hidden);
                    assert_eq!(SortKey::from_stored(&saved.sort_key), Some(sort_key));
                    assert_eq!(
                        SortDirection::from_stored(&saved.sort_direction),
                        Some(direction)
                    );
                }
                snapshots.borrow_mut().clear();
                manager.reset_list_columns();
                assert_eq!(manager.default_sort(), (sort_key, direction));
                assert_eq!(
                    snapshots.borrow().as_slice(),
                    &[((sort_key, direction), vec![])],
                    "resetting columns must preserve sorting too"
                );
            }
        },
    );
}

#[test]
fn list_columns_order_and_visibility_persist_and_update() {
    gtk_test(
        "ui::preferences::tests::preferences::list_columns::list_columns_order_and_visibility_persist_and_update",
        || {
            let manager = PreferenceManager::shared();

            assert_eq!(
                manager.list_column_order(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Mode,
                    ListColumnId::Size,
                    ListColumnId::Type,
                    ListColumnId::Modified,
                ]
            );
            assert!(manager.list_hidden_columns().is_empty());
            assert!(manager.is_list_column_visible(ListColumnId::Mode));

            manager.set_list_column_visible(ListColumnId::Mode, false);
            manager.set_list_column_visible(ListColumnId::Type, false);
            assert!(!manager.is_list_column_visible(ListColumnId::Mode));
            assert!(!manager.is_list_column_visible(ListColumnId::Type));
            assert!(manager.is_list_column_visible(ListColumnId::Name));
            assert!(manager.is_list_column_visible(ListColumnId::Size));
            assert_eq!(
                manager.list_visible_columns(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Size,
                    ListColumnId::Modified,
                ]
            );

            manager.set_list_column_visible(ListColumnId::Name, false);
            assert!(manager.is_list_column_visible(ListColumnId::Name));

            manager.move_list_column(ListColumnId::Modified, ListColumnId::Type, false);
            assert_eq!(
                manager.list_column_order(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Mode,
                    ListColumnId::Size,
                    ListColumnId::Modified,
                    ListColumnId::Type,
                ]
            );
            use ListColumnId::{Mode, Modified, Name, Size, Type};
            for (column, target, after, expected) in [
                (Name, Type, true, [Mode, Size, Modified, Type, Name]),
                (Name, Mode, false, [Name, Mode, Size, Modified, Type]),
                (Type, Name, false, [Type, Name, Mode, Size, Modified]),
                (Type, Modified, true, [Name, Mode, Size, Modified, Type]),
                (Size, Modified, false, [Name, Mode, Size, Modified, Type]),
                (Modified, Modified, true, [Name, Mode, Size, Modified, Type]),
            ] {
                manager.move_list_column(column, target, after);
                assert_eq!(manager.list_column_order(), expected);
            }
            crate::storage::writer::settle();

            let saved = read_preferences().expect("saved preferences");
            assert_eq!(saved.list_column_order, manager.list_column_order());
            assert_eq!(saved.list_hidden_columns, manager.list_hidden_columns());

            manager.reset_list_columns();
            assert_eq!(
                manager.list_column_order(),
                vec![
                    ListColumnId::Name,
                    ListColumnId::Mode,
                    ListColumnId::Size,
                    ListColumnId::Type,
                    ListColumnId::Modified,
                ]
            );
            assert!(manager.list_hidden_columns().is_empty());
            crate::storage::writer::settle();
            let saved_reset = read_preferences().expect("saved preferences");
            assert_eq!(saved_reset.list_column_order, manager.list_column_order());
            assert!(saved_reset.list_hidden_columns.is_empty());
        },
    );
}

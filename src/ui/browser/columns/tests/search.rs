// SPDX-License-Identifier: MIT

use super::*;
use crate::{services::SearchItem, test_support::gtk_test};

fn items(names: &[&str]) -> Vec<SearchItem> {
    names
        .iter()
        .map(|name| SearchItem::for_test(Path::new("/fixture").join(name), false))
        .collect()
}

#[test]
fn search_updates_retain_interior_objects_and_all_surviving_selections() {
    gtk_test(
        "ui::browser::columns::tests::search::search_updates_retain_interior_objects_and_all_surviving_selections",
        || {
            let model = gtk::StringList::new(&["a", "b", "c", "d", "e"]);
            let filtered = gtk::FilterListModel::new(Some(model.clone()), None::<gtk::Filter>);
            let selection = gtk::MultiSelection::new(Some(filtered.clone()));
            let results = Rc::new(RefCell::new(items(&["a", "b", "c", "d", "e"])));
            let syncing = Cell::new(false);
            let a = model.item(0).expect("first result");
            let c = model.item(2).expect("interior result");
            let e = model.item(4).expect("last result");
            selection.select_item(0, false);
            selection.select_item(2, false);
            selection.select_item(3, false);
            selection.select_item(4, false);
            let notifications = Rc::new(RefCell::new(Vec::new()));
            let observed = notifications.clone();
            let current_results = results.clone();
            filtered.connect_items_changed(move |model, position, removed, added| {
                observed.borrow_mut().push((position, removed, added));
                let results = current_results.borrow();
                assert_eq!(model.n_items() as usize, results.len());
                for (position, item) in results.iter().enumerate() {
                    assert_eq!(
                        model
                            .item(position as u32)
                            .expect("notified result")
                            .downcast::<gtk::StringObject>()
                            .expect("string result")
                            .string(),
                        item.name
                    );
                }
            });
            super::super::search::update_results(
                &model,
                &results,
                &selection,
                &syncing,
                items(&["a", "c", "e"]),
            );
            assert_eq!(model.item(0).as_ref(), Some(&a));
            assert_eq!(model.item(1).as_ref(), Some(&c));
            assert_eq!(model.item(2).as_ref(), Some(&e));
            assert_eq!(bitset_positions(&selection.selection()), vec![0, 1, 2]);
            assert_eq!(*notifications.borrow(), vec![(1, 1, 0), (2, 1, 0)]);
            assert!(!syncing.get());
            notifications.borrow_mut().clear();
            super::super::search::update_results(
                &model,
                &results,
                &selection,
                &syncing,
                items(&["a", "c", "e"]),
            );
            assert!(notifications.borrow().is_empty());
            assert_eq!(bitset_positions(&selection.selection()), vec![0, 1, 2]);
            super::super::search::update_results(
                &model,
                &results,
                &selection,
                &syncing,
                items(&["a", "b", "c", "d", "e"]),
            );
            assert_eq!(model.item(0).as_ref(), Some(&a));
            assert_eq!(model.item(2).as_ref(), Some(&c));
            assert_eq!(model.item(4).as_ref(), Some(&e));
            assert_eq!(bitset_positions(&selection.selection()), vec![0, 2, 4]);
        },
    );
}

#[test]
fn search_updates_handle_reordering_metadata_replacement_and_empty_results() {
    gtk_test(
        "ui::browser::columns::tests::search::search_updates_handle_reordering_metadata_replacement_and_empty_results",
        || {
            let model = gtk::StringList::new(&[]);
            let selection = gtk::MultiSelection::new(Some(model.clone()));
            let results = RefCell::new(Vec::new());
            let syncing = Cell::new(false);
            for names in [
                vec!["a", "b", "c"],
                vec!["c", "a", "b"],
                vec!["other/a", "b"],
                vec![],
            ] {
                let expected = items(&names);
                super::super::search::update_results(
                    &model,
                    &results,
                    &selection,
                    &syncing,
                    expected.clone(),
                );
                assert_eq!(*results.borrow(), expected);
                assert_eq!(model.n_items() as usize, expected.len());
                for (position, item) in expected.iter().enumerate() {
                    assert_eq!(
                        model.string(position as u32).as_deref(),
                        Some(item.name.as_str())
                    );
                }
            }
            let mut changed = items(&["a"]);
            super::super::search::update_results(
                &model,
                &results,
                &selection,
                &syncing,
                changed.clone(),
            );
            let old = model.item(0).expect("original result");
            changed[0].is_directory = true;
            super::super::search::update_results(
                &model,
                &results,
                &selection,
                &syncing,
                changed.clone(),
            );
            assert_ne!(model.item(0).expect("updated result"), old);
            assert_eq!(*results.borrow(), changed);
            changed[0].name = "renamed".into();
            super::super::search::update_results(&model, &results, &selection, &syncing, changed);
            assert_eq!(model.string(0).as_deref(), Some("renamed"));
        },
    );
}

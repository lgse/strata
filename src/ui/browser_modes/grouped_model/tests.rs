// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc};

use super::*;
use crate::model::{EntryKind, SortDirection, SortKey, ViewPreferences};
use crate::ui::browser_modes::{TypeGroupSorter, tests::typed_entry};

fn value(name: &str, kind: EntryKind) -> String {
    crate::ui::browser::entry_model_value(&typed_entry(name, kind))
}

fn splice(source: &gtk::StringList, position: u32, removed: u32, entries: &[(&str, EntryKind)]) {
    let values: Vec<_> = entries
        .iter()
        .map(|(name, kind)| value(name, *kind))
        .collect();
    let values: Vec<_> = values.iter().map(String::as_str).collect();
    source.splice(position, removed, &values);
}

fn compare(actual: &GroupedModel, expected: &gtk::SortListModel) {
    assert_eq!(actual.n_items(), expected.n_items());
    for position in 0..actual.n_items() {
        let item = actual.item(position).expect("projected row");
        assert_eq!(
            Some(item.clone()),
            expected.item(position),
            "row {position}"
        );
        assert_eq!(
            actual.position_of(&item),
            Some(position),
            "reverse lookup at {position}"
        );
        let (start, end) = actual.section(position);
        let (expected_start, expected_end) = expected.section(position);
        assert_eq!(
            (start, end.min(actual.n_items())),
            (expected_start, expected_end.min(expected.n_items())),
            "section at {position}"
        );
    }
}

#[test]
fn projection_matches_gtk_grouping_through_filter_sort_and_source_changes() {
    crate::test_support::gtk_test(
        "ui::browser_modes::grouped_model::tests::projection_matches_gtk_grouping_through_filter_sort_and_source_changes",
        || {
            let source = gtk::StringList::new(&[]);
            let show_hidden = Rc::new(Cell::new(false));
            let query = Rc::new(RefCell::new(String::new()));
            let filter = crate::ui::browser::entry_filter(show_hidden.clone(), query.clone());
            let filtered = gtk::FilterListModel::new(Some(source.clone()), Some(filter.clone()));
            let preferences = Rc::new(Cell::new(Some(ViewPreferences::default())));
            let sorter = TypeGroupSorter::new({
                let preferences = preferences.clone();
                move || preferences.get()
            });
            let expected =
                gtk::SortListModel::new(Some(filtered.clone()), Some(sorter.sorter.clone()));
            expected.set_section_sorter(Some(&sorter.sorter));
            let actual = GroupedModel::new(&filtered, Some(&sorter.sorter));
            splice(
                &source,
                0,
                0,
                &[
                    ("a.png", EntryKind::File),
                    ("b", EntryKind::Directory),
                    ("c.jpg", EntryKind::File),
                    ("d.txt", EntryKind::File),
                ],
            );
            compare(&actual, &expected);
            splice(
                &source,
                4,
                0,
                &[
                    ("e.json", EntryKind::File),
                    ("f.csv", EntryKind::File),
                    ("socket", EntryKind::Other),
                ],
            );
            compare(&actual, &expected);
            let mut hidden = typed_entry(".hidden.csv", EntryKind::File);
            hidden.is_hidden = true;
            source.splice(0, 0, &[&crate::ui::browser::entry_model_value(&hidden)]);
            compare(&actual, &expected);
            show_hidden.set(true);
            filter.changed(gtk::FilterChange::LessStrict);
            compare(&actual, &expected);
            query.replace("png".into());
            filter.changed(gtk::FilterChange::Different);
            compare(&actual, &expected);
            query.replace(String::new());
            filter.changed(gtk::FilterChange::Different);
            compare(&actual, &expected);
            preferences.set(Some(ViewPreferences {
                sort_key: SortKey::Type,
                sort_direction: SortDirection::Descending,
                ..ViewPreferences::default()
            }));
            sorter.refresh(preferences.get());
            compare(&actual, &expected);
            splice(&source, 2, 3, &[("renamed.xml", EntryKind::File)]);
            compare(&actual, &expected);
            actual.set_sorter(None);
            expected.set_section_sorter(None::<&gtk::Sorter>);
            expected.set_sorter(None::<&gtk::Sorter>);
            compare(&actual, &expected);
            splice(&source, 1, 0, &[("inserted.TXT", EntryKind::File)]);
            compare(&actual, &expected);
            source.splice(0, source.n_items(), &[]);
            compare(&actual, &expected);
            actual.set_sorter(Some(&sorter.sorter));
            expected.set_sorter(Some(&sorter.sorter));
            expected.set_section_sorter(Some(&sorter.sorter));
            splice(&source, 0, 0, &[("fresh.csv", EntryKind::File)]);
            compare(&actual, &expected);
        },
    );
}

#[test]
fn chunked_arrivals_preserve_native_selection_without_replacing_old_rows() {
    crate::test_support::gtk_test(
        "ui::browser_modes::grouped_model::tests::chunked_arrivals_preserve_native_selection_without_replacing_old_rows",
        || {
            let source = gtk::StringList::new(&[]);
            let sorter = TypeGroupSorter::new(|| Some(ViewPreferences::default()));
            let model = GroupedModel::new(&source, Some(&sorter.sorter));
            let selection = gtk::MultiSelection::new(Some(model.clone()));
            let removed_total = Rc::new(Cell::new(0));
            let removed = removed_total.clone();
            model.connect_items_changed(move |model, position, gone, added| {
                assert!(position + added <= model.n_items());
                removed.set(removed.get() + gone);
            });
            let mut target = None;
            for start in (0usize..4096).step_by(128) {
                let values: Vec<_> = (start..start + 128)
                    .map(|i| {
                        value(
                            &format!("{i:06}.{}", ["txt", "csv", "json", "png"][i % 4]),
                            EntryKind::File,
                        )
                    })
                    .collect();
                let values: Vec<_> = values.iter().map(String::as_str).collect();
                source.splice(start as u32, 0, &values);
                if start == 0 {
                    let item = source.item(0).expect("first TXT file");
                    let position = (0..model.n_items())
                        .find(|position| model.item(*position).as_ref() == Some(&item))
                        .expect("TXT in view");
                    selection.select_item(position, true);
                    target = Some(item);
                }
                let selected_position = model
                    .position_of(target.as_ref().expect("selected TXT"))
                    .expect("selected TXT remains in the projection");
                assert!(selection.is_selected(selected_position));
                let selected: Vec<_> = (0..selection.n_items())
                    .filter(|position| selection.is_selected(*position))
                    .filter_map(|position| selection.item(position))
                    .collect();
                assert_eq!(selected, [target.clone().expect("selected TXT")]);
            }
            assert_eq!(model.n_items(), 4096);
            assert_eq!(
                removed_total.get(),
                0,
                "arrivals must be insertion-only notifications"
            );
        },
    );
}

#[test]
fn source_changes_from_an_observer_converge_without_retaining_the_model() {
    crate::test_support::gtk_test(
        "ui::browser_modes::grouped_model::tests::source_changes_from_an_observer_converge_without_retaining_the_model",
        || {
            let source = gtk::StringList::new(&[]);
            let sorter = TypeGroupSorter::new(|| Some(ViewPreferences::default()));
            let model = GroupedModel::new(&source, Some(&sorter.sorter));
            let once = Rc::new(Cell::new(false));
            let changed = once.clone();
            let nested_source = source.clone();
            model.connect_items_changed(move |_, _, _, added| {
                if added > 0 && !changed.replace(true) {
                    splice(
                        &nested_source,
                        nested_source.n_items(),
                        0,
                        &[("nested.json", EntryKind::File)],
                    );
                }
            });
            splice(
                &source,
                0,
                0,
                &[
                    ("a.txt", EntryKind::File),
                    ("b.csv", EntryKind::File),
                    ("c.png", EntryKind::File),
                ],
            );
            let expected =
                gtk::SortListModel::new(Some(source.clone()), Some(sorter.sorter.clone()));
            expected.set_section_sorter(Some(&sorter.sorter));
            compare(&model, &expected);
            let replaced = source.item(0).expect("original row");
            splice(&source, 0, 1, &[("a.txt", EntryKind::File)]);
            assert_eq!(
                model.position_of(&replaced),
                None,
                "a replaced object must not resolve to its same-name replacement"
            );
            compare(&model, &expected);
            let weak = model.downgrade();
            drop(model);
            assert!(weak.upgrade().is_none());
            splice(
                &source,
                source.n_items(),
                0,
                &[("after-close.txt", EntryKind::File)],
            );
        },
    );
}

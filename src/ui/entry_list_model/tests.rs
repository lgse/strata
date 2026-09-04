// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    process::Command,
    rc::Rc,
};

use super::EntryListModel;
use gtk::{gio, prelude::*};

const GTK_CHILD: &str = "STRATA_ENTRY_LIST_MODEL_GTK_CHILD";
const TEST_NAME: &str = "ui::entry_list_model::tests::lazy_model_counts_items_splices_and_types";

fn values_model(count: u32) -> (EntryListModel, Rc<Cell<u32>>) {
    let calls = Rc::new(Cell::new(0u32));
    let calls_for_resolve = calls.clone();
    let model = EntryListModel::new(Rc::new(move |position| {
        calls_for_resolve.set(calls_for_resolve.get().saturating_add(1));
        (position < count).then(|| format!("f\tfile-{position:04}"))
    }));
    model.replace(count);
    calls.set(0);
    (model, calls)
}

#[test]
fn lazy_model_counts_items_splices_and_types() {
    if std::env::var_os(GTK_CHILD).is_some() {
        if gtk::init().is_err() {
            return;
        }
        let (items, calls) = values_model(8);
        let item = items
            .item(3)
            .and_downcast::<gtk::StringObject>()
            .expect("position 3 should resolve to a string object");
        assert_eq!(item.string(), "f\tfile-0003");
        let list = items.upcast::<gio::ListModel>();
        list.item(0)
            .and_downcast::<gtk::StringObject>()
            .expect("items should stay string objects");
        assert_eq!(calls.get(), 2);
        return;
    }

    let (model, calls) = values_model(0);
    model.replace(100_000);
    model.splice(10, 0, 500);
    model.splice(0, 100_000, 7);
    assert_eq!(model.n_items(), 507);
    assert_eq!(calls.get(), 0);

    let replacement_events: Rc<RefCell<Vec<(u32, u32, u32)>>> = Default::default();
    let observed = replacement_events.clone();
    model.connect_items_changed(move |_, position, removed, added| {
        observed.borrow_mut().push((position, removed, added));
    });
    model.replace(507);
    assert_eq!(*replacement_events.borrow(), vec![(0, 507, 507)]);
    assert_eq!(calls.get(), 0);

    let seen: Rc<RefCell<Vec<u32>>> = Default::default();
    let observed = seen.clone();
    let items = EntryListModel::new(Rc::new(move |position| {
        observed.borrow_mut().push(position);
        Some(format!("f\tfile-{position:04}"))
    }));
    items.replace(8);
    assert_eq!(items.value(7).as_deref(), Some("f\tfile-0007"));
    assert_eq!(*seen.borrow(), vec![7]);
    assert!(items.item(8).is_none());
    assert!(items.value(8).is_none());
    assert!(items.item(u32::MAX).is_none());
    assert_eq!(*seen.borrow(), vec![7]);

    let missing = EntryListModel::new(Rc::new(|_| None));
    missing.replace(4);
    assert_eq!(missing.n_items(), 4);
    assert!(missing.item(0).is_none());
    assert!(missing.value(0).is_none());

    let (spliced, _) = values_model(0);
    let splice_events: Rc<RefCell<Vec<(u32, u32, u32)>>> = Default::default();
    let observed = splice_events.clone();
    spliced.connect_items_changed(move |_, position, removed, added| {
        observed.borrow_mut().push((position, removed, added));
    });
    spliced.replace(10);
    spliced.splice(2, 0, 3);
    spliced.splice(0, 10, 0);
    spliced.splice(1_000, 500, 2);
    spliced.replace(0);
    assert_eq!(spliced.n_items(), 0);
    assert_eq!(
        *splice_events.borrow(),
        vec![(0, 0, 10), (2, 0, 3), (0, 10, 0), (3, 0, 2), (0, 5, 0)]
    );
    splice_events.borrow_mut().clear();
    spliced.replace(0);
    assert!(splice_events.borrow().is_empty());

    let list = items.upcast::<gio::ListModel>();
    assert_eq!(list.item_type(), gtk::StringObject::static_type());
    assert_eq!(list.n_items(), 8);

    // GTK initialization permanently claims GLib's process-wide default
    // context, so exercise object construction without poisoning other tests.
    let status = Command::new(std::env::current_exe().expect("test executable should exist"))
        .args(["--exact", TEST_NAME])
        .env(GTK_CHILD, "1")
        .status()
        .expect("isolated GTK model test should start");
    assert!(status.success(), "isolated GTK model test failed");
}

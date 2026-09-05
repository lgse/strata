// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn clearing_recursive_search_deactivates_before_rows_rebind() {
    const CHILD: &str = "STRATA_RECURSIVE_SEARCH_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser::tests::recursive_search::clearing_recursive_search_deactivates_before_rows_rebind",
            ])
            .env(CHILD, "1")
            .status()
            .expect("isolated GTK test should start");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }

    let directory_model = EntryListModel::new(Rc::new(|position| {
        (position < 3).then(|| format!("entry-{position}"))
    }));
    directory_model.replace(3);
    let search_model = gtk::StringList::new(&["hit-a", "hit-b"]);
    let filtered_model =
        gtk::FilterListModel::new(Some(search_model.clone()), None::<gtk::CustomFilter>);
    let search_active = Rc::new(Cell::new(true));
    let search_results = Rc::new(RefCell::new(Vec::new()));

    let active_during_rebind: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
    let observed = active_during_rebind.clone();
    let flag = search_active.clone();
    filtered_model.connect_items_changed(move |_, _, _, added| {
        if added > 0 {
            observed.borrow_mut().push(flag.get());
        }
    });

    deactivate_recursive_search(
        &search_active,
        &search_results,
        &search_model,
        &filtered_model,
        &directory_model,
    );

    assert!(!search_active.get());
    assert_eq!(filtered_model.n_items(), 3);
    assert!(
        !active_during_rebind.borrow().is_empty(),
        "swapping back to the directory model should have re-added rows"
    );
    assert!(
        active_during_rebind.borrow().iter().all(|active| !active),
        "rows rebound while recursive search still looked active"
    );
}

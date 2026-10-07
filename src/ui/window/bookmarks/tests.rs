// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::glib;

use super::super::load_pinned_places;
use crate::model::Location;

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "operation timed out");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn check_trash_and_restore_do_not_resurrect_removed_pins() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("isolated home"));
    let parent = home.join("fixture/gone");
    let child = parent.join("child");
    let keep = home.join("fixture/keep");
    std::fs::create_dir_all(&child).expect("deleted folder");
    std::fs::create_dir_all(&keep).expect("retained folder");
    let initial = vec![
        (Location::local(&parent), "Gone".into()),
        (Location::local(&child), "Child".into()),
        (Location::local(&keep), "Keep".into()),
    ];
    super::super::save_pinned_places(&initial).expect("pins");
    let preferences = crate::ui::preferences::PreferenceManager::shared();
    let first = super::super::browser_for_window();
    let second = super::super::browser_for_window();
    let first_sidebar = super::super::build_sidebar(first.clone(), preferences.clone(), true);
    let second_sidebar = super::super::build_sidebar(second, preferences, true);
    let browser = first.browser();
    browser.navigate(Location::local(parent.parent().expect("parent")));
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    browser.select_entries_by_name(&["gone".into()]);
    let entries = browser.selected_entries();
    assert_eq!(entries.len(), 1);
    let finished = Rc::new(RefCell::new(None));
    let observed = finished.clone();
    browser.observe(move |event| {
        if let crate::app::BrowserEvent::DeletionFinished { succeeded } =
            crate::test_support::operations::operation_event(event)
        {
            observed.replace(Some(*succeeded));
        }
    });
    browser.delete(entries, false);
    wait_until(|| finished.borrow().is_some());
    assert_eq!(*finished.borrow(), Some(true));
    assert!(!parent.exists());
    let expected = vec![(Location::local(&keep), "Keep".into())];
    assert_eq!(load_pinned_places().expect("saved pins"), expected);
    assert_eq!(*first_sidebar.state.pinned_places.borrow(), expected);
    assert_eq!(*second_sidebar.state.pinned_places.borrow(), expected);
    assert_eq!(
        first_sidebar.state.visible_pins.borrow().as_slice(),
        &[Location::local(&keep)]
    );
    assert_eq!(
        second_sidebar.state.visible_pins.borrow().as_slice(),
        &[Location::local(&keep)]
    );
    let restored = Rc::new(std::cell::Cell::new(false));
    let observed = restored.clone();
    browser.observe(move |event| {
        if matches!(event, crate::app::BrowserEvent::RestorationFinished { .. }) {
            observed.set(true);
        }
    });
    assert!(first.undo_last_operation());
    wait_until(|| restored.get());
    assert!(parent.exists());
    assert_eq!(load_pinned_places().expect("pins after restore"), expected);
}

#[test]
fn trash_and_restore_do_not_resurrect_removed_pins() {
    crate::test_support::gtk_test(
        "ui::window::bookmarks::tests::trash_and_restore_do_not_resurrect_removed_pins",
        check_trash_and_restore_do_not_resurrect_removed_pins,
    );
}

#[test]
fn deletion_updates_surviving_sidebars_after_the_origin_disconnects() {
    crate::test_support::gtk_test(
        "ui::window::bookmarks::tests::deletion_updates_surviving_sidebars_after_the_origin_disconnects",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            let gone = directory.path().join("gone");
            std::fs::create_dir(&gone).expect("folder");
            let pins = vec![(Location::local(&gone), "Gone".into())];
            super::super::save_pinned_places(&pins).expect("pins");
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            let first = super::super::browser_for_window();
            let second = super::super::browser_for_window();
            let origin = super::super::build_sidebar(first.clone(), preferences.clone(), true);
            let surviving = super::super::build_sidebar(second, preferences, true);
            let browser = first.browser();
            browser.navigate(Location::local(directory.path()));
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            browser.select(0, 0);
            let entries = browser.selected_entries();
            assert_eq!(entries.len(), 1);
            browser.clear_observer();
            origin.disconnect();
            browser.delete(entries, true);
            wait_until(|| !gone.exists() && surviving.state.pinned_places.borrow().is_empty());
            assert!(load_pinned_places().expect("saved pins").is_empty());
            assert_eq!(*origin.state.pinned_places.borrow(), pins);
        },
    );
}

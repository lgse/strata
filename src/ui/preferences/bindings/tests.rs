// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;

#[test]
fn bindings_initialize_deduplicate_and_release_destroyed_anchors() {
    gtk_test(
        "ui::preferences::bindings::tests::bindings_initialize_deduplicate_and_release_destroyed_anchors",
        || {
            let manager = PreferenceManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let values = Rc::new(RefCell::new(Vec::new()));
            let observed = values.clone();
            manager.bind_preference(
                &anchor,
                PreferenceManager::folder_peeking,
                move |_, value| observed.borrow_mut().push(value),
            );
            assert_eq!(*values.borrow(), [true]);
            manager.set_folder_peeking(false);
            manager.set_folder_peeking(false);
            manager.set_type_to_search(false);
            assert_eq!(*values.borrow(), [true, false]);
            let revision = manager.changes.revision.get();
            manager.set_type_to_search(false);
            assert_eq!(manager.changes.revision.get(), revision);
            drop(anchor);
            assert!(manager.changes.listeners.borrow().is_empty());
            assert_eq!(Rc::strong_count(&values), 1);
        },
    );
}

#[test]
fn failed_saves_still_apply_and_retry_without_repeating_notifications() {
    gtk_test(
        "ui::preferences::bindings::tests::failed_saves_still_apply_and_retry_without_repeating_notifications",
        || {
            let manager = PreferenceManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let values = Rc::new(RefCell::new(Vec::new()));
            let observed = values.clone();
            manager.bind_preference(
                &anchor,
                PreferenceManager::folder_peeking,
                move |_, value| observed.borrow_mut().push(value),
            );
            fs::create_dir_all(settings_path()).expect("block settings file with a directory");
            manager.set_folder_peeking(false);
            assert_eq!(*values.borrow(), [true, false]);
            assert!(manager.persistence_dirty.get());
            fs::remove_dir(settings_path()).expect("remove write failure fixture");
            manager.set_folder_peeking(false);
            assert!(!manager.persistence_dirty.get());
            assert!(!read_preferences().expect("retried save").folder_peeking);
            assert_eq!(*values.borrow(), [true, false]);
        },
    );
}

#[test]
fn reentrant_changes_reach_all_bindings_without_notification_loops() {
    gtk_test(
        "ui::preferences::bindings::tests::reentrant_changes_reach_all_bindings_without_notification_loops",
        || {
            let manager = PreferenceManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let values = Rc::new(RefCell::new(Vec::new()));
            let observed = values.clone();
            manager.bind_preference(
                &anchor,
                PreferenceManager::type_to_search,
                move |_, value| observed.borrow_mut().push(value),
            );
            let weak = Rc::downgrade(&manager);
            manager.bind_preference(
                &anchor,
                PreferenceManager::folder_peeking,
                move |_, value| {
                    if let Some(manager) = weak.upgrade() {
                        manager.set_folder_peeking(value);
                        manager.set_type_to_search(value);
                    }
                },
            );
            manager.set_folder_peeking(false);
            assert_eq!(*values.borrow(), [true, false]);
            manager.set_folder_peeking(true);
            assert_eq!(*values.borrow(), [true, false, true]);
        },
    );
}

#[test]
fn a_channel_change_reaches_only_the_views_that_still_exist() {
    let ran = RefCell::new(Vec::new());
    let live = notify_live(
        vec![(1, true), (2, false), (3, true)],
        |(_, alive)| *alive,
        |(id, _)| ran.borrow_mut().push(*id),
    );

    assert_eq!(ran.into_inner(), vec![1, 3]);
    assert_eq!(live, vec![(1, true), (3, true)]);
}

#[test]
fn a_channel_change_with_no_surviving_views_clears_the_registry() {
    let ran = RefCell::new(0_u32);
    let live = notify_live(
        vec![(1, false)],
        |(_, alive)| *alive,
        |_| *ran.borrow_mut() += 1,
    );

    assert_eq!(ran.into_inner(), 0);
    assert!(live.is_empty());
}

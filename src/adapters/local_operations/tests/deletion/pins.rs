// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn provider_cleans_only_confirmed_pins_on_failure_and_cancellation() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::pins::provider_cleans_only_confirmed_pins_on_failure_and_cancellation",
        || {
            for cancel_after_first in [false, true] {
                let directory = tempfile::tempdir().expect("fixture");
                let first = directory.path().join("first");
                let second = directory.path().join("second");
                fs::create_dir(&first).expect("first folder");
                if cancel_after_first {
                    fs::create_dir(&second).expect("second folder");
                }
                let bookmarks = crate::adapters::bookmarks::pinned_places_path();
                fs::create_dir_all(bookmarks.parent().expect("bookmarks directory"))
                    .expect("config");
                let pin = |path: &Path| format!("{} Pin\n", gio::File::for_path(path).uri());
                fs::write(&bookmarks, format!("{}{}", pin(&first), pin(&second))).expect("pins");
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let operation = Rc::new(RefCell::new(None::<LoadHandle>));
                let cancelling = operation.clone();
                operation.replace(Some(LocalOperationProvider.delete(
                    DeleteRequest {
                        id: OperationRequestId(1372),
                        entries: vec![directory_entry(&first), directory_entry(&second)],
                        permanent: true,
                    },
                    Rc::new(move |event| {
                        if cancel_after_first
                            && matches!(event, OperationEvent::DeleteProgress { completed: 1, .. })
                        {
                            cancelling.borrow_mut().take();
                        }
                        emitted.borrow_mut().push(event);
                    }),
                )));
                let context = glib::MainContext::default();
                while !events.borrow().iter().any(|event| {
                    matches!(
                        event,
                        OperationEvent::Cancelled { .. }
                            | OperationEvent::CompletedWithErrors { .. }
                    )
                }) {
                    context.iteration(true);
                }
                crate::storage::writer::settle();
                assert_eq!(
                    fs::read_to_string(&bookmarks).expect("remaining pins"),
                    pin(&second)
                );
                let events = events.borrow();
                if cancel_after_first {
                    assert!(events.iter().any(|event| matches!(event, OperationEvent::Cancelled { result, .. } if result.completed == [Location::local(&first)] && result.not_attempted == [Location::local(&second)])));
                    assert!(second.exists());
                } else {
                    assert!(events.iter().any(|event| matches!(event, OperationEvent::CompletedWithErrors { deleted_locations, .. } if deleted_locations == &[Location::local(&first)])));
                }
                assert!(!first.exists());
            }
        },
    );
}

#[test]
fn deleting_browser_can_drop_after_a_confirmed_deletion_without_losing_pin_cleanup() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::pins::deleting_browser_can_drop_after_a_confirmed_deletion_without_losing_pin_cleanup",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            let first = directory.path().join("first");
            let second = directory.path().join("second");
            fs::create_dir(&first).expect("first folder");
            fs::create_dir(&second).expect("second folder");
            let bookmarks = crate::adapters::bookmarks::pinned_places_path();
            fs::create_dir_all(bookmarks.parent().expect("bookmarks directory")).expect("config");
            let pin = |path: &Path| format!("{} Pin\n", gio::File::for_path(path).uri());
            fs::write(&bookmarks, format!("{}{}", pin(&first), pin(&second))).expect("pins");
            let browser = crate::app::Browser::new(Rc::new(crate::adapters::LocalFileSource));
            browser.set_operation_provider(Rc::new(LocalOperationProvider));
            let weak = Rc::downgrade(&browser);
            let owner = Rc::new(RefCell::new(Some(browser.clone())));
            let release = owner.clone();
            browser.observe(move |event| {
                if matches!(
                    event,
                    crate::app::BrowserEvent::DeletionProgress { completed: 1, .. }
                ) {
                    release
                        .borrow_mut()
                        .take()
                        .expect("origin browser")
                        .clear_observer();
                }
            });
            browser.delete(
                vec![directory_entry(&first), directory_entry(&second)],
                true,
            );
            drop(browser);
            let context = glib::MainContext::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while weak.upgrade().is_some() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "origin browser was retained"
                );
                context.iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!first.exists());
            assert!(second.exists());
            crate::storage::writer::settle();
            assert_eq!(
                fs::read_to_string(bookmarks).expect("saved pins"),
                pin(&second)
            );
        },
    );
}

#[test]
fn bookmark_cleanup_failure_preserves_the_successful_deletion_outcome() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::pins::bookmark_cleanup_failure_preserves_the_successful_deletion_outcome",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            let gone = directory.path().join("gone");
            fs::create_dir(&gone).expect("folder");
            let bookmarks = crate::adapters::bookmarks::pinned_places_path();
            fs::create_dir_all(&bookmarks).expect("unreadable bookmark file");
            let failures = Rc::new(RefCell::new(Vec::new()));
            let observed = failures.clone();
            let _watch = crate::adapters::bookmarks::watch_changes(move |result| {
                observed.borrow_mut().push(result.is_err());
                true
            });
            let events = Rc::new(RefCell::new(Vec::new()));
            let emitted = events.clone();
            let _operation = LocalOperationProvider.delete(
                DeleteRequest {
                    id: OperationRequestId(1372),
                    entries: vec![directory_entry(&gone)],
                    permanent: true,
                },
                Rc::new(move |event| emitted.borrow_mut().push(event)),
            );
            while !events
                .borrow()
                .iter()
                .any(|event| matches!(event, OperationEvent::Deleted { .. }))
            {
                glib::MainContext::default().iteration(true);
            }
            crate::storage::writer::settle();
            assert!(!gone.exists());
            assert_eq!(*failures.borrow(), [true]);
            assert!(
                matches!(events.borrow().last(), Some(OperationEvent::Deleted { locations, .. }) if locations == &[Location::local(&gone)])
            );
            assert!(!events.borrow().iter().any(|event| matches!(
                event,
                OperationEvent::Failed { .. } | OperationEvent::CompletedWithErrors { .. }
            )));
        },
    );
}

// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use super::*;
use crate::adapters::{LocalFileSource, LocalOperationProvider};
use crate::ui::browser::PeekBehavior;
use crate::ui::theme::ThemeManager;

fn view() -> BrowserView {
    let view = BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default());
    view.set_operation_provider(Rc::new(LocalOperationProvider));
    let window = gtk::Window::builder().child(&view.widget()).build();
    view.connect_navigation_cleanup(&window);
    let browser = view.browser();
    window.connect_destroy(move |_| {
        browser.bump_navigation_generation();
        browser.clear_observer();
    });
    window.present();
    view
}

fn request() -> Rc<OpenRequest> {
    OpenRequest::new(gtk::MountOperation::new(None::<&gtk::Window>))
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "operation timed out");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn has_spinner(widget: &gtk::Widget) -> bool {
    if widget.is::<gtk::Spinner>() {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if has_spinner(&widget) {
            return true;
        }
    }
    false
}

fn button_with_label(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Ok(button) = widget.clone().downcast::<gtk::Button>()
        && button.label().as_deref() == Some(label)
    {
        return Some(button);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(button) = button_with_label(&widget, label) {
            return Some(button);
        }
    }
    None
}

#[test]
fn query_kind_classifies_a_directory() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::query_kind_classifies_a_directory",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let file = gio::File::for_path(root.path());
            let kind = glib::MainContext::new().block_on(query_kind(&file, None));
            assert!(matches!(kind, Ok(Kind::Directory)));
        },
    );
}

#[test]
fn query_kind_classifies_a_regular_file() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::query_kind_classifies_a_regular_file",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let path = root.path().join("open me.txt");
            std::fs::write(&path, b"hello").expect("fixture file");
            let kind =
                glib::MainContext::new().block_on(query_kind(&gio::File::for_path(&path), None));
            assert!(matches!(kind, Ok(Kind::File)));
        },
    );
}

#[test]
fn query_kind_follows_a_symlink_to_a_directory() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::query_kind_follows_a_symlink_to_a_directory",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let target = root.path().join("target");
            std::fs::create_dir(&target).expect("fixture directory");
            let link = root.path().join("link");
            std::os::unix::fs::symlink(&target, &link).expect("fixture symlink");
            let kind =
                glib::MainContext::new().block_on(query_kind(&gio::File::for_path(&link), None));
            assert!(matches!(kind, Ok(Kind::Directory)));
        },
    );
}

#[test]
fn query_kind_treats_a_broken_symlink_as_a_file() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::query_kind_treats_a_broken_symlink_as_a_file",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let missing = root.path().join("missing");
            let link = root.path().join("broken");
            std::os::unix::fs::symlink(&missing, &link).expect("fixture symlink");
            let kind =
                glib::MainContext::new().block_on(query_kind(&gio::File::for_path(&link), None));
            assert!(matches!(kind, Ok(Kind::File)));
        },
    );
}

#[test]
fn query_kind_fails_a_plain_missing_path() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::query_kind_fails_a_plain_missing_path",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let missing = root.path().join("does-not-exist");
            let kind =
                glib::MainContext::new().block_on(query_kind(&gio::File::for_path(&missing), None));
            assert!(kind.is_err());
        },
    );
}

#[test]
fn classify_reveals_a_regular_file_in_its_parent() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::classify_reveals_a_regular_file_in_its_parent",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let root = tempfile::tempdir().expect("fixture");
            let file_path = root.path().join("open me.txt");
            std::fs::write(&file_path, b"hello").expect("fixture file");
            let file = gio::File::for_path(&file_path);
            let location = location_for_file(&file).expect("native location");

            let browser = view();
            classify(browser.clone(), file, location);

            wait_until(|| browser.browser().active_location().is_some());
            let active = browser.browser().active_location().expect("navigated");
            assert_eq!(active.native_path(), Some(root.path()));
        },
    );
}

#[test]
fn classify_opens_a_directory_argument() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::classify_opens_a_directory_argument",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let root = tempfile::tempdir().expect("fixture");
            let file = gio::File::for_path(root.path());
            let location = location_for_file(&file).expect("native location");

            let browser = view();
            classify(browser.clone(), file, location);

            wait_until(|| browser.browser().active_location().is_some());
            let active = browser.browser().active_location().expect("navigated");
            assert_eq!(active.native_path(), Some(root.path()));
        },
    );
}

#[test]
fn fast_failure_keeps_retry_after_the_connecting_delay() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::fast_failure_keeps_retry_after_the_connecting_delay",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let root = tempfile::tempdir().expect("fixture");
            let missing = root.path().join("missing");
            let file = gio::File::for_path(&missing);
            let location = location_for_file(&file).expect("native location");
            let browser = view();

            classify(browser.clone(), file, location);
            wait_until(|| {
                status_widget(&browser.overlay())
                    .as_ref()
                    .and_then(|status| button_with_label(status, "Retry"))
                    .is_some()
            });
            pump_for(CONNECTING_DELAY + Duration::from_millis(200));

            let status = status_widget(&browser.overlay()).expect("error status");
            assert!(button_with_label(&status, "Retry").is_some());
            assert!(!has_spinner(&status));
        },
    );
}

#[test]
fn connecting_cancel_invalidates_the_request_and_clears_status() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::connecting_cancel_invalidates_the_request_and_clears_status",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let browser = view();
            let generation = browser.browser().bump_navigation_generation();
            show_connecting(browser.downgrade(), generation, request());
            let status = status_widget(&browser.overlay()).expect("connecting status");
            button_with_label(&status, "Cancel")
                .expect("cancel button")
                .emit_clicked();

            assert!(browser.browser().navigation_generation() > generation);
            assert!(status_widget(&browser.overlay()).is_none());
        },
    );
}

#[test]
fn errors_do_not_expose_uri_credentials() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::errors_do_not_expose_uri_credentials",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let browser = view();
            show_error(
                &browser,
                gio::File::for_uri("sftp://user:secret@example.invalid/file.txt"),
                Location::uri("sftp://user@example.invalid/file.txt"),
            );
            let status = status_widget(&browser.overlay()).expect("error status");
            assert_eq!(
                status
                    .first_child()
                    .and_then(|content| content.first_child())
                    .and_downcast::<gtk::Label>()
                    .expect("error label")
                    .text(),
                "Unable to open location"
            );
            assert!(button_with_label(&status, "Retry").is_some());
        },
    );
}

#[test]
fn window_close_aborts_pending_request_and_releases_the_view() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::window_close_aborts_pending_request_and_releases_the_view",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let browser = view();
            let weak = browser.downgrade();
            let window = browser
                .overlay()
                .root()
                .and_downcast::<gtk::Window>()
                .expect("window");
            let operation = gtk::MountOperation::new(Some(&window));
            let replies = Rc::new(RefCell::new(Vec::new()));
            let observed_replies = replies.clone();
            operation.connect_reply(move |_, reply| observed_replies.borrow_mut().push(reply));
            let request = OpenRequest::new(operation);
            let timer = glib::timeout_add_local_once(Duration::from_secs(30), || {});
            request.timer.replace(Some(timer));
            let generation = browser.browser().bump_navigation_generation();
            let cleanup_request = request.clone();
            browser.set_navigation_cleanup(move || cleanup_request.abort());
            show_connecting(browser.downgrade(), generation, request.clone());

            window.close();
            drop(window);
            drop(browser);
            pump_for(Duration::from_millis(50));

            assert!(!request.active.get());
            assert!(request.timer.borrow().is_none());
            assert!(request.operation.parent().is_none());
            assert_eq!(*replies.borrow(), [gio::MountOperationResult::Aborted]);
            assert!(weak.upgrade().is_none());
        },
    );
}

#[test]
fn navigation_aborts_pending_request_and_mount_prompt() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::navigation_aborts_pending_request_and_mount_prompt",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let browser = view();
            let window = browser
                .overlay()
                .root()
                .and_downcast::<gtk::Window>()
                .expect("window");
            let operation = gtk::MountOperation::new(Some(&window));
            let replies = Rc::new(RefCell::new(Vec::new()));
            let observed_replies = replies.clone();
            operation.connect_reply(move |_, reply| observed_replies.borrow_mut().push(reply));
            let request = OpenRequest::new(operation);
            let timer = glib::timeout_add_local_once(Duration::from_secs(30), || {});
            request.timer.replace(Some(timer));
            let generation = browser.browser().bump_navigation_generation();
            let cleanup_request = request.clone();
            browser.set_navigation_cleanup(move || cleanup_request.abort());
            show_connecting(browser.downgrade(), generation, request.clone());

            let elsewhere = tempfile::tempdir().expect("elsewhere fixture");
            browser.navigate_location(Location::local(elsewhere.path()));

            assert!(!request.active.get());
            assert!(request.timer.borrow().is_none());
            assert!(request.operation.parent().is_none());
            assert_eq!(*replies.borrow(), [gio::MountOperationResult::Aborted]);
            assert!(status_widget(&browser.overlay()).is_none());
        },
    );
}

#[test]
fn navigation_dismisses_connecting_status() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::navigation_dismisses_connecting_status",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let browser = view();
            let generation = browser.browser().bump_navigation_generation();
            show_connecting(browser.downgrade(), generation, request());
            assert!(status_widget(&browser.overlay()).is_some());

            let elsewhere = tempfile::tempdir().expect("elsewhere fixture");
            browser.navigate_location(Location::local(elsewhere.path()));

            assert!(status_widget(&browser.overlay()).is_none());
        },
    );
}

#[test]
fn new_navigation_wins_over_a_pending_open_argument_classify() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::new_navigation_wins_over_a_pending_open_argument_classify",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let root = tempfile::tempdir().expect("fixture");
            let file_path = root.path().join("open me.txt");
            std::fs::write(&file_path, b"hello").expect("fixture file");
            let file = gio::File::for_path(&file_path);
            let location = location_for_file(&file).expect("native location");

            let elsewhere = tempfile::tempdir().expect("elsewhere fixture");
            let browser = view();
            classify(browser.clone(), file, location);
            browser
                .browser()
                .navigate(crate::model::Location::local(elsewhere.path()));

            pump_for(Duration::from_millis(200));

            let active = browser.browser().active_location().expect("navigated");
            assert_eq!(active.native_path(), Some(elsewhere.path()));
        },
    );
}

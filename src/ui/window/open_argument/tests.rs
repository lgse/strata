// SPDX-License-Identifier: MIT

use std::{
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
    window.present();
    view
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "operation timed out");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn query_kind_classifies_a_directory() {
    crate::test_support::gtk_test(
        "ui::window::open_argument::tests::query_kind_classifies_a_directory",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let file = gio::File::for_path(root.path());
            let kind = glib::MainContext::new().block_on(query_kind(&file));
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
            let kind = glib::MainContext::new().block_on(query_kind(&gio::File::for_path(&path)));
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
            let kind = glib::MainContext::new().block_on(query_kind(&gio::File::for_path(&link)));
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
            let kind = glib::MainContext::new().block_on(query_kind(&gio::File::for_path(&link)));
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
                glib::MainContext::new().block_on(query_kind(&gio::File::for_path(&missing)));
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
            // Simulate the user navigating away before classification resolves.
            browser
                .browser()
                .navigate(crate::model::Location::local(elsewhere.path()));

            // Give the (invalidated) classify task every chance to run and misbehave.
            let deadline = Instant::now() + Duration::from_millis(200);
            while Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }

            let active = browser.browser().active_location().expect("navigated");
            assert_eq!(active.native_path(), Some(elsewhere.path()));
        },
    );
}

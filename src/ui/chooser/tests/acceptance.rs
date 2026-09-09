// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser_modes::BrowserMode;
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "chooser did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn request(root: PathBuf) -> ChooserRequest {
    ChooserRequest {
        token: "acceptance".into(),
        title: "Acceptance".into(),
        accept_label: "Open".into(),
        modal: false,
        parent: None,
        parent_size_hint: None,
        initial_directory: root,
        kind: ChooserKind::Open {
            directory: false,
            multiple: false,
        },
        filters: Vec::new(),
        current_filter: None,
        choices: Vec::new(),
    }
}

#[test]
fn filtered_selection_only_accepts_on_enter_or_open_with_exact_nested_path() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filtered_selection_only_accepts_on_enter_or_open_with_exact_nested_path",
        || {
            crate::ui::prepare_portal_ui();
            ThemeManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let nested = root.path().join("folder/nested.txt");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(&nested, "nested").expect("nested file");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let state = build_chooser(
                request(root.path().to_path_buf()),
                Arc::new(AtomicBool::new(false)),
                move |value| {
                    received.replace(Some(value));
                },
            )
            .expect("chooser");
            let browser = state.view.browser();
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });

            state.view.show_filter_with_query("nested");
            browser.select(0, 0);
            assert!(
                state.completion.borrow().is_some(),
                "single click only selects"
            );
            assert!(
                !state.error.is_visible(),
                "single click must not show an error"
            );
            browser.set_chooser_location(Location::local(&nested));
            state.accept();
            wait_until(|| result.borrow().is_some());
            let selected = result
                .borrow_mut()
                .take()
                .expect("result")
                .expect("accepted");
            assert_eq!(selected.uris().len(), 1);
            assert_eq!(
                selected.uris()[0].to_string(),
                gio::File::for_path(&nested).uri()
            );
        },
    );
}

#[test]
fn observer_activation_returns_exact_nested_path() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::observer_activation_returns_exact_nested_path",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let nested = root.path().join("folder/nested.txt");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(&nested, "nested").expect("nested file");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let state = build_chooser(
                request(root.path().to_path_buf()),
                Arc::new(AtomicBool::new(false)),
                move |value| {
                    received.replace(Some(value));
                },
            )
            .expect("chooser");
            state.activate_file(&Location::local(&nested));
            let selected = result
                .borrow_mut()
                .take()
                .expect("result")
                .expect("accepted");
            assert_eq!(
                selected.uris()[0].to_string(),
                gio::File::for_path(&nested).uri()
            );
        },
    );
}

#[test]
fn recursive_folder_selection_navigates_without_accepting() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::recursive_folder_selection_navigates_without_accepting",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let folder = root.path().join("folder");
            std::fs::create_dir(&folder).expect("folder");
            let state = build_chooser(
                request(root.path().to_path_buf()),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .expect("chooser");
            let browser = state.view.browser();
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            browser.navigate(Location::local(&folder));
            wait_until(|| browser.active_location() == Some(Location::local(&folder)));
            assert!(state.completion.borrow().is_some());
            assert!(!state.error.is_visible());
        },
    );
}

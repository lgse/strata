// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn full_file_path_selects_and_accepts_the_named_file() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::open_paths::full_file_path_selects_and_accepts_the_named_file",
        || {
            crate::ui::prepare_portal_ui();
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                let initial = root.path().join("initial");
                let destination = root.path().join("destination");
                let target = destination.join("report.pdf");
                std::fs::create_dir(&initial).expect("initial folder");
                std::fs::create_dir(&destination).expect("destination folder");
                std::fs::write(&target, "report").expect("target file");

                let (state, result) = open_chooser(request(initial.clone()));
                let browser = state.view.browser();
                wait_listed(&browser, 0);

                state.view.begin_location_edit();
                let focused =
                    gtk::prelude::RootExt::focus(&state.window).expect("focused location editor");
                let entry = focused
                    .clone()
                    .downcast::<gtk::Entry>()
                    .ok()
                    .or_else(|| {
                        focused
                            .ancestor(gtk::Entry::static_type())
                            .and_downcast::<gtk::Entry>()
                    })
                    .expect("focused location entry");
                entry.set_text(&target.to_string_lossy());
                entry.emit_activate();

                wait_until(|| {
                    browser.active_location() == Some(Location::local(&destination))
                        && browser
                            .selected_entries()
                            .as_slice()
                            .first()
                            .is_some_and(|entry| entry.location == Location::local(&target))
                });
                assert!(!browser.selection_is_load_cursor(), "{mode:?}");

                let selected = accept(&state, &result);
                assert_single_uri(&selected, &target, &format!("{mode:?}"));
                state.window.close();
            }
        },
    );
}

#[test]
fn directory_confirmation_distinguishes_load_cursor_from_explicit_selection() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::open_paths::directory_confirmation_distinguishes_load_cursor_from_explicit_selection",
        || {
            crate::ui::prepare_portal_ui();
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                for explicit in [false, true] {
                    let root = tempfile::tempdir().expect("fixture");
                    let child = root.path().join("child");
                    std::fs::create_dir(&child).expect("child folder");
                    std::fs::write(root.path().join("hidden-by-folder-policy.txt"), "file")
                        .expect("file");
                    let mut chooser_request = request(root.path().to_path_buf());
                    chooser_request.kind = ChooserKind::Open {
                        directory: true,
                        multiple: false,
                    };
                    let (state, result) = open_chooser(chooser_request);
                    let browser = state.view.browser();
                    wait_column(&browser, 0, 1);
                    assert!(browser.selection_is_load_cursor(), "{mode:?}");
                    if explicit {
                        browser.select(0, 0);
                        assert!(!browser.selection_is_load_cursor(), "{mode:?}");
                    }
                    let selected = accept(&state, &result);
                    let expected = if explicit {
                        child.as_path()
                    } else {
                        root.path()
                    };
                    assert_single_uri(
                        &selected,
                        expected,
                        &format!("{mode:?}, explicit={explicit}"),
                    );
                    state.window.close();
                }
            }
        },
    );
}

#[test]
fn observer_activation_returns_exact_nested_path() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::open_paths::observer_activation_returns_exact_nested_path",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let nested = root.path().join("folder/nested.txt");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(&nested, "nested").expect("nested file");
            let (state, result) = open_chooser(request(root.path().to_path_buf()));
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
        "ui::chooser::tests::acceptance::open_paths::recursive_folder_selection_navigates_without_accepting",
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
            wait_listed(&browser, 0);
            browser.navigate(Location::local(&folder));
            wait_until(|| browser.active_location() == Some(Location::local(&folder)));
            assert!(state.completion.borrow().is_some());
            assert!(!state.error.is_visible());
        },
    );
}

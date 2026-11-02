// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn filter_text_select_file_click_open_accepts_root_file() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filter_search::filter_text_select_file_click_open_accepts_root_file",
        || {
            crate::ui::prepare_portal_ui();
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                std::fs::write(root.path().join("readme.md"), "readme").expect("file");
                std::fs::write(root.path().join("todo.txt"), "todo").expect("file");
                let (state, result) = open_chooser(request(root.path().to_path_buf()));
                let browser = state.view.browser();
                wait_column(&browser, 0, 2);

                assert!(state.view.show_filter_with_query("readme"));
                wait_until(|| {
                    select_first_file_list_item(&state.view.widget());
                    state
                        .view
                        .selected_search_results()
                        .is_some_and(|entries| !entries.is_empty())
                });
                assert!(
                    !state.error.is_visible(),
                    "selecting a root file must not show an error: {mode:?}"
                );
                if mode == BrowserMode::Columns {
                    assert!(state.view.show_filter_with_query("no-matches"));
                    wait_until(|| state.view.selected_search_results() == Some(Vec::new()));
                    state.accept_button.grab_focus();
                    state.accept_button.emit_clicked();
                    assert!(result.borrow().is_none(), "no results must not accept");
                    assert!(state.error.is_visible(), "no results must show an error");
                    while glib::MainContext::default().pending() {
                        glib::MainContext::default().iteration(false);
                    }
                    assert!(state.view.show_filter_with_query("readme"));
                    wait_until(|| {
                        select_first_file_list_item(&state.view.widget());
                        state
                            .view
                            .selected_search_results()
                            .is_some_and(|entries| entries.len() == 1)
                    });
                }
                let selected = accept(&state, &result);
                assert_single_uri(
                    &selected,
                    root.path().join("readme.md"),
                    &format!("{mode:?}"),
                );
                state.window.close();
            }
        },
    );
}

#[test]
fn columns_filter_after_navigating_into_subfolder_accepts_search_result() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filter_search::columns_filter_after_navigating_into_subfolder_accepts_search_result",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::Columns);
            let root = tempfile::tempdir().expect("fixture");
            std::fs::write(root.path().join("readme.md"), "readme").expect("file");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(root.path().join("folder/notes.txt"), "notes").expect("nested file");
            let (state, result) = open_chooser(request(root.path().to_path_buf()));
            let browser = state.view.browser();
            wait_column(&browser, 0, 2);

            let folder_position = (0..2)
                .find(|&position| {
                    browser
                        .entry_at(0, position)
                        .is_some_and(|entry| entry.is_directory())
                })
                .expect("folder position");
            browser.activate(0, folder_position);
            wait_column(&browser, 1, 1);

            assert!(state.view.show_filter_with_query("notes"));
            wait_until(|| {
                select_first_file_list_item(&state.view.widget());
                state
                    .view
                    .selected_search_results()
                    .is_some_and(|entries| !entries.is_empty())
            });
            state.accept_button.grab_focus();
            assert!(
                !state.error.is_visible(),
                "selecting a search result must not show an error"
            );
            let selected = accept(&state, &result);
            assert_single_uri(&selected, root.path().join("folder/notes.txt"), "");
        },
    );
}

#[test]
fn columns_filter_in_non_active_column_accepts_search_result_on_open() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filter_search::columns_filter_in_non_active_column_accepts_search_result_on_open",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::Columns);
            let root = tempfile::tempdir().expect("fixture");
            std::fs::write(root.path().join("readme.md"), "readme").expect("file");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(root.path().join("folder/notes.txt"), "notes").expect("nested file");
            let (state, result) = open_chooser(request(root.path().to_path_buf()));
            let browser = state.view.browser();
            wait_column(&browser, 0, 2);

            let folder_position = (0..2)
                .find(|&position| {
                    browser
                        .entry_at(0, position)
                        .is_some_and(|entry| entry.is_directory())
                })
                .expect("folder position");
            browser.activate(0, folder_position);
            wait_column(&browser, 1, 1);

            browser.select(1, 0);
            wait_until(|| {
                browser.selected_entries().first().is_some_and(|entry| {
                    entry.location == Location::local(root.path().join("folder/notes.txt"))
                })
            });
            let root_filter_entry =
                nth_filter_entry(&state.view.widget(), 0).expect("column 0 filter");
            root_filter_entry.set_text("no-matches");
            root_filter_entry.grab_focus_without_selecting();
            wait_until(|| state.view.selected_search_results() == Some(Vec::new()));
            state.accept_button.grab_focus();
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            browser.select(1, 0);
            assert_eq!(
                browser.selected_entries()[0].location,
                Location::local(root.path().join("folder/notes.txt"))
            );
            assert_eq!(state.view.selected_search_results(), Some(Vec::new()));
            state.accept_button.emit_clicked();
            assert!(
                result.borrow().is_none(),
                "empty search must not accept another column's file"
            );
            assert!(state.error.is_visible(), "empty search must show an error");

            root_filter_entry.set_text("readme");
            root_filter_entry.grab_focus_without_selecting();
            wait_until(|| {
                select_first_file_list_item_in_first_list(&state.view.widget());
                state
                    .view
                    .selected_search_results()
                    .is_some_and(|entries| !entries.is_empty())
            });
            state.accept_button.grab_focus();
            let selected = accept(&state, &result);
            assert_single_uri(&selected, root.path().join("readme.md"), "");
        },
    );
}

#[test]
fn filtered_selection_only_accepts_on_enter_or_open_with_exact_nested_path() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filter_search::filtered_selection_only_accepts_on_enter_or_open_with_exact_nested_path",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let first = root.path().join("folder/nested-a.txt");
            let nested = root.path().join("folder/nested-b.txt");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(&first, "first").expect("first nested file");
            std::fs::write(&nested, "second").expect("second nested file");
            let (state, result) = open_chooser(request(root.path().to_path_buf()));
            let browser = state.view.browser();
            wait_listed(&browser, 0);

            state.view.show_filter_with_query("nested*.TXT");
            wait_until(|| {
                search_results(&state.view.widget()).is_some_and(|results| results.n_items() > 1)
            });
            let results = search_results(&state.view.widget()).expect("search results");
            results.select(0, true);
            let first_selected = state
                .view
                .selected_search_results()
                .expect("active search")
                .first()
                .expect("first search selection")
                .location
                .clone();
            assert!(
                first_selected == Location::local(&first)
                    || first_selected == Location::local(&nested)
            );
            results.unselect_all();
            results.select(1, true);
            wait_until(|| {
                state.view.selected_search_results().is_some_and(|entries| {
                    entries
                        .first()
                        .is_some_and(|entry| entry.location != first_selected)
                })
            });
            let second_selected = state
                .view
                .selected_search_results()
                .expect("active search")
                .first()
                .expect("second search selection")
                .location
                .clone();
            assert!(
                state.completion.borrow().is_some(),
                "selecting a recursive result must not accept it"
            );
            assert!(
                !state.error.is_visible(),
                "selection must not show an error"
            );
            let selected = accept(&state, &result);
            assert_single_uri(
                &selected,
                second_selected.native_path().expect("native path"),
                "",
            );
        },
    );
}

#[test]
fn dismissing_recursive_results_then_extending_widget_selection_accepts_current_files() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filter_search::dismissing_recursive_results_then_extending_widget_selection_accepts_current_files",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::Icons);
            let root = tempfile::tempdir().expect("fixture");
            let nested = root.path().join("folder/nested.txt");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(&nested, "nested").expect("nested file");
            for name in ["alpha.txt", "beta.txt"] {
                std::fs::write(root.path().join(name), name).expect("root file");
            }
            let mut chooser_request = request(root.path().to_path_buf());
            chooser_request.kind = ChooserKind::Open {
                directory: false,
                multiple: true,
            };
            let (state, result) = open_chooser(chooser_request);
            let browser = state.view.browser();
            wait_column(&browser, 0, 3);

            state.view.show_filter_with_query("nested");
            wait_until(|| {
                search_results(&state.view.widget()).is_some_and(|results| results.n_items() > 0)
            });
            let results = search_results(&state.view.widget()).expect("search results");
            results.select(0, true);
            wait_until(|| {
                state
                    .view
                    .selected_search_results()
                    .is_some_and(|entries| !entries.is_empty())
            });
            state.view.show_filter_with_query("");
            wait_until(|| state.view.selected_search_results().is_none());

            let selection = visible_collection_selection(&state.view.widget())
                .expect("visible browser collection");
            state
                .filename
                .as_ref()
                .expect("open name")
                .set_text("missing.txt");
            selection.select_item(1, true);
            selection.select_item(2, false);
            wait_until(|| browser.selected_entries().len() == 2);
            state.accept_button.emit_clicked();
            wait_until(|| result.borrow().is_some());
            let mut uris = result
                .borrow_mut()
                .take()
                .expect("result")
                .expect("accepted")
                .uris()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            uris.sort();
            assert_eq!(
                uris,
                [root.path().join("alpha.txt"), root.path().join("beta.txt")]
                    .map(|path| gio::File::for_path(path).uri().to_string())
            );
        },
    );
}

#[test]
fn active_recursive_search_without_selection_does_not_accept_hidden_browser_selection() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filter_search::active_recursive_search_without_selection_does_not_accept_hidden_browser_selection",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::Icons);
            let root = tempfile::tempdir().expect("fixture");
            let hidden_selection = root.path().join("selected.txt");
            let nested = root.path().join("folder/nested.txt");
            std::fs::create_dir(nested.parent().expect("parent")).expect("folder");
            std::fs::write(&hidden_selection, "selected").expect("root file");
            std::fs::write(&nested, "nested").expect("nested file");
            let (state, result) = open_chooser(request(root.path().to_path_buf()));
            let browser = state.view.browser();
            wait_column(&browser, 0, 2);

            let selection = visible_collection_selection(&state.view.widget())
                .expect("visible browser collection");
            selection.select_item(1, true);
            wait_until(|| {
                browser
                    .selected_entries()
                    .first()
                    .is_some_and(|entry| entry.location == Location::local(&hidden_selection))
            });

            state.view.show_filter_with_query("nested");
            wait_until(|| {
                search_results(&state.view.widget()).is_some_and(|results| results.n_items() > 0)
            });
            let results = search_results(&state.view.widget()).expect("search results");
            results.select(0, true);
            wait_until(|| {
                state
                    .view
                    .selected_search_results()
                    .is_some_and(|entries| entries.len() == 1)
            });
            results.unselect_all();
            wait_until(|| state.view.selected_search_results() == Some(Vec::new()));

            state.accept_button.emit_clicked();
            assert!(result.borrow().is_none(), "deselection must not accept");
            assert!(state.error.is_visible(), "deselection must show an error");

            state.view.show_filter_with_query("no-matches");
            wait_until(|| {
                results.n_items() == 0 && state.view.selected_search_results() == Some(Vec::new())
            });
            state.accept_button.emit_clicked();
            assert!(result.borrow().is_none(), "no results must not accept");
            assert!(state.error.is_visible(), "no results must show an error");
        },
    );
}

#[test]
fn recursive_multi_selection_accepts_every_selected_file_in_all_modes() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filter_search::recursive_multi_selection_accepts_every_selected_file_in_all_modes",
        || {
            crate::ui::prepare_portal_ui();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                let paths = [
                    root.path().join("one/nested-a.txt"),
                    root.path().join("two/nested-b.txt"),
                ];
                for path in &paths {
                    std::fs::create_dir(path.parent().expect("parent")).expect("folder");
                    std::fs::write(path, "nested").expect("nested file");
                }
                let mut chooser_request = request(root.path().to_path_buf());
                chooser_request.kind = ChooserKind::Open {
                    directory: false,
                    multiple: true,
                };
                let (state, result) = open_chooser(chooser_request);
                let browser = state.view.browser();
                wait_listed(&browser, 0);

                // Startup's deferred focus restoration must precede simulated filter input.
                let initialized = Rc::new(Cell::new(false));
                let initialized_at_idle = initialized.clone();
                glib::idle_add_local_once(move || initialized_at_idle.set(true));
                wait_until(|| initialized.get());
                state.view.show_filter_with_query("nested");
                wait_until(|| state.view.selected_search_results().is_some());
                wait_until(|| {
                    if let Some(results) = search_results(&state.view.widget()) {
                        results.select(0, true);
                        results.select(1, false);
                    } else {
                        select_first_two_file_list_items(&state.view.widget());
                    }
                    state
                        .view
                        .selected_search_results()
                        .is_some_and(|entries| entries.len() == 2)
                });
                state.accept_button.emit_clicked();
                wait_until(|| result.borrow().is_some());
                let mut uris = result
                    .borrow_mut()
                    .take()
                    .expect("result")
                    .expect("accepted")
                    .uris()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                uris.sort();
                let mut expected = paths
                    .map(|path| gio::File::for_path(path).uri().to_string())
                    .to_vec();
                expected.sort();
                assert_eq!(uris, expected);
            }
        },
    );
}

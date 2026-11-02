// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn filter_dropdown_select_file_click_open_accepts_filtered_file() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::type_filters::filter_dropdown_select_file_click_open_accepts_filtered_file",
        || {
            crate::ui::prepare_portal_ui();
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                std::fs::write(root.path().join("readme.md"), "readme").expect("file");
                std::fs::write(root.path().join("notes.txt"), "notes").expect("file");
                let mut chooser_request = request(root.path().to_path_buf());
                chooser_request.filters = vec![
                    FileFilter::new("All files").glob("*"),
                    FileFilter::new("Text").glob("*.txt"),
                ];
                let (state, result) = open_chooser(chooser_request);
                let browser = state.view.browser();
                wait_column(&browser, 0, 2);

                {
                    let dropdown = state.filter_dropdown.as_ref().expect("filter dropdown");
                    let changed = dropdown.changed.borrow();
                    changed.as_ref().expect("filter callback")(1);
                }

                wait_column(&browser, 0, 1);

                let selection = visible_collection_selection(&state.view.widget())
                    .expect("visible browser collection");
                selection.select_item(0, true);
                wait_until(|| {
                    browser.selected_entries().first().is_some_and(|entry| {
                        entry.location == Location::local(root.path().join("notes.txt"))
                    })
                });

                assert!(
                    !state.error.is_visible(),
                    "selecting a filtered file must not show an error: {mode:?}"
                );
                let selected = accept(&state, &result);
                assert_single_uri(
                    &selected,
                    root.path().join("notes.txt"),
                    &format!("{mode:?}"),
                );
                state.window.close();
            }
        },
    );
}

#[test]
fn type_filter_refresh_preserves_recursive_query_and_replaces_eligible_results() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::type_filters::type_filter_refresh_preserves_recursive_query_and_replaces_eligible_results",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_filter_include_subfolders(true);
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                std::fs::create_dir(root.path().join("nested")).expect("folder");
                std::fs::write(root.path().join("other.txt"), "unrelated").expect("file");
                for name in ["needle.txt", "needle.png"] {
                    std::fs::write(root.path().join("nested").join(name), "fixture").expect("file");
                }
                let mut request = request(root.path().to_path_buf());
                request.filters = vec![
                    FileFilter::new("Text").glob("*.txt"),
                    FileFilter::new("Images").mimetype("image/png"),
                ];
                let (state, result) = open_chooser(request);
                let browser = state.view.browser();
                wait_column(&browser, 0, 2);
                assert!(state.view.show_filter_with_query("needle"));
                let query = nth_filter_entry(&state.view.widget(), 0).expect("query");
                let select_result = |name: &str| {
                    wait_until(|| {
                        let Some(results) = visible_collection_selection(&state.view.widget())
                        else {
                            return false;
                        };
                        if state.view.selected_search_results().is_none() || results.n_items() != 1
                        {
                            return false;
                        }
                        results.select_item(0, true);
                        state.view.selected_search_results().is_some_and(|entries| {
                            entries.len() == 1
                                && entries[0].location
                                    == Location::local(root.path().join("nested").join(name))
                        })
                    });
                };
                select_result("needle.txt");
                let dropdown = state.filter_dropdown.as_ref().expect("type filter");
                for (index, name, count) in [
                    (1, "needle.png", 1),
                    (0, "needle.txt", 2),
                    (1, "needle.png", 1),
                ] {
                    dropdown.selected.set(index);
                    dropdown.changed.borrow().as_ref().expect("callback")(index);
                    assert_eq!(query.text(), "needle", "{mode:?}");
                    state.accept_button.emit_clicked();
                    assert!(
                        result.borrow().is_none(),
                        "a removed selection must not be accepted: {mode:?}"
                    );
                    wait_until(|| {
                        browser
                            .column_snapshot(0)
                            .is_some_and(|column| !column.loading && column.count == count)
                    });
                    assert_eq!(query.text(), "needle", "{mode:?}");
                    assert!(
                        browser
                            .entry_at(0, 0)
                            .is_some_and(|entry| entry.is_directory()),
                        "normal listing retains folder identity: {mode:?}"
                    );
                    select_result(name);
                }
                let selected = accept(&state, &result);
                assert_eq!(
                    selected
                        .uris()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                    vec![
                        gio::File::for_path(root.path().join("nested/needle.png"))
                            .uri()
                            .to_string()
                    ]
                );
            }
        },
    );
}

#[test]
fn type_filtered_search_finds_eligible_entries_beyond_the_first_ranked_candidates() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::type_filters::type_filtered_search_finds_eligible_entries_beyond_the_first_ranked_candidates",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_filter_include_subfolders(true);
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                std::fs::create_dir(root.path().join("nested")).expect("directory");
                let target = root.path().join("nested/needle-picture-with-long-name.png");
                std::fs::write(&target, "fixture").expect("image filename");
                for index in 0..120 {
                    std::fs::write(root.path().join(format!("needle-{index:03}.txt")), "file")
                        .expect("file");
                }
                let mut request = request(root.path().to_path_buf());
                request.filters = vec![FileFilter::new("Images").mimetype("image/png")];
                let (state, result) = open_chooser(request);
                wait_until(|| {
                    state
                        .view
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading && column.count == 1)
                });
                assert!(state.view.show_filter_with_query("needle"));
                wait_until(|| {
                    let Some(selection) = visible_collection_selection(&state.view.widget()) else {
                        return false;
                    };
                    if state.view.selected_search_results().is_none() || selection.n_items() != 1 {
                        return false;
                    }
                    selection.select_item(0, true);
                    state.view.selected_search_results().is_some_and(|entries| {
                        entries.len() == 1 && entries[0].location == Location::local(&target)
                    })
                });
                let selected = accept(&state, &result);
                assert_eq!(
                    selected
                        .uris()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                    vec![gio::File::for_path(&target).uri().to_string()]
                );
            }
        },
    );
}

#[test]
fn folder_only_requests_filter_recursive_results_for_single_and_multiple_selection() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::type_filters::folder_only_requests_filter_recursive_results_for_single_and_multiple_selection",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_filter_include_subfolders(true);
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                for multiple in [false, true] {
                    let root = tempfile::tempdir().expect("fixture");
                    for name in ["needle-directory-a", "needle-directory-b"] {
                        std::fs::create_dir(root.path().join(name)).expect("folder");
                    }
                    for index in 0..120 {
                        std::fs::write(root.path().join(format!("needle-{index:03}.txt")), "file")
                            .expect("file");
                    }
                    let mut request = request(root.path().to_path_buf());
                    request.kind = ChooserKind::Open {
                        directory: true,
                        multiple,
                    };
                    let (state, result) = open_chooser(request);
                    let browser = state.view.browser();
                    wait_column(&browser, 0, 2);
                    assert!((0..2).all(|position| {
                        browser
                            .entry_at(0, position)
                            .is_some_and(|entry| entry.is_directory())
                    }));
                    assert!(state.view.show_filter_with_query("needle"));
                    wait_until(|| {
                        state.view.selected_search_results().is_some()
                            && visible_collection_selection(&state.view.widget())
                                .is_some_and(|results| results.n_items() == 2)
                    });
                    let results =
                        visible_collection_selection(&state.view.widget()).expect("results");
                    results.select_item(0, true);
                    if multiple {
                        results.select_item(1, false);
                    }
                    let entries = state
                        .view
                        .selected_search_results()
                        .expect("search selection");
                    assert_eq!(entries.len(), if multiple { 2 } else { 1 });
                    assert!(entries.iter().all(|entry| entry.is_directory()));
                    let mut expected = entries
                        .iter()
                        .map(|entry| {
                            gio::File::for_path(entry.location.native_path().expect("local"))
                                .uri()
                                .to_string()
                        })
                        .collect::<Vec<_>>();
                    expected.sort();
                    let selected = accept(&state, &result);
                    let mut actual = selected
                        .uris()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>();
                    actual.sort();
                    assert_eq!(actual, expected, "{mode:?}, multiple={multiple}");
                }
            }
        },
    );
}

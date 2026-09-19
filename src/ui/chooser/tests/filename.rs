// SPDX-License-Identifier: MIT

use super::acceptance::{request, search_results_list, visible_collection_selection, wait_until};
use super::*;

#[test]
fn save_filename_follows_widget_selection_without_accepting() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::filename::save_filename_follows_widget_selection_without_accepting",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            for name in ["first.txt", "second.txt"] {
                std::fs::write(root.path().join(name), name).expect("file");
            }
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let mut request = request(root.path().to_path_buf());
                request.kind = ChooserKind::SaveFile {
                    current_name: Some("suggested.txt".into()),
                };
                let completed = Rc::new(Cell::new(false));
                let received = completed.clone();
                let state = build_chooser(request, Arc::new(AtomicBool::new(false)), move |_| {
                    received.set(true)
                })
                .expect("chooser");
                let browser = state.view.browser();
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading && column.count == 3)
                });
                let filename = state.filename.as_ref().expect("filename");
                assert_eq!(filename.text(), "suggested.txt");
                let selection =
                    visible_collection_selection(&state.view.widget()).expect("collection");
                for name in ["first.txt", "second.txt"] {
                    let position = (0..3)
                        .find(|position| {
                            browser
                                .entry_at(0, *position)
                                .is_some_and(|entry| entry.display_name == name)
                        })
                        .expect("file position");
                    browser.commit_selection();
                    assert!(
                        selection.select_item(position as u32, true),
                        "{mode:?}: {name}"
                    );
                    assert_eq!(
                        browser
                            .selected_entries()
                            .first()
                            .map(|entry| entry.display_name.clone()),
                        Some(name.to_owned()),
                        "{mode:?}"
                    );
                    wait_until(|| filename.text() == name);
                    assert!(!completed.get());
                }
                let folder = (0..3)
                    .find(|position| {
                        browser
                            .entry_at(0, *position)
                            .is_some_and(|entry| entry.is_directory())
                    })
                    .expect("folder position");
                selection.select_item(folder as u32, true);
                assert_eq!(filename.text(), "second.txt");
                assert!(!completed.get());
                state.window.close();
            }
        },
    );
}

#[test]
fn save_filename_follows_recursive_search_selection() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::filename::save_filename_follows_recursive_search_selection",
        || {
            crate::ui::prepare_portal_ui();
            for mode in [BrowserMode::List, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                let child = root.path().join("child");
                std::fs::create_dir(&child).expect("folder");
                std::fs::write(child.join("target.txt"), "text").expect("file");
                let mut request = request(root.path().to_path_buf());
                request.kind = ChooserKind::SaveFile {
                    current_name: Some("suggested.txt".into()),
                };
                let state = build_chooser(request, Arc::new(AtomicBool::new(false)), |_| {})
                    .expect("chooser");
                wait_until(|| {
                    state
                        .view
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
                });
                state.view.show_filter_with_query("target");
                wait_until(|| state.view.selected_search_results().is_some());
                if mode == BrowserMode::Columns {
                    wait_until(|| {
                        visible_collection_selection(&state.view.widget())
                            .is_some_and(|selection| selection.n_items() == 1)
                    });
                    visible_collection_selection(&state.view.widget())
                        .expect("search results")
                        .select_item(0, true);
                } else {
                    wait_until(|| search_results_list(&state.view.widget()).is_some());
                    let list = search_results_list(&state.view.widget()).expect("search results");
                    list.select_row(list.row_at_index(0).as_ref());
                }
                wait_until(|| state.filename.as_ref().expect("filename").text() == "target.txt");
                state.window.close();
            }
        },
    );
}

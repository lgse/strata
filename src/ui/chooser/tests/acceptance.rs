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

fn search_results_list(widget: &gtk::Widget) -> Option<gtk::ListBox> {
    if let Ok(list) = widget.clone().downcast::<gtk::ListBox>()
        && list.has_css_class("file-list")
        && list.row_at_index(0).is_some()
    {
        return Some(list);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(list) = search_results_list(&current) {
            return Some(list);
        }
        child = current.next_sibling();
    }
    None
}

fn select_first_two_file_list_items(widget: &gtk::Widget) {
    if let Ok(list) = widget.clone().downcast::<gtk::ListView>()
        && list.has_css_class("file-list")
        && let Some(selection) = list.model()
        && selection.n_items() >= 2
    {
        selection.select_item(0, true);
        selection.select_item(1, false);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        select_first_two_file_list_items(&current);
        child = current.next_sibling();
    }
}

fn visible_collection_selection(widget: &gtk::Widget) -> Option<gtk::SelectionModel> {
    let selection = widget
        .clone()
        .downcast::<gtk::ListView>()
        .ok()
        .and_then(|view| view.is_mapped().then(|| view.model()).flatten())
        .or_else(|| {
            widget
                .clone()
                .downcast::<gtk::GridView>()
                .ok()
                .and_then(|view| view.is_mapped().then(|| view.model()).flatten())
        });
    if selection.is_some() {
        return selection;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(selection) = visible_collection_selection(&current) {
            return Some(selection);
        }
        child = current.next_sibling();
    }
    None
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
            let first = root.path().join("folder/nested-a.txt");
            let nested = root.path().join("folder/nested-b.txt");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(&first, "first").expect("first nested file");
            std::fs::write(&nested, "second").expect("second nested file");
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
            wait_until(|| {
                search_results_list(&state.view.widget())
                    .is_some_and(|list| list.row_at_index(1).is_some())
            });
            let list = search_results_list(&state.view.widget()).expect("search results");
            list.select_row(list.row_at_index(0).as_ref());
            assert_eq!(
                state
                    .view
                    .selected_search_results()
                    .expect("active search")
                    .first()
                    .expect("first search selection")
                    .location,
                Location::local(&first)
            );
            list.select_row(list.row_at_index(1).as_ref());
            wait_until(|| {
                state.view.selected_search_results().is_some_and(|entries| {
                    entries
                        .first()
                        .is_some_and(|entry| entry.location == Location::local(&nested))
                })
            });
            assert!(
                state.completion.borrow().is_some(),
                "selecting a recursive result must not accept it"
            );
            assert!(
                !state.error.is_visible(),
                "selection must not show an error"
            );
            state.accept_button.emit_clicked();
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
fn dismissing_recursive_results_then_extending_widget_selection_accepts_current_files() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::dismissing_recursive_results_then_extending_widget_selection_accepts_current_files",
        || {
            crate::ui::prepare_portal_ui();
            ThemeManager::shared().set_browser_mode(BrowserMode::Icons);
            let root = tempfile::tempdir().expect("fixture");
            let nested = root.path().join("folder/nested.txt");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(&nested, "nested").expect("nested file");
            for name in ["alpha.txt", "beta.txt"] {
                std::fs::write(root.path().join(name), name).expect("root file");
            }
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut chooser_request = request(root.path().to_path_buf());
            chooser_request.kind = ChooserKind::Open {
                directory: false,
                multiple: true,
            };
            let state = build_chooser(
                chooser_request,
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
                    .is_some_and(|column| !column.loading && column.count == 3)
            });

            state.view.show_filter_with_query("nested");
            wait_until(|| search_results_list(&state.view.widget()).is_some());
            let list = search_results_list(&state.view.widget()).expect("search results");
            list.select_row(list.row_at_index(0).as_ref());
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
        "ui::chooser::tests::acceptance::active_recursive_search_without_selection_does_not_accept_hidden_browser_selection",
        || {
            crate::ui::prepare_portal_ui();
            ThemeManager::shared().set_browser_mode(BrowserMode::Icons);
            let root = tempfile::tempdir().expect("fixture");
            let hidden_selection = root.path().join("selected.txt");
            let nested = root.path().join("folder/nested.txt");
            std::fs::create_dir(nested.parent().expect("parent")).expect("folder");
            std::fs::write(&hidden_selection, "selected").expect("root file");
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
                    .is_some_and(|column| !column.loading && column.count == 2)
            });

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
            wait_until(|| search_results_list(&state.view.widget()).is_some());
            let list = search_results_list(&state.view.widget()).expect("search results");
            list.select_row(list.row_at_index(0).as_ref());
            wait_until(|| {
                state
                    .view
                    .selected_search_results()
                    .is_some_and(|entries| entries.len() == 1)
            });
            list.unselect_all();
            wait_until(|| state.view.selected_search_results() == Some(Vec::new()));

            state.accept_button.emit_clicked();
            assert!(result.borrow().is_none(), "deselection must not accept");
            assert!(state.error.is_visible(), "deselection must show an error");

            state.view.show_filter_with_query("no-matches");
            wait_until(|| {
                list.row_at_index(0).is_none()
                    && state.view.selected_search_results() == Some(Vec::new())
            });
            state.accept_button.emit_clicked();
            assert!(result.borrow().is_none(), "no results must not accept");
            assert!(state.error.is_visible(), "no results must show an error");
        },
    );
}

#[test]
fn columns_recursive_multi_selection_accepts_every_selected_file() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::columns_recursive_multi_selection_accepts_every_selected_file",
        || {
            crate::ui::prepare_portal_ui();
            ThemeManager::shared().set_browser_mode(BrowserMode::Columns);
            let root = tempfile::tempdir().expect("fixture");
            let paths = [
                root.path().join("one/nested-a.txt"),
                root.path().join("two/nested-b.txt"),
            ];
            for path in &paths {
                std::fs::create_dir(path.parent().expect("parent")).expect("folder");
                std::fs::write(path, "nested").expect("nested file");
            }
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut chooser_request = request(root.path().to_path_buf());
            chooser_request.kind = ChooserKind::Open {
                directory: false,
                multiple: true,
            };
            let state = build_chooser(
                chooser_request,
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
            wait_until(|| {
                select_first_two_file_list_items(&state.view.widget());
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

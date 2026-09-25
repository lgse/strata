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

#[track_caller]
pub(super) fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "chooser did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[derive(Clone)]
pub(super) enum SearchResults {
    Rows(gtk::ListBox),
    Collection(gtk::SelectionModel),
}

impl SearchResults {
    pub(super) fn n_items(&self) -> u32 {
        match self {
            Self::Rows(list) => list.observe_children().n_items(),
            Self::Collection(selection) => selection.n_items(),
        }
    }

    pub(super) fn select(&self, position: u32, exclusive: bool) {
        match self {
            Self::Rows(list) => {
                if exclusive {
                    list.unselect_all();
                }
                list.select_row(list.row_at_index(position as i32).as_ref());
            }
            Self::Collection(selection) => {
                selection.select_item(position, exclusive);
            }
        }
    }

    fn unselect_all(&self) {
        match self {
            Self::Rows(list) => list.unselect_all(),
            Self::Collection(selection) => {
                selection.unselect_all();
            }
        }
    }
}

pub(super) fn search_results(widget: &gtk::Widget) -> Option<SearchResults> {
    if let Some(list) = widget.downcast_ref::<gtk::ListBox>()
        && list.has_css_class("file-list")
        && list.is_mapped()
        && list.row_at_index(0).is_some()
    {
        return Some(SearchResults::Rows(list.clone()));
    }
    if let Some(list) = widget.downcast_ref::<gtk::ListView>()
        && list.has_css_class("search-results")
        && list.is_mapped()
        && let Some(selection) = list.model()
    {
        return Some(SearchResults::Collection(selection));
    }
    if let Some(grid) = widget.downcast_ref::<gtk::GridView>()
        && grid.has_css_class("search-results")
        && grid.is_mapped()
        && let Some(selection) = grid.model()
    {
        return Some(SearchResults::Collection(selection));
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(results) = search_results(&current) {
            return Some(results);
        }
        child = current.next_sibling();
    }
    None
}

fn select_first_two_file_list_items(widget: &gtk::Widget) {
    if let Some(list) = widget.downcast_ref::<gtk::ListView>()
        && list.has_css_class("file-list")
        && let Some(selection) = list.model()
        && selection.n_items() >= 2
    {
        selection.select_item(0, true);
        selection.select_item(1, false);
    }
    if let Some(grid) = widget.downcast_ref::<gtk::GridView>()
        && grid.has_css_class("file-icons")
        && let Some(selection) = grid.model()
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

fn select_first_file_list_item(widget: &gtk::Widget) {
    if let Some(list) = widget.downcast_ref::<gtk::ListView>()
        && list.has_css_class("file-list")
        && list.is_mapped()
        && let Some(selection) = list.model()
        && selection.n_items() >= 1
    {
        selection.select_item(0, true);
    }
    if let Some(grid) = widget.downcast_ref::<gtk::GridView>()
        && grid.has_css_class("file-icons")
        && grid.is_mapped()
        && let Some(selection) = grid.model()
        && selection.n_items() >= 1
    {
        selection.select_item(0, true);
    }
    if let Ok(list) = widget.clone().downcast::<gtk::ListBox>()
        && list.has_css_class("file-list")
        && list.is_mapped()
        && let Some(row) = list.row_at_index(0)
    {
        list.select_row(Some(&row));
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        select_first_file_list_item(&current);
        child = current.next_sibling();
    }
}

fn select_first_file_list_item_in_first_list(widget: &gtk::Widget) -> bool {
    if let Some(list) = widget.downcast_ref::<gtk::ListView>()
        && list.has_css_class("file-list")
        && list.is_mapped()
        && let Some(selection) = list.model()
        && selection.n_items() >= 1
    {
        selection.select_item(0, true);
        return true;
    }
    if let Some(grid) = widget.downcast_ref::<gtk::GridView>()
        && grid.has_css_class("file-icons")
        && grid.is_mapped()
        && let Some(selection) = grid.model()
        && selection.n_items() >= 1
    {
        selection.select_item(0, true);
        return true;
    }
    if let Ok(list) = widget.clone().downcast::<gtk::ListBox>()
        && list.has_css_class("file-list")
        && list.is_mapped()
        && let Some(row) = list.row_at_index(0)
    {
        list.select_row(Some(&row));
        return true;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if select_first_file_list_item_in_first_list(&current) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

fn nth_filter_entry(widget: &gtk::Widget, index: usize) -> Option<gtk::Entry> {
    let mut entries = Vec::new();
    collect_filter_entries(widget, &mut entries);
    entries.into_iter().nth(index)
}

fn collect_filter_entries(widget: &gtk::Widget, entries: &mut Vec<gtk::Entry>) {
    if let Ok(entry) = widget.clone().downcast::<gtk::Entry>()
        && entry.has_css_class("column-filter-entry")
    {
        entries.push(entry);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        collect_filter_entries(&current, entries);
        child = current.next_sibling();
    }
}

pub(super) fn visible_collection_selection(widget: &gtk::Widget) -> Option<gtk::SelectionModel> {
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

pub(super) fn request(root: PathBuf) -> ChooserRequest {
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
fn full_file_path_selects_and_accepts_the_named_file() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::full_file_path_selects_and_accepts_the_named_file",
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

                let result = Rc::new(RefCell::new(None));
                let received = result.clone();
                let state = build_chooser(
                    request(initial.clone()),
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

                state.accept_button.emit_clicked();
                wait_until(|| result.borrow().is_some());
                let selected = result
                    .borrow_mut()
                    .take()
                    .expect("result")
                    .expect("accepted");
                assert_eq!(selected.uris().len(), 1, "{mode:?}");
                assert_eq!(
                    selected.uris()[0].to_string(),
                    gio::File::for_path(&target).uri(),
                    "{mode:?}"
                );
                state.window.close();
            }
        },
    );
}

#[test]
fn directory_confirmation_distinguishes_load_cursor_from_explicit_selection() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::directory_confirmation_distinguishes_load_cursor_from_explicit_selection",
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
                    let result = Rc::new(RefCell::new(None));
                    let received = result.clone();
                    let mut chooser_request = request(root.path().to_path_buf());
                    chooser_request.kind = ChooserKind::Open {
                        directory: true,
                        multiple: false,
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
                            .is_some_and(|column| !column.loading && column.count == 1)
                    });
                    assert!(browser.selection_is_load_cursor(), "{mode:?}");
                    if explicit {
                        browser.select(0, 0);
                        assert!(!browser.selection_is_load_cursor(), "{mode:?}");
                    }
                    state.accept_button.emit_clicked();
                    wait_until(|| result.borrow().is_some());
                    let selected = result
                        .borrow_mut()
                        .take()
                        .expect("result")
                        .expect("accepted");
                    let expected = if explicit {
                        child.as_path()
                    } else {
                        root.path()
                    };
                    assert_eq!(selected.uris().len(), 1, "{mode:?}, explicit={explicit}");
                    assert_eq!(
                        selected.uris()[0].to_string(),
                        gio::File::for_path(expected).uri(),
                        "{mode:?}, explicit={explicit}"
                    );
                    state.window.close();
                }
            }
        },
    );
}

#[test]
fn filter_dropdown_select_file_click_open_accepts_filtered_file() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filter_dropdown_select_file_click_open_accepts_filtered_file",
        || {
            crate::ui::prepare_portal_ui();
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                std::fs::write(root.path().join("readme.md"), "readme").expect("file");
                std::fs::write(root.path().join("notes.txt"), "notes").expect("file");
                let result = Rc::new(RefCell::new(None));
                let received = result.clone();
                let mut chooser_request = request(root.path().to_path_buf());
                chooser_request.filters = vec![
                    FileFilter::new("All files").glob("*"),
                    FileFilter::new("Text").glob("*.txt"),
                ];
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
                        .is_some_and(|column| !column.loading && column.count == 2)
                });

                {
                    let dropdown = state.filter_dropdown.as_ref().expect("filter dropdown");
                    let changed = dropdown.changed.borrow();
                    changed.as_ref().expect("filter callback")(1);
                }

                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading && column.count == 1)
                });

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
                state.accept_button.emit_clicked();
                wait_until(|| result.borrow().is_some());
                let selected = result
                    .borrow_mut()
                    .take()
                    .expect("result")
                    .expect("accepted");
                assert_eq!(selected.uris().len(), 1, "{mode:?}");
                assert_eq!(
                    selected.uris()[0].to_string(),
                    gio::File::for_path(root.path().join("notes.txt")).uri(),
                    "{mode:?}"
                );
                state.window.close();
            }
        },
    );
}

#[test]
fn type_filter_refresh_preserves_recursive_query_and_replaces_eligible_results() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::type_filter_refresh_preserves_recursive_query_and_replaces_eligible_results",
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
                let result = Rc::new(RefCell::new(None));
                let received = result.clone();
                let state =
                    build_chooser(request, Arc::new(AtomicBool::new(false)), move |value| {
                        received.replace(Some(value));
                    })
                    .expect("chooser");
                let browser = state.view.browser();
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading && column.count == 2)
                });
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
                state.accept_button.emit_clicked();
                wait_until(|| result.borrow().is_some());
                let selected = result
                    .borrow_mut()
                    .take()
                    .expect("result")
                    .expect("accepted");
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
        "ui::chooser::tests::acceptance::type_filtered_search_finds_eligible_entries_beyond_the_first_ranked_candidates",
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
                let result = Rc::new(RefCell::new(None));
                let received = result.clone();
                let state =
                    build_chooser(request, Arc::new(AtomicBool::new(false)), move |value| {
                        received.replace(Some(value));
                    })
                    .expect("chooser");
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
                state.accept_button.emit_clicked();
                wait_until(|| result.borrow().is_some());
                let selected = result
                    .borrow_mut()
                    .take()
                    .expect("result")
                    .expect("accepted");
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
        "ui::chooser::tests::acceptance::folder_only_requests_filter_recursive_results_for_single_and_multiple_selection",
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
                    let result = Rc::new(RefCell::new(None));
                    let received = result.clone();
                    let state =
                        build_chooser(request, Arc::new(AtomicBool::new(false)), move |value| {
                            received.replace(Some(value));
                        })
                        .expect("chooser");
                    let browser = state.view.browser();
                    wait_until(|| {
                        browser
                            .column_snapshot(0)
                            .is_some_and(|column| !column.loading && column.count == 2)
                    });
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
                    state.accept_button.emit_clicked();
                    wait_until(|| result.borrow().is_some());
                    let selected = result
                        .borrow_mut()
                        .take()
                        .expect("result")
                        .expect("accepted");
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

#[test]
fn filter_text_select_file_click_open_accepts_root_file() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filter_text_select_file_click_open_accepts_root_file",
        || {
            crate::ui::prepare_portal_ui();
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                std::fs::write(root.path().join("readme.md"), "readme").expect("file");
                std::fs::write(root.path().join("todo.txt"), "todo").expect("file");
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
                state.accept_button.emit_clicked();
                wait_until(|| result.borrow().is_some());
                let selected = result
                    .borrow_mut()
                    .take()
                    .expect("result")
                    .expect("accepted");
                assert_eq!(selected.uris().len(), 1, "{mode:?}");
                assert_eq!(
                    selected.uris()[0].to_string(),
                    gio::File::for_path(root.path().join("readme.md")).uri(),
                    "{mode:?}"
                );
                state.window.close();
            }
        },
    );
}

#[test]
fn columns_filter_after_navigating_into_subfolder_accepts_search_result() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::columns_filter_after_navigating_into_subfolder_accepts_search_result",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::Columns);
            let root = tempfile::tempdir().expect("fixture");
            std::fs::write(root.path().join("readme.md"), "readme").expect("file");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(root.path().join("folder/notes.txt"), "notes").expect("nested file");
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

            let folder_position = (0..2)
                .find(|&position| {
                    browser
                        .entry_at(0, position)
                        .is_some_and(|entry| entry.is_directory())
                })
                .expect("folder position");
            browser.activate(0, folder_position);
            wait_until(|| {
                browser
                    .column_snapshot(1)
                    .is_some_and(|column| !column.loading && column.count == 1)
            });

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
                gio::File::for_path(root.path().join("folder/notes.txt")).uri()
            );
        },
    );
}

#[test]
fn columns_filter_in_non_active_column_accepts_search_result_on_open() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::columns_filter_in_non_active_column_accepts_search_result_on_open",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::Columns);
            let root = tempfile::tempdir().expect("fixture");
            std::fs::write(root.path().join("readme.md"), "readme").expect("file");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(root.path().join("folder/notes.txt"), "notes").expect("nested file");
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

            let folder_position = (0..2)
                .find(|&position| {
                    browser
                        .entry_at(0, position)
                        .is_some_and(|entry| entry.is_directory())
                })
                .expect("folder position");
            browser.activate(0, folder_position);
            wait_until(|| {
                browser
                    .column_snapshot(1)
                    .is_some_and(|column| !column.loading && column.count == 1)
            });

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
                gio::File::for_path(root.path().join("readme.md")).uri()
            );
        },
    );
}

#[test]
fn filtered_selection_only_accepts_on_enter_or_open_with_exact_nested_path() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::filtered_selection_only_accepts_on_enter_or_open_with_exact_nested_path",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
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
                gio::File::for_path(second_selected.native_path().expect("native path")).uri()
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
            PreferenceManager::shared().set_browser_mode(BrowserMode::Icons);
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
            PreferenceManager::shared().set_browser_mode(BrowserMode::Icons);
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
        "ui::chooser::tests::acceptance::recursive_multi_selection_accepts_every_selected_file_in_all_modes",
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

#[test]
fn save_file_ignores_load_cursor() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save_file_ignores_load_cursor",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(root.path().join("child")).expect("child folder");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFile {
                current_name: Some("output.txt".into()),
            };
            let state = build_chooser(
                save_request,
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
                    .is_some_and(|column| !column.loading && column.count == 1)
            });
            assert!(browser.selection_is_load_cursor());
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
                gio::File::for_path(root.path().join("output.txt")).uri()
            );
            state.window.close();
        },
    );
}

#[test]
fn save_file_accepts_selected_folder_without_navigating_into_it() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save_file_accepts_selected_folder_without_navigating_into_it",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let target_folder = root.path().join("target_folder");
            std::fs::create_dir(&target_folder).expect("folder");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFile {
                current_name: Some("output.txt".into()),
            };
            let state = build_chooser(
                save_request,
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
                    .is_some_and(|column| !column.loading && column.count == 1)
            });

            browser.select(0, 0);
            wait_until(|| {
                browser
                    .selected_entries()
                    .first()
                    .is_some_and(|entry| entry.location == Location::local(&target_folder))
            });

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
                gio::File::for_path(target_folder.join("output.txt")).uri()
            );
        },
    );
}

#[test]
fn save_files_accepts_selected_folder_without_navigating_into_it() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save_files_accepts_selected_folder_without_navigating_into_it",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let target_folder = root.path().join("target_folder");
            std::fs::create_dir(&target_folder).expect("folder");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFiles {
                names: vec!["one.txt".into(), "two.txt".into()],
            };
            let state = build_chooser(
                save_request,
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
                    .is_some_and(|column| !column.loading && column.count == 1)
            });

            browser.select(0, 0);
            wait_until(|| {
                browser
                    .selected_entries()
                    .first()
                    .is_some_and(|entry| entry.location == Location::local(&target_folder))
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
            let mut expected = [target_folder.join("one.txt"), target_folder.join("two.txt")]
                .map(|path| gio::File::for_path(path).uri().to_string())
                .to_vec();
            expected.sort();
            assert_eq!(uris, expected);
        },
    );
}

#[test]
fn save_file_in_icons_mode_accepts_selected_folder_without_navigating_into_it() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save_file_in_icons_mode_accepts_selected_folder_without_navigating_into_it",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::Icons);
            let root = tempfile::tempdir().expect("fixture");
            let target_folder = root.path().join("target_folder");
            std::fs::create_dir(&target_folder).expect("folder");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFile {
                current_name: Some("output.txt".into()),
            };
            let state = build_chooser(
                save_request,
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
                    .is_some_and(|column| !column.loading && column.count == 1)
            });

            browser.select(0, 0);
            wait_until(|| {
                browser
                    .selected_entries()
                    .first()
                    .is_some_and(|entry| entry.location == Location::local(&target_folder))
            });

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
                gio::File::for_path(target_folder.join("output.txt")).uri()
            );
        },
    );
}

#[test]
fn save_file_with_search_results_accepts_selected_folder_without_navigating_into_it() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save_file_with_search_results_accepts_selected_folder_without_navigating_into_it",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let target_folder = root.path().join("sub/target_folder");
            std::fs::create_dir_all(&target_folder).expect("folders");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFile {
                current_name: Some("output.txt".into()),
            };
            let state = build_chooser(
                save_request,
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

            state.view.show_filter_with_query("target");
            wait_until(|| {
                search_results(&state.view.widget()).is_some_and(|results| results.n_items() > 0)
            });
            let results = search_results(&state.view.widget()).expect("search results");
            results.select(0, true);
            wait_until(|| {
                state.view.selected_search_results().is_some_and(|entries| {
                    entries
                        .first()
                        .is_some_and(|e| e.location == Location::local(&target_folder))
                })
            });

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
                gio::File::for_path(target_folder.join("output.txt")).uri()
            );
        },
    );
}

#[test]
fn save_file_with_selected_file_saves_to_active_folder() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save_file_with_selected_file_saves_to_active_folder",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let existing_file = root.path().join("existing.txt");
            std::fs::write(&existing_file, "existing").expect("file");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFile {
                current_name: Some("new_file.txt".into()),
            };
            let state = build_chooser(
                save_request,
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
                    .is_some_and(|column| !column.loading && column.count == 1)
            });

            let filename = state.filename.as_ref().expect("filename");
            assert_eq!(filename.text(), "new_file.txt");
            assert!(browser.selection_is_load_cursor());
            assert_eq!(browser.selected_entries().len(), 1);
            browser.commit_selection();
            browser.set_selection(0, &[0], Some(0));
            assert!(!browser.selection_is_load_cursor());
            wait_until(|| filename.text() == "existing.txt");
            for (invalid, message) in [("", "Enter a name"), ("bad/name", "Names cannot contain /")]
            {
                filename.set_text(invalid);
                state.accept_button.emit_clicked();
                assert!(result.borrow().is_none());
                assert!(filename.has_css_class("error"));
                assert_eq!(filename.tooltip_text().as_deref(), Some(message));
                assert!(
                    !state.error.is_visible(),
                    "filename errors belong to the field, not a second banner"
                );
                assert!(!root.path().join("bad").exists());
            }
            filename.set_text("new_file.txt");
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
                gio::File::for_path(root.path().join("new_file.txt")).uri()
            );
        },
    );
}

fn key_controller(window: &gtk::Window) -> gtk::EventControllerKey {
    let controllers = window.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(controller) = controllers
            .item(index)
            .and_downcast::<gtk::EventControllerKey>()
        {
            return controller;
        }
    }
    panic!("no EventControllerKey on the chooser window");
}

fn press(window: &gtk::Window, key: gtk::gdk::Key) -> bool {
    key_controller(window).emit_by_name::<bool>(
        "key-pressed",
        &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
    )
}

#[test]
fn arrow_scope_keeps_left_in_the_chooser_file_view() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::arrow_scope_keeps_left_in_the_chooser_file_view",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_arrow_navigation_scoped(true);
            let root = tempfile::tempdir().expect("fixture");
            for name in ["a.txt", "b.txt", "c.txt"] {
                std::fs::write(root.path().join(name), "text").expect("fixture file");
            }
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let state = build_chooser(
                    request(root.path().to_path_buf()),
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .expect("chooser");
                state.view.set_view_mode(mode);
                let browser = state.view.browser();
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading && column.count == 3)
                });
                for scoped in [true, false, true] {
                    PreferenceManager::shared().set_arrow_navigation_scoped(scoped);
                    for key in [gtk::gdk::Key::Left, gtk::gdk::Key::Up] {
                        browser.select(0, 0);
                        browser.focus_active();
                        wait_until(|| {
                            state.view.item_view_has_focus() && state.view.item_at_sidebar_edge()
                        });
                        press(&state.window, key);
                        assert_eq!(
                            state.view.item_view_has_focus(),
                            scoped,
                            "{mode:?}: {key:?} with arrow scope {scoped}"
                        );
                    }
                }
                state.window.close();
            }
        },
    );
}

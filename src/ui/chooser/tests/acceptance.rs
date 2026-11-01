// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser_modes::BrowserMode;
use std::{
    cell::RefCell,
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

mod filter_search;
mod name_field;
mod open_paths;
mod save;
mod type_filters;

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

fn mapped_with_class(widget: &impl IsA<gtk::Widget>, class: &str) -> bool {
    widget.has_css_class(class) && widget.is_mapped()
}

fn file_list_rows(widget: &gtk::Widget) -> Option<gtk::ListBox> {
    widget
        .downcast_ref::<gtk::ListBox>()
        .filter(|list| mapped_with_class(*list, "file-list"))
        .filter(|list| list.row_at_index(0).is_some())
        .cloned()
}

fn search_result_collection(widget: &gtk::Widget) -> Option<gtk::SelectionModel> {
    if !mapped_with_class(widget, "search-results") {
        return None;
    }
    let list = widget
        .downcast_ref::<gtk::ListView>()
        .and_then(|list| list.model());
    list.or_else(|| {
        widget
            .downcast_ref::<gtk::GridView>()
            .and_then(|grid| grid.model())
    })
}

fn children(widget: &gtk::Widget) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
}

pub(super) fn search_results(widget: &gtk::Widget) -> Option<SearchResults> {
    if let Some(list) = file_list_rows(widget) {
        return Some(SearchResults::Rows(list));
    }
    if let Some(selection) = search_result_collection(widget) {
        return Some(SearchResults::Collection(selection));
    }
    children(widget).find_map(|child| search_results(&child))
}

/// The model of a file list or icon grid holding at least `minimum` items.
fn file_view_model(widget: &gtk::Widget, minimum: u32) -> Option<gtk::SelectionModel> {
    let list = widget
        .downcast_ref::<gtk::ListView>()
        .filter(|list| list.has_css_class("file-list"))
        .and_then(|list| list.model());
    let grid = || {
        widget
            .downcast_ref::<gtk::GridView>()
            .filter(|grid| grid.has_css_class("file-icons"))
            .and_then(|grid| grid.model())
    };
    list.or_else(grid)
        .filter(|selection| selection.n_items() >= minimum)
}

/// Visits `widget` and then its descendants, reading each child list as it goes.
fn visit_live(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(current) = child {
        visit_live(&current, visit);
        child = current.next_sibling();
    }
}

fn select_first_two_file_list_items(widget: &gtk::Widget) {
    visit_live(widget, &mut |widget| {
        if let Some(selection) = file_view_model(widget, 2) {
            selection.select_item(0, true);
            selection.select_item(1, false);
        }
    });
}

/// Selects the first item of `widget` when it is a shown file view; returns whether
/// it was one.
fn select_first_item_of(widget: &gtk::Widget) -> bool {
    if let Some(selection) = file_view_model(widget, 1).filter(|_| widget.is_mapped()) {
        selection.select_item(0, true);
        return true;
    }
    let Some(list) = widget
        .downcast_ref::<gtk::ListBox>()
        .filter(|list| mapped_with_class(*list, "file-list"))
    else {
        return false;
    };
    let Some(row) = list.row_at_index(0) else {
        return false;
    };
    list.select_row(Some(&row));
    true
}

fn select_first_file_list_item(widget: &gtk::Widget) {
    visit_live(widget, &mut |widget| {
        select_first_item_of(widget);
    });
}

fn select_first_file_list_item_in_first_list(widget: &gtk::Widget) -> bool {
    if select_first_item_of(widget) {
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

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        result.extend(descendants(&current));
    }
    result
}

type Completion = Rc<RefCell<Option<ashpd::backend::Result<SelectedFiles>>>>;

/// Builds a chooser for `request` whose completion lands in the returned cell.
fn open_chooser(request: ChooserRequest) -> (Rc<ChooserState>, Completion) {
    let result = Completion::default();
    let received = result.clone();
    let state = build_chooser(request, Arc::new(AtomicBool::new(false)), move |value| {
        received.replace(Some(value));
    })
    .expect("chooser");
    (state, result)
}

/// Clicks the accept button and returns the files the chooser accepted.
fn accept(state: &ChooserState, result: &Completion) -> SelectedFiles {
    state.accept_button.emit_clicked();
    wait_until(|| result.borrow().is_some());
    result
        .borrow_mut()
        .take()
        .expect("result")
        .expect("accepted")
}

fn wait_listed(browser: &crate::app::Browser, depth: usize) {
    wait_until(|| {
        browser
            .column_snapshot(depth)
            .is_some_and(|column| !column.loading)
    });
}

fn wait_column(browser: &crate::app::Browser, depth: usize, count: usize) {
    wait_until(|| {
        browser
            .column_snapshot(depth)
            .is_some_and(|column| !column.loading && column.count == count)
    });
}

fn assert_single_uri(selected: &SelectedFiles, path: impl AsRef<Path>, context: &str) {
    assert_eq!(selected.uris().len(), 1, "{context}");
    assert_eq!(
        selected.uris()[0].to_string(),
        gio::File::for_path(path).uri(),
        "{context}"
    );
}

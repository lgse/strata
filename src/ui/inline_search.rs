// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{glib, prelude::*};

use crate::{
    app::Browser,
    services::{SearchEvent, SearchHandle, SearchItem, index_tree},
};

struct State {
    entry: glib::WeakRef<gtk::Entry>,
    stack: gtk::Stack,
    list: gtk::ListBox,
    status: gtk::Label,
    items: RefCell<Vec<SearchItem>>,
    handle: RefCell<Option<SearchHandle>>,
    generation: Cell<u64>,
}

/// Keeps the view's normal presentation intact when the recursive query is dismissed.
pub(super) fn wrap(
    content: &impl IsA<gtk::Widget>,
    entry: &gtk::Entry,
    root: Option<PathBuf>,
    browser: &Rc<Browser>,
) -> gtk::Widget {
    let Some(root) = root else {
        return content.clone().upcast();
    };
    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    stack.add_named(content, Some("files"));
    let results = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let status = gtk::Label::new(None);
    status.add_css_class("status-message");
    results.append(&status);
    let list = gtk::ListBox::new();
    list.add_css_class("file-list");
    list.set_activate_on_single_click(false);
    list.set_selection_mode(gtk::SelectionMode::Single);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroll.add_css_class("fixed-scrollbar");
    results.append(&scroll);
    stack.add_named(&results, Some("search"));
    let state = Rc::new(State {
        entry: entry.downgrade(),
        stack: stack.clone(),
        list,
        status,
        items: RefCell::new(Vec::new()),
        handle: RefCell::new(None),
        generation: Cell::new(0),
    });
    let weak = Rc::downgrade(&state);
    let weak_browser = Rc::downgrade(browser);
    state.list.connect_row_activated(move |_, row| {
        if let Some(state) = weak.upgrade() {
            super::browser::activate_recursive_search_result(
                &weak_browser,
                &state.items,
                row.index() as u32,
            );
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&state);
    let weak_browser = Rc::downgrade(browser);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if state.handle.borrow().is_none()
            || modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK,
            )
        {
            return glib::Propagation::Proceed;
        }
        let current = state.list.selected_row().map(|row| row.index() as u32);
        if matches!(key, gtk::gdk::Key::Up | gtk::gdk::Key::Down) {
            let next = super::browser::search_result_navigation_position(
                current,
                state.items.borrow().len() as u32,
                if key == gtk::gdk::Key::Down { 1 } else { -1 },
            );
            if let Some(row) = next.and_then(|position| state.list.row_at_index(position as i32)) {
                state.list.select_row(Some(&row));
            }
            return glib::Propagation::Stop;
        }
        if super::browser::recursive_search_activation_key(key)
            && super::browser::activate_recursive_search_result(
                &weak_browser,
                &state.items,
                current.unwrap_or(0),
            )
        {
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    entry.add_controller(keys);
    let weak_browser = Rc::downgrade(browser);
    super::browser::debounce_filter_entry(entry, move |text| {
        let query = text.trim();
        if query.is_empty() {
            state.generation.set(state.generation.get().wrapping_add(1));
            state.handle.borrow_mut().take();
            state.items.borrow_mut().clear();
            state.list.remove_all();
            state.stack.set_visible_child_name("files");
            return;
        }
        state.stack.set_visible_child_name("search");
        state.list.remove_all();
        state.items.borrow_mut().clear();
        state.status.set_text("Searching…");
        state.status.set_visible(true);
        if let Some(handle) = state.handle.borrow().as_ref() {
            handle.query(query);
            return;
        }
        let generation = state.generation.get();
        let show_hidden = weak_browser
            .upgrade()
            .is_some_and(|browser| browser.preferences().show_hidden);
        let (handle, receiver) = index_tree(root.clone(), show_hidden);
        handle.query(query);
        state.handle.replace(Some(handle));
        let weak = Rc::downgrade(&state);
        let result_root = root.clone();
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(entry) = state.entry.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.generation.get() != generation || state.handle.borrow().is_none() {
                return glib::ControlFlow::Break;
            }
            let mut latest = None;
            for event in receiver.try_iter().take(8) {
                latest = Some(event);
            }
            if let Some(SearchEvent::Results {
                query: returned,
                items,
                indexing,
                truncated,
            }) = latest
                && !returned.is_empty()
                && returned == entry.text().trim()
            {
                state.list.remove_all();
                for item in &items {
                    let row = gtk::ListBoxRow::new();
                    // Keep keyboard focus in the query, away from file-operation shortcuts.
                    row.set_focusable(false);
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    line.add_css_class("file-row");
                    line.append(&crate::assets::primary_icon(
                        if item.is_directory {
                            crate::assets::icons::FOLDER
                        } else {
                            crate::assets::icons::DOCUMENTS
                        },
                        17,
                    ));
                    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
                    labels.set_hexpand(true);
                    let name = gtk::Label::builder()
                        .label(&item.name)
                        .xalign(0.0)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .build();
                    let path = relative_result_path(&result_root, &item.path);
                    let origin = gtk::Label::builder()
                        .label(&path)
                        .xalign(0.0)
                        .wrap(true)
                        .wrap_mode(gtk::pango::WrapMode::WordChar)
                        .lines(2)
                        .ellipsize(gtk::pango::EllipsizeMode::Middle)
                        .build();
                    origin.add_css_class("file-search-path");
                    labels.append(&name);
                    labels.append(&origin);
                    row.set_tooltip_text(Some(&path));
                    line.append(&labels);
                    row.set_child(Some(&line));
                    state.list.append(&row);
                }
                state.status.set_visible(items.is_empty() || truncated);
                state.status.set_text(if truncated {
                    "Showing partial search results"
                } else if indexing {
                    "Searching…"
                } else {
                    "No matching files"
                });
                state.items.replace(items);
            }
            glib::ControlFlow::Continue
        });
    });
    stack.upcast()
}

fn relative_result_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests;

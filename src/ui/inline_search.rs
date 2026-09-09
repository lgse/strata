// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{glib, prelude::*};

use crate::{
    app::Browser,
    services::{SearchEvent, SearchHandle, SearchItem, index_filter},
};

pub(super) const SEARCH_RESULTS_LABEL: &str = "Search results";

struct State {
    entry: glib::WeakRef<gtk::Entry>,
    stack: gtk::Stack,
    list: gtk::ListBox,
    status: gtk::Label,
    items: RefCell<Vec<SearchItem>>,
    positions: RefCell<HashMap<gtk::ListBoxRow, usize>>,
    handle: RefCell<Option<SearchHandle>>,
    generation: Cell<u64>,
}

#[derive(Clone)]
pub(super) struct InlineSearch {
    pub widget: gtk::Widget,
    state: Option<Rc<State>>,
}

impl InlineSearch {
    pub fn is_item_target(&self, picked: &gtk::Widget) -> bool {
        self.state
            .as_ref()
            .is_some_and(|state| result_at_widget(state, picked).is_some())
    }

    pub fn install_context_menu(&self, view: &Rc<super::browser::ViewState>, depth: usize) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        let weak = Rc::downgrade(state);
        let resolve = Rc::new(move |picked: &gtk::Widget| {
            let state = weak.upgrade()?;
            let row = result_at_widget(&state, picked)?;
            let entry = state
                .items
                .borrow()
                .get(row.index() as usize)
                .map(super::browser::search_result_entry)?;
            state.list.select_row(Some(&row));
            Some((None, entry))
        });
        super::browser::install_resolved_item_context_menu(
            view,
            state.list.upcast_ref(),
            resolve,
            depth,
        );
    }

    pub fn selected_entry(&self) -> Option<crate::model::FileEntry> {
        let state = self.state.as_ref()?;
        let focused = self.widget.root()?.focus()?;
        let entry = state.entry.upgrade()?;
        if state.stack.visible_child_name().as_deref() != Some("search")
            || !(focused.is_ancestor(&entry)
                || focused == entry.upcast::<gtk::Widget>()
                || focused.is_ancestor(&state.list)
                || focused == state.list.clone().upcast::<gtk::Widget>())
        {
            return None;
        }
        let row = state.list.selected_row()?;
        state
            .items
            .borrow()
            .get(row.index() as usize)
            .map(super::browser::search_result_entry)
    }
}

/// Keeps the view's normal presentation intact when the recursive query is dismissed.
pub(super) fn wrap(
    content: &impl IsA<gtk::Widget>,
    entry: &gtk::Entry,
    root: Option<PathBuf>,
    browser: &Rc<Browser>,
) -> InlineSearch {
    let Some(root) = root else {
        return InlineSearch {
            widget: content.clone().upcast(),
            state: None,
        };
    };
    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    stack.add_named(content, Some("files"));
    let results = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let status = gtk::Label::new(None);
    status.add_css_class("status-message");
    results.append(&status);
    let list = gtk::ListBox::new();
    list.add_css_class("file-list");
    super::accessibility::set_label(&list, SEARCH_RESULTS_LABEL);
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
        positions: RefCell::new(HashMap::new()),
        handle: RefCell::new(None),
        generation: Cell::new(0),
    });
    let weak = Rc::downgrade(&state);
    state.list.set_sort_func(move |left, right| {
        let Some(state) = weak.upgrade() else {
            return gtk::Ordering::Equal;
        };
        let positions = state.positions.borrow();
        positions.get(left).cmp(&positions.get(right)).into()
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
    let search = InlineSearch {
        widget: stack.clone().upcast(),
        state: Some(state.clone()),
    };
    super::browser::bind_filter_query(entry, move |text, recursive, restart| {
        if restart {
            state.generation.set(state.generation.get().wrapping_add(1));
            state.handle.borrow_mut().take();
        }
        let query = text.trim();
        if query.is_empty() {
            state.generation.set(state.generation.get().wrapping_add(1));
            state.handle.borrow_mut().take();
            state.items.borrow_mut().clear();
            state.positions.borrow_mut().clear();
            clear_rows(&state.list);
            state.stack.set_visible_child_name("files");
            return;
        }
        state.stack.set_visible_child_name("search");
        if state.items.borrow().is_empty() {
            state.status.set_text("Searching…");
            state.status.set_visible(true);
        }
        if let Some(handle) = state.handle.borrow().as_ref() {
            handle.query(query);
            return;
        }
        let generation = state.generation.get();
        let show_hidden = weak_browser
            .upgrade()
            .is_some_and(|browser| browser.preferences().show_hidden);
        let (handle, receiver) = index_filter(root.clone(), show_hidden, recursive);
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
                coverage,
            }) = latest
                && !returned.is_empty()
                && returned == entry.text().trim()
            {
                state
                    .status
                    .set_visible(items.is_empty() || coverage.is_partial());
                state.status.set_text(&if coverage.is_partial() {
                    coverage.message()
                } else if indexing {
                    "Searching…".to_owned()
                } else {
                    "No matching files".to_owned()
                });
                update_rows(&state, items, &result_root, recursive);
            }
            glib::ControlFlow::Continue
        });
    });
    search
}

fn result_at_widget(state: &State, picked: &gtk::Widget) -> Option<gtk::ListBoxRow> {
    let mut current = Some(picked.clone());
    while let Some(widget) = current {
        if let Some(row) = widget.downcast_ref::<gtk::ListBoxRow>()
            && row.parent().as_ref() == Some(state.list.upcast_ref())
        {
            return Some(row.clone());
        }
        current = widget.parent();
    }
    None
}

fn update_rows(state: &State, items: Vec<SearchItem>, root: &Path, recursive: bool) {
    let selected = state.list.selected_row().map(|row| row.index() as usize);
    let focused = state.list.root().and_then(|root| root.focus());
    let old = state.items.replace(items);
    let selected_path = selected
        .and_then(|index| old.get(index))
        .map(|item| &item.path);
    let mut retained: HashMap<_, _> = old
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            Some((
                item.path.clone(),
                (item, state.list.row_at_index(index as i32)?),
            ))
        })
        .collect();
    let items = state.items.borrow();
    let mut rows = Vec::with_capacity(items.len());
    for item in items.iter() {
        let row = if let Some((previous, row)) = retained.remove(&item.path) {
            if previous.name == item.name && previous.is_directory == item.is_directory {
                if let Some(origin) = row
                    .child()
                    .and_then(|line| line.last_child())
                    .and_then(|labels| labels.last_child())
                {
                    origin.set_visible(recursive);
                }
                row
            } else {
                super::thumbnail::cancel_thumbnails_in(row.upcast_ref());
                state.list.remove(&row);
                result_row(&state.list, item, root, recursive)
            }
        } else {
            result_row(&state.list, item, root, recursive)
        };
        rows.push(row);
    }
    for (_, row) in retained.into_values() {
        super::thumbnail::cancel_thumbnails_in(row.upcast_ref());
        state.list.remove(&row);
    }
    state.positions.replace(
        rows.iter()
            .enumerate()
            .map(|(index, row)| (row.clone(), index))
            .collect(),
    );
    // GTK sorts in place, keeping rows rooted, selected, and their thumbnail work alive.
    state.list.invalidate_sort();
    let next = selected_path
        .and_then(|path| items.iter().position(|item| &item.path == path))
        .or_else(|| {
            selected
                .filter(|_| !items.is_empty())
                .map(|index| index.min(items.len() - 1))
        });
    state.list.select_row(
        next.and_then(|index| state.list.row_at_index(index as i32))
            .as_ref(),
    );
    if let Some(focused) = focused {
        if focused.root().is_some() {
            if state.list.root().and_then(|root| root.focus()).as_ref() != Some(&focused) {
                focused.grab_focus();
            }
        } else if let Some(entry) = state.entry.upgrade() {
            entry.grab_focus();
        }
    }
}

fn result_row(
    list: &gtk::ListBox,
    item: &SearchItem,
    result_root: &Path,
    recursive: bool,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    // Keep keyboard focus in the query, away from file-operation shortcuts.
    row.set_focusable(false);
    super::accessibility::set_label(&row, &item.name);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.add_css_class("file-row");
    let icon = super::thumbnail::ThumbnailSlot::new(17);
    line.append(&icon);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let name = gtk::Label::builder()
        .label(&item.name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let path = relative_result_path(result_root, &item.path);
    let origin = gtk::Label::builder()
        .label(&path)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    origin.add_css_class("file-search-path");
    origin.set_visible(recursive);
    labels.append(&name);
    labels.append(&origin);
    row.set_tooltip_text(Some(&path));
    line.append(&labels);
    row.set_child(Some(&line));
    // Thumbnail scheduling resolves the owning viewport, so attach the row first.
    list.append(&row);
    if item.is_directory {
        super::thumbnail::show_customized_icon(&icon, &item.path, crate::assets::icons::FOLDER, 17);
    } else {
        super::thumbnail::set_thumbnail_or_icon_for_path(
            &icon,
            &item.path,
            crate::assets::icons::DOCUMENTS,
            17,
            17,
        );
    }
    row
}

fn clear_rows(list: &gtk::ListBox) {
    super::thumbnail::cancel_thumbnails_in(list.upcast_ref());
    list.remove_all();
}

fn relative_result_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests;

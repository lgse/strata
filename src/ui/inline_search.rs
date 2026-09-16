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
    selection: gtk::MultiSelection,
    selection_rows: gtk::gio::ListStore,
    syncing_selection: Cell<bool>,
    anchor: glib::WeakRef<gtk::ListBoxRow>,
    items: RefCell<Vec<SearchItem>>,
    positions: RefCell<HashMap<gtk::ListBoxRow, usize>>,
    handle: RefCell<Option<SearchHandle>>,
    generation: Cell<u64>,
    root: PathBuf,
    recursive: Cell<bool>,
    context_menu_trigger: RefCell<Option<super::browser::ContextMenuTrigger>>,
}

impl State {
    fn current_row(&self) -> Option<gtk::ListBoxRow> {
        self.list
            .root()
            .and_then(|root| root.focus())
            .and_then(|focus| result_at_widget(self, &focus))
            .or_else(|| self.list.selected_rows().into_iter().next())
    }

    fn sync_selection(&self) {
        if self.syncing_selection.replace(true) {
            return;
        }
        let selected = gtk::Bitset::new_empty();
        for row in self.list.selected_rows() {
            selected.add(row.index() as u32);
        }
        self.selection.set_selection(
            &selected,
            &gtk::Bitset::new_range(0, self.selection.n_items()),
        );
        self.syncing_selection.set(false);
    }
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
            if !row.is_selected() {
                state.list.unselect_all();
                state.list.select_row(Some(&row));
            }
            Some((None, entry))
        });
        let trigger = super::browser::install_resolved_item_context_menu(
            view,
            state.list.upcast_ref(),
            resolve,
            depth,
        );
        state.context_menu_trigger.replace(Some(trigger));
    }

    pub fn context_menu_target(&self) -> Option<super::browser::ContextMenuTarget> {
        let state = self.state.as_ref()?;
        let row = state.current_row()?;
        let bounds = row.compute_bounds(&state.list)?;
        Some((
            state.context_menu_trigger.borrow().as_ref()?.clone(),
            f64::from(bounds.center().x()),
            f64::from(bounds.center().y()),
        ))
    }

    pub fn selected_entry(&self) -> Option<crate::model::FileEntry> {
        let state = self.state.as_ref()?;
        let focused = self.widget.root()?.focus()?;
        let entry = state.entry.upgrade()?;
        if !(focused.is_ancestor(&entry)
            || focused == entry.upcast::<gtk::Widget>()
            || focused.is_ancestor(&state.list)
            || focused == state.list.clone().upcast::<gtk::Widget>())
        {
            return None;
        }
        self.selected_entries()?.into_iter().next()
    }

    pub fn selected_entries(&self) -> Option<Vec<crate::model::FileEntry>> {
        let state = self.state.as_ref()?;
        if state.stack.visible_child_name().as_deref() != Some("search") {
            return None;
        }
        let entries = state
            .list
            .selected_rows()
            .into_iter()
            .filter_map(|row| {
                state
                    .items
                    .borrow()
                    .get(row.index() as usize)
                    .map(super::browser::search_result_entry)
            })
            .collect();
        Some(entries)
    }

    pub fn select_all(&self) -> bool {
        let Some(state) = self
            .state
            .as_ref()
            .filter(|state| state.stack.visible_child_name().as_deref() == Some("search"))
        else {
            return false;
        };
        state.list.select_all();
        true
    }

    pub fn focus_result(&self, path: &Path) -> bool {
        let Some(state) = self.state.as_ref() else {
            return false;
        };
        if state.stack.visible_child_name().as_deref() != Some("search") {
            return false;
        }
        let position = state
            .items
            .borrow()
            .iter()
            .position(|item| item.path == path);
        let Some(row) = position.and_then(|position| state.list.row_at_index(position as i32))
        else {
            return false;
        };
        if !row.is_selected() {
            state.list.unselect_all();
            state.list.select_row(Some(&row));
        }
        row.set_focusable(true);
        row.grab_focus()
    }

    pub fn refresh_cut_rows(&self) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        for (position, item) in state.items.borrow().iter().enumerate() {
            if let Some(line) = state
                .list
                .row_at_index(position as i32)
                .and_then(|row| row.child())
                .and_downcast::<gtk::Box>()
            {
                super::browser::set_cut_result_style(
                    &line,
                    &crate::model::Location::local(&item.path),
                );
            }
        }
    }

    pub fn prune_missing(&self) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        if state.handle.borrow().is_none() {
            return;
        }
        let pruned: Vec<_> = state
            .items
            .borrow()
            .iter()
            .filter(|item| search_path_present(&item.path))
            .cloned()
            .collect();
        if pruned.len() != state.items.borrow().len() {
            let recursive = state.recursive.get();
            update_rows(state, pruned, &state.root, recursive);
        }
    }

    pub fn show_directory_listing(&self) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        show_directory_listing(state);
    }
}

fn show_directory_listing(state: &State) {
    state.generation.set(state.generation.get().wrapping_add(1));
    state.handle.borrow_mut().take();
    state.items.borrow_mut().clear();
    state.positions.borrow_mut().clear();
    state.selection_rows.remove_all();
    state.anchor.set(None);
    clear_rows(&state.list);
    state.stack.set_visible_child_name("files");
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
    list.set_selection_mode(gtk::SelectionMode::Multiple);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroll.add_css_class("fixed-scrollbar");
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    results.append(&overlay);
    stack.add_named(&results, Some("search"));
    let selection_rows = gtk::gio::ListStore::new::<gtk::ListBoxRow>();
    let selection = gtk::MultiSelection::new(Some(selection_rows.clone()));
    let state = Rc::new(State {
        entry: entry.downgrade(),
        stack: stack.clone(),
        list,
        status,
        selection,
        selection_rows,
        syncing_selection: Cell::new(false),
        anchor: glib::WeakRef::new(),
        items: RefCell::new(Vec::new()),
        positions: RefCell::new(HashMap::new()),
        handle: RefCell::new(None),
        generation: Cell::new(0),
        root: root.clone(),
        recursive: Cell::new(false),
        context_menu_trigger: RefCell::new(None),
    });
    install_selection(&state, &scroll, &overlay);
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
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&state);
    click.connect_pressed(move |gesture, _press_count, _x, y| {
        let Some(state) = weak.upgrade() else { return };
        let Some(row) = state.list.row_at_y(y as i32) else {
            return;
        };
        let modifiers = gesture.current_event_state();
        if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
            let anchor = state.anchor.upgrade().unwrap_or_else(|| row.clone());
            if !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                state.list.unselect_all();
            }
            for index in anchor.index().min(row.index())..=anchor.index().max(row.index()) {
                if let Some(row) = state.list.row_at_index(index) {
                    state.list.select_row(Some(&row));
                }
            }
        } else if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            state.anchor.set(Some(&row));
            if row.is_selected() {
                state.list.unselect_row(&row);
            } else {
                state.list.select_row(Some(&row));
            }
        } else {
            state.anchor.set(Some(&row));
            if !row.is_selected() {
                state.list.unselect_all();
                state.list.select_row(Some(&row));
            }
        }
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        {
            row.grab_focus();
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    let weak = Rc::downgrade(&state);
    let weak_browser = Rc::downgrade(browser);
    click.connect_released(move |gesture, press_count, _x, y| {
        if press_count != 1
            || gesture.current_event_state().intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK,
            )
        {
            return;
        }
        let Some(state) = weak.upgrade() else { return };
        let Some(row) = state.list.row_at_y(y as i32) else {
            return;
        };
        if state.list.selected_rows().len() > 1 {
            return;
        }
        let Some(browser) = weak_browser.upgrade() else {
            return;
        };
        let Some(item) = state.items.borrow().get(row.index() as usize).cloned() else {
            return;
        };
        if browser.is_chooser_mode() {
            if item.is_directory {
                browser.navigate(crate::model::Location::local(item.path));
            }
        } else {
            super::browser::activate_recursive_search_result(
                &Rc::downgrade(&browser),
                &state.items,
                row.index() as u32,
            );
        }
    });
    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let weak = Rc::downgrade(&state);
    drag.connect_prepare(move |source, _x, y| {
        let state = weak.upgrade()?;
        let row = state.list.row_at_y(y as i32)?;
        let items = state.items.borrow();
        let entries: Vec<_> = if row.is_selected() {
            state.list.selected_rows()
        } else {
            vec![row.clone()]
        }
        .into_iter()
        .filter_map(|row| items.get(row.index() as usize))
        .map(super::browser::search_result_entry)
        .collect();
        source.set_actions(super::browser::drag_actions_for_modifiers(
            source.current_event_state(),
        ));
        let paintable = gtk::WidgetPaintable::new(Some(&row));
        source.set_icon(Some(&paintable), 0, 0);
        super::browser::file_drag_content(&entries)
    });
    state.list.add_controller(drag.clone());
    state.list.add_controller(click.clone());
    drag.group_with(&click);
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
                    | gtk::gdk::ModifierType::SUPER_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK,
            )
        {
            return glib::Propagation::Proceed;
        }
        let current = state.current_row().map(|row| row.index() as u32);
        if key == gtk::gdk::Key::Up {
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Down {
            if let Some(row) = state.list.row_at_index(current.unwrap_or(0) as i32) {
                state.list.unselect_all();
                state.list.select_row(Some(&row));
                if let Some(window) = state.list.root().and_downcast::<gtk::Window>() {
                    window.set_focus_visible(true);
                }
                row.grab_focus();
            }
            return glib::Propagation::Stop;
        }
        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
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
    let return_to_filter = gtk::EventControllerKey::new();
    return_to_filter.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&state);
    return_to_filter.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if key != gtk::gdk::Key::Up
            || modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK,
            )
            || !state.current_row().is_some_and(|row| row.index() == 0)
            || state
                .list
                .root()
                .and_then(|root| root.focus())
                .and_then(|focus| focus.ancestor(gtk::Popover::static_type()))
                .is_some()
        {
            return glib::Propagation::Proceed;
        }
        state
            .entry
            .upgrade()
            .filter(|entry| entry.grab_focus_without_selecting())
            .map_or(glib::Propagation::Proceed, |_| glib::Propagation::Stop)
    });
    state.list.add_controller(return_to_filter);
    let weak_browser = Rc::downgrade(browser);
    let search = InlineSearch {
        widget: stack.clone().upcast(),
        state: Some(state.clone()),
    };
    super::browser::bind_filter_query(entry, move |text, recursive, restart| {
        state.recursive.set(recursive);
        if restart {
            state.generation.set(state.generation.get().wrapping_add(1));
            state.handle.borrow_mut().take();
        }
        let query = text.trim();
        if query.is_empty() {
            show_directory_listing(&state);
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
                mut items,
                indexing,
                coverage,
            }) = latest
                && !returned.is_empty()
                && returned == entry.text().trim()
            {
                items.retain(|item| search_path_present(&item.path));
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

fn install_selection(state: &Rc<State>, scroll: &gtk::ScrolledWindow, overlay: &gtk::Overlay) {
    let weak = Rc::downgrade(state);
    state.list.connect_selected_rows_changed(move |_| {
        if let Some(state) = weak.upgrade() {
            state.sync_selection();
        }
    });
    let weak = Rc::downgrade(state);
    state
        .selection
        .connect_selection_changed(move |selection, _, _| {
            let Some(state) = weak.upgrade() else { return };
            if state.syncing_selection.replace(true) {
                return;
            }
            state.list.unselect_all();
            for position in 0..selection.n_items() {
                if selection.is_selected(position)
                    && let Some(row) = state.list.row_at_index(position as i32)
                {
                    state.list.select_row(Some(&row));
                }
            }
            state.syncing_selection.set(false);
        });
    let weak = Rc::downgrade(state);
    let targets = Rc::new(RefCell::new(vec![super::marquee::MarqueeTarget {
        selection: state.selection.clone(),
        visit_items: Rc::new(move |visit| {
            let Some(state) = weak.upgrade() else { return };
            for position in 0..state.selection.n_items() {
                if let Some(row) = state.list.row_at_index(position as i32) {
                    visit(position, row.upcast_ref());
                }
            }
        }),
    }]));
    let weak = Rc::downgrade(state);
    super::marquee::install(super::marquee::MarqueeSetup {
        view: state.list.clone().upcast(),
        surface: scroll.clone().upcast(),
        scroll: scroll.clone(),
        overlay: overlay.clone(),
        targets: targets.clone(),
        is_item: super::marquee::item_bounds_predicate(targets),
        clear_selection: Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                state.list.unselect_all();
            }
        }),
    });
}

pub(super) fn search_path_present(path: &Path) -> bool {
    // Preserve dangling symlinks and uncertain paths; only confirmed absence removes a hit.
    path.symlink_metadata().map_or_else(
        |error| error.kind() != std::io::ErrorKind::NotFound,
        |_| true,
    )
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
    let selected: Vec<_> = state
        .list
        .selected_rows()
        .into_iter()
        .map(|row| row.index() as usize)
        .collect();
    let focused = state.list.root().and_then(|root| root.focus());
    let old = state.items.replace(items);
    let selected_paths: Vec<_> = selected
        .iter()
        .filter_map(|index| old.get(*index))
        .map(|item| &item.path)
        .collect();
    state.syncing_selection.set(true);
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
    state
        .selection_rows
        .splice(0, state.selection_rows.n_items(), &rows);
    state.list.unselect_all();
    for (index, item) in items.iter().enumerate() {
        if selected_paths.contains(&&item.path) {
            state
                .list
                .select_row(state.list.row_at_index(index as i32).as_ref());
        }
    }
    if state.list.selected_rows().is_empty()
        && let Some(index) = selected.first().filter(|_| !items.is_empty())
    {
        let index = (*index).min(items.len() - 1);
        state
            .list
            .select_row(state.list.row_at_index(index as i32).as_ref());
    }
    state.syncing_selection.set(false);
    state.sync_selection();
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
    row.set_focusable(true);
    super::accessibility::set_label(&row, &item.name);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.add_css_class("file-row");
    line.add_css_class("filter-result");
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
    super::browser::set_cut_result_style(&line, &crate::model::Location::local(&item.path));
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

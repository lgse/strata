// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::mpsc::TryRecvError,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::services::{SearchCoverage, SearchEvent, SearchHandle, SearchItem, index_trees};

const MAX_RESULT_UPDATES_PER_FRAME: usize = 8;

#[derive(Clone)]
pub struct SearchDialog {
    state: Rc<SearchState>,
}

struct SearchState {
    layer: gtk::Box,
    field: gtk::Entry,
    indexing_spinner: gtk::Spinner,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    results: gtk::Stack,
    status: gtk::Label,
    truncated_hint: gtk::Label,
    visible_results: RefCell<Vec<SearchItem>>,
    positions: Rc<RefCell<HashMap<gtk::ListBoxRow, usize>>>,
    requested_thumbnails: RefCell<HashSet<PathBuf>>,
    rendered_query: RefCell<String>,
    search: RefCell<Option<SearchHandle>>,
    generation: Cell<u64>,
    interaction_revision: Cell<u64>,
    navigation_started: Cell<bool>,
    reconciling_results: Cell<bool>,
    activate: Rc<dyn Fn(SearchItem)>,
    dismiss: Rc<dyn Fn()>,
}

impl SearchDialog {
    #[expect(
        deprecated,
        reason = "GTK 4.12 deprecated translate_coordinates and allocation without a replacement for click-in-bounds checks"
    )]
    pub fn new(activate: Rc<dyn Fn(SearchItem)>, dismiss: Rc<dyn Fn()>) -> Self {
        let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layer.add_css_class("search-backdrop");
        layer.add_css_class("app-modal-layer");
        layer.set_halign(gtk::Align::Fill);
        layer.set_valign(gtk::Align::Fill);
        layer.set_hexpand(true);
        layer.set_vexpand(true);
        layer.set_focusable(true);
        layer.set_visible(false);

        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("search-dialog");
        panel.set_halign(gtk::Align::Center);
        panel.set_valign(gtk::Align::Center);
        panel.set_size_request(760, 452);
        panel.set_vexpand(false);
        panel.set_overflow(gtk::Overflow::Hidden);

        let search_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        search_bar.add_css_class("search-bar");
        search_bar.append(&crate::assets::primary_icon(
            crate::assets::icons::SEARCH,
            20,
        ));
        let field = gtk::Entry::builder()
            .placeholder_text("Search files and folders…")
            .hexpand(true)
            .build();
        field.add_css_class("search-field");
        search_bar.append(&field);
        let indexing_spinner = gtk::Spinner::new();
        indexing_spinner.add_css_class("search-indexing-spinner");
        indexing_spinner.set_tooltip_text(Some("Indexing files…"));
        indexing_spinner.set_valign(gtk::Align::Center);
        indexing_spinner.set_visible(false);
        search_bar.append(&indexing_spinner);
        panel.append(&search_bar);

        let status = gtk::Label::new(Some("Type to search Home and mounted local drives"));
        status.add_css_class("search-status");
        status.set_wrap(true);

        let list = gtk::ListBox::new();
        list.add_css_class("search-results");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(true);
        let positions = Rc::new(RefCell::new(HashMap::new()));
        let sorted_positions = positions.clone();
        list.set_sort_func(move |left, right| {
            let positions = sorted_positions.borrow();
            positions.get(left).cmp(&positions.get(right)).into()
        });
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&list)
            .build();
        scroller.add_css_class("search-results-scroll");
        let results = gtk::Stack::new();
        results.add_css_class("search-results-stack");
        results.set_size_request(-1, 360);
        results.add_named(&status, Some("status"));
        results.add_named(&scroller, Some("results"));
        results.set_visible_child_name("status");
        panel.append(&results);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        footer.add_css_class("search-footer");
        let navigation =
            gtk::Label::new(Some("↑↓  navigate · j/k in results · Ctrl+K  edit query"));
        let open = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        open.set_valign(gtk::Align::Center);
        open.append(&crate::assets::primary_icon(
            crate::assets::icons::CORNER_DOWN_LEFT,
            13,
        ));
        let enter_hint = gtk::Label::new(Some("navigate results"));
        open.append(&enter_hint);
        let focus = gtk::EventControllerFocus::new();
        let hint = enter_hint.clone();
        let editing_results = list.clone();
        focus.connect_enter(move |_| {
            hint.set_text("navigate results");
            editing_results.add_css_class("query-editing");
        });
        let navigating_results = list.clone();
        focus.connect_leave(move |_| {
            enter_hint.set_text("open");
            navigating_results.remove_css_class("query-editing");
        });
        field.add_controller(focus);
        navigation.add_css_class("search-hint");
        open.add_css_class("search-hint");
        footer.append(&navigation);
        footer.append(&open);
        let truncated_hint = gtk::Label::new(None);
        truncated_hint.set_wrap(true);
        truncated_hint.set_max_width_chars(58);
        truncated_hint.add_css_class("search-hint");
        truncated_hint.add_css_class("search-hint-warning");
        truncated_hint.set_hexpand(true);
        truncated_hint.set_halign(gtk::Align::End);
        truncated_hint.set_visible(false);
        footer.append(&truncated_hint);
        panel.append(&footer);
        let top_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        top_spacer.set_vexpand(true);
        let bottom_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom_spacer.set_vexpand(true);
        let left_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        left_spacer.set_hexpand(true);
        let right_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        right_spacer.set_hexpand(true);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.append(&left_spacer);
        row.append(&panel);
        row.append(&right_spacer);
        layer.append(&top_spacer);
        layer.append(&row);
        layer.append(&bottom_spacer);

        let state = Rc::new(SearchState {
            layer,
            field,
            indexing_spinner,
            list,
            scroller,
            results,
            status,
            truncated_hint,
            visible_results: RefCell::new(Vec::new()),
            positions,
            requested_thumbnails: RefCell::new(HashSet::new()),
            rendered_query: RefCell::new(String::new()),
            search: RefCell::new(None),
            generation: Cell::new(0),
            interaction_revision: Cell::new(0),
            navigation_started: Cell::new(false),
            reconciling_results: Cell::new(false),
            activate,
            dismiss,
        });

        let changed = Rc::downgrade(&state);
        state.field.connect_changed(move |field| {
            if let Some(state) = changed.upgrade() {
                begin_query(&state, &field.text());
            }
        });
        let activated = Rc::downgrade(&state);
        state.list.connect_row_activated(move |_, row| {
            if let Some(state) = activated.upgrade() {
                activate_position(&state, row.index());
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let keyed = Rc::downgrade(&state);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(state) = keyed.upgrade() else {
                return glib::Propagation::Proceed;
            };
            record_interaction(&state);
            if key == gdk::Key::Escape {
                hide(&state);
                return glib::Propagation::Stop;
            }
            if modifiers == gdk::ModifierType::CONTROL_MASK
                && matches!(key, gdk::Key::k | gdk::Key::K)
            {
                state.field.grab_focus_without_selecting();
                return glib::Propagation::Stop;
            }
            if modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            let editing = contains_keyboard_focus(state.field.upcast_ref());
            if (matches!(key, gdk::Key::Down | gdk::Key::Up)
                || (!editing && matches!(key, gdk::Key::j | gdk::Key::k)))
                && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
            {
                move_selection(
                    &state,
                    if matches!(key, gdk::Key::Down | gdk::Key::j) {
                        1
                    } else {
                        -1
                    },
                );
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
                if editing {
                    focus_results(&state);
                } else {
                    activate_selected(&state);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        state.layer.add_controller(keys);

        let click_state = Rc::downgrade(&state);
        let click_panel = panel.clone();
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(move |_, _, x, y| {
            let Some(state) = click_state.upgrade() else {
                return;
            };
            record_interaction(&state);
            let on_panel = click_panel
                .translate_coordinates(&state.layer, 0.0, 0.0)
                .is_some_and(|(px, py)| {
                    let alloc = click_panel.allocation();
                    x >= px
                        && x < px + alloc.width() as f64
                        && y >= py
                        && y < py + alloc.height() as f64
                });
            if !on_panel {
                hide(&state);
            }
        });
        state.layer.add_controller(click);
        let wheel_state = Rc::downgrade(&state);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        wheel.connect_scroll(move |_, _, _| {
            if let Some(state) = wheel_state.upgrade() {
                record_interaction(&state);
            }
            glib::Propagation::Proceed
        });
        state.layer.add_controller(wheel);
        let adjustment = state.scroller.vadjustment();
        let changed = Rc::downgrade(&state);
        adjustment.connect_changed(move |_| {
            if let Some(state) = changed.upgrade()
                && !state.reconciling_results.get()
            {
                refresh_visible_thumbnails(&state);
            }
        });
        let scrolled = Rc::downgrade(&state);
        adjustment.connect_value_changed(move |_| {
            if let Some(state) = scrolled.upgrade()
                && !state.reconciling_results.get()
            {
                refresh_visible_thumbnails(&state);
            }
        });

        Self { state }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.layer.clone().upcast()
    }

    pub fn show(&self, roots: Vec<PathBuf>, show_hidden: bool) {
        self.state.generation.set(self.state.generation.get() + 1);
        let generation = self.state.generation.get();
        self.state.search.borrow_mut().take();
        let locations = roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        self.state.field.set_tooltip_text(Some(&format!(
            "Search locations:\n{locations}\nRemote shares are not included."
        )));
        self.state.field.set_sensitive(!roots.is_empty());
        clear_results(&self.state);
        self.state.results.set_visible_child_name("status");
        self.state.field.set_text("");
        self.state.status.set_visible(true);
        self.state
            .status
            .set_text("Type to search Home and mounted local drives");
        self.state.truncated_hint.set_visible(false);
        self.state.indexing_spinner.set_visible(true);
        self.state.indexing_spinner.start();
        self.state.layer.set_visible(true);
        super::browser::animate_in(&self.state.layer);
        self.state.field.grab_focus_without_selecting();

        if roots.is_empty() {
            self.state
                .status
                .set_text("No local search locations available.");
            self.state.indexing_spinner.stop();
            self.state.indexing_spinner.set_visible(false);
            self.state.layer.grab_focus();
            return;
        }
        let (handle, receiver) = index_trees(roots, show_hidden);
        self.state.search.replace(Some(handle));
        let weak = Rc::downgrade(&self.state);
        let _poll = glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !state.layer.is_visible() || state.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let mut latest = None;
            for _ in 0..MAX_RESULT_UPDATES_PER_FRAME {
                match receiver.try_recv() {
                    Ok(event) => latest = Some(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return glib::ControlFlow::Break,
                }
            }
            if let Some(SearchEvent::Results {
                query,
                items,
                indexing,
                coverage,
            }) = latest
            {
                if indexing {
                    state.indexing_spinner.set_visible(true);
                    state.indexing_spinner.start();
                } else {
                    state.indexing_spinner.stop();
                    state.indexing_spinner.set_visible(false);
                }
                if query == state.field.text().trim() {
                    state.truncated_hint.set_text(&coverage.message());
                    state.truncated_hint.set_visible(coverage.is_partial());
                }
                if !query.is_empty() && query == state.field.text().trim() {
                    render_results(&state, items, indexing, coverage);
                }
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn focus_query(&self) {
        self.state.field.grab_focus_without_selecting();
    }

    pub fn hide(&self) {
        hide(&self.state);
    }

    pub fn is_visible(&self) -> bool {
        self.state.layer.is_visible()
    }
}

fn begin_query(state: &Rc<SearchState>, query: &str) {
    record_interaction(state);
    state.navigation_started.set(false);
    if query.trim().is_empty() {
        clear_results(state);
        state.results.set_visible_child_name("status");
        state.status.set_text(
            "Type to search Home and mounted local drives\nFuzzy matching · try a name or path fragment",
        );
    } else if state.visible_results.borrow().is_empty() {
        state.results.set_visible_child_name("status");
        state.status.set_text("Searching…");
    }
    if let Some(search) = state.search.borrow().as_ref() {
        search.query(query);
    }
}

fn render_results(
    state: &Rc<SearchState>,
    results: Vec<SearchItem>,
    indexing: bool,
    coverage: SearchCoverage,
) {
    let query = state.field.text().trim().to_owned();
    let query_changed = state.rendered_query.borrow().as_str() != query;
    let old_items = state.visible_results.borrow().clone();
    let results_changed = old_items != results;
    let selected_index = state.list.selected_row().map(|row| row.index());
    let selected_path = selected_index
        .and_then(|position| usize::try_from(position).ok())
        .and_then(|position| old_items.get(position))
        .map(|item| item.path.clone());
    let focused = state.list.root().and_then(|root| root.focus());
    let focused_path = old_items.iter().enumerate().find_map(|(position, item)| {
        let row = state.list.row_at_index(position as i32)?;
        focused
            .as_ref()
            .filter(|focused| **focused == row || focused.is_ancestor(&row))
            .map(|_| item.path.clone())
    });
    let waiting_for_results = state.layer.has_focus();
    let had_result_focus = focused_path.is_some();
    let scroll_position = state.scroller.vadjustment().value();

    if results_changed {
        state.reconciling_results.set(true);
        let mut rows = old_items
            .iter()
            .enumerate()
            .filter_map(|(position, item)| {
                state
                    .list
                    .row_at_index(position as i32)
                    .map(|row| (item.path.clone(), (item.clone(), row)))
            })
            .collect::<HashMap<_, _>>();
        let mut ordered_rows = Vec::with_capacity(results.len());
        let mut requested = state.requested_thumbnails.borrow_mut();

        for item in &results {
            let row = if let Some((previous, row)) = rows.remove(&item.path) {
                if previous == *item {
                    row
                } else {
                    requested.remove(&item.path);
                    super::thumbnail::cancel_thumbnails_in(row.upcast_ref());
                    state.list.remove(&row);
                    let row = result_row(item);
                    state.list.append(&row);
                    row
                }
            } else {
                let row = result_row(item);
                state.list.append(&row);
                row
            };
            ordered_rows.push(row);
        }
        for (path, (_, row)) in rows {
            requested.remove(&path);
            super::thumbnail::cancel_thumbnails_in(row.upcast_ref());
            state.list.remove(&row);
        }
        drop(requested);

        state.positions.replace(
            ordered_rows
                .into_iter()
                .enumerate()
                .map(|(position, row)| (row, position))
                .collect(),
        );
        state.visible_results.replace(results);
        state.list.invalidate_sort();
        state.reconciling_results.set(false);
    }

    state.rendered_query.replace(query);
    let has_results = !state.visible_results.borrow().is_empty();
    state.truncated_hint.set_text(&coverage.message());
    state.truncated_hint.set_visible(coverage.is_partial());
    state
        .results
        .set_visible_child_name(if has_results { "results" } else { "status" });
    if has_results && (results_changed || query_changed) {
        let items = state.visible_results.borrow();
        let restored = if query_changed {
            0
        } else {
            selected_path
                .as_ref()
                .and_then(|path| items.iter().position(|item| &item.path == path))
                .or_else(|| {
                    selected_index.map(|position| {
                        usize::try_from(position)
                            .unwrap_or_default()
                            .min(items.len() - 1)
                    })
                })
                .unwrap_or(0)
        };
        drop(items);
        state
            .list
            .select_row(state.list.row_at_index(restored as i32).as_ref());
    } else if !has_results {
        state.status.set_text(if indexing {
            "Searching…"
        } else {
            "No matching files or folders"
        });
    }

    if results_changed
        && had_result_focus
        && focused
            .as_ref()
            .is_some_and(|focused| focused.root().is_none())
    {
        let focused_position = focused_path.and_then(|path| {
            state
                .visible_results
                .borrow()
                .iter()
                .position(|item| item.path == path)
        });
        if let Some(row) = focused_position
            .and_then(|position| state.list.row_at_index(position as i32))
            .or_else(|| state.list.selected_row())
        {
            row.grab_focus();
        } else {
            state.layer.grab_focus();
        }
    }
    if waiting_for_results && has_results {
        focus_results(state);
    }

    if results_changed {
        record_interaction(state);
        let revision = state.interaction_revision.get();
        let weak = Rc::downgrade(state);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                if state.layer.is_visible() && state.interaction_revision.get() == revision {
                    if state.navigation_started.get() {
                        if let Some(row) = state.list.selected_row() {
                            scroll_row_into_view(&state, &row);
                        }
                    } else {
                        state.scroller.vadjustment().set_value(scroll_position);
                    }
                }
                refresh_visible_thumbnails(&state);
            }
        });
    }
}

fn result_row(item: &SearchItem) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("search-result");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = super::thumbnail::ThumbnailSlot::new(19);
    icon.add_css_class("search-result-thumbnail");
    let fallback = if item.is_directory {
        crate::assets::icons::FOLDER
    } else {
        crate::assets::icons::DOCUMENTS
    };
    super::thumbnail::show_customized_icon(&icon, &item.path, fallback, 19);
    content.append(&icon);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let name = gtk::Label::new(Some(&item.name));
    name.add_css_class("search-result-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let full_path = item.path.to_string_lossy();
    row.set_tooltip_text(Some(&full_path));
    let path = gtk::Label::new(Some(&full_path));
    path.add_css_class("search-result-path");
    path.set_xalign(0.0);
    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    labels.append(&name);
    labels.append(&path);
    content.append(&labels);
    row.set_child(Some(&content));
    row
}

fn refresh_visible_thumbnails(state: &SearchState) {
    let adjustment = state.scroller.vadjustment();
    let viewport_top = adjustment.value();
    let viewport_height = adjustment.page_size();
    let changes = {
        let items = state.visible_results.borrow();
        let mut requested = state.requested_thumbnails.borrow_mut();
        let mut changes = Vec::new();
        for (position, item) in items.iter().enumerate() {
            let Some(row) = i32::try_from(position)
                .ok()
                .and_then(|position| state.list.row_at_index(position))
            else {
                continue;
            };
            let Some(bounds) = row.compute_bounds(&state.list) else {
                continue;
            };
            let visible = intersects_viewport(
                f64::from(bounds.y()),
                f64::from(bounds.height()),
                viewport_top,
                viewport_height,
            );
            let Some(image) = row
                .child()
                .and_then(|content| content.first_child())
                .and_then(|child| child.downcast::<super::thumbnail::ThumbnailSlot>().ok())
            else {
                continue;
            };
            if visible && requested.insert(item.path.clone()) {
                changes.push((image, Some(item.path.clone()), item.is_directory));
            } else if !visible && requested.remove(&item.path) {
                changes.push((image, None, item.is_directory));
            }
        }
        changes
    };
    for (image, path, is_directory) in changes {
        let fallback = if is_directory {
            crate::assets::icons::FOLDER
        } else {
            crate::assets::icons::DOCUMENTS
        };
        if let Some(path) = path {
            super::thumbnail::set_thumbnail_or_icon_for_path(&image, &path, fallback, 19, 32);
        } else {
            super::thumbnail::show_fallback_icon(&image, fallback, 19);
        }
    }
}

fn intersects_viewport(
    row_top: f64,
    row_height: f64,
    viewport_top: f64,
    viewport_height: f64,
) -> bool {
    row_top < viewport_top + viewport_height && row_top + row_height > viewport_top
}

fn contains_keyboard_focus(widget: &gtk::Widget) -> bool {
    widget
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| gtk::prelude::GtkWindowExt::focus(&window))
        .is_some_and(|focused| focused == *widget || focused.is_ancestor(widget))
}

fn record_interaction(state: &SearchState) {
    state
        .interaction_revision
        .set(state.interaction_revision.get() + 1);
}

fn focus_results(state: &SearchState) {
    state.navigation_started.set(true);
    if let Some(row) = state.list.selected_row() {
        row.grab_focus();
        scroll_row_into_view(state, &row);
    } else {
        state.layer.grab_focus();
    }
}

fn move_selection(state: &SearchState, direction: i32) {
    record_interaction(state);
    let editing = contains_keyboard_focus(state.field.upcast_ref());
    let count = state.visible_results.borrow().len() as i32;
    if count == 0 {
        return;
    }

    let current = state.list.selected_row().map_or(0, |row| row.index());
    let next = if state.navigation_started.replace(true) {
        (current + direction).clamp(0, count - 1)
    } else {
        current.clamp(0, count - 1)
    };
    if let Some(row) = state.list.row_at_index(next) {
        state.list.select_row(Some(&row));
        if !editing {
            row.grab_focus();
        }
        scroll_row_into_view(state, &row);
    }
}

fn scroll_row_into_view(state: &SearchState, row: &gtk::ListBoxRow) {
    let Some(bounds) = row.compute_bounds(&state.list) else {
        return;
    };
    let adjustment = state.scroller.vadjustment();
    let viewport_top = adjustment.value();
    let viewport_bottom = viewport_top + adjustment.page_size();
    let row_top = f64::from(bounds.y());
    let row_bottom = row_top + f64::from(bounds.height());
    if row_top < viewport_top {
        adjustment.set_value(row_top);
    } else if row_bottom > viewport_bottom {
        adjustment.set_value(row_bottom - adjustment.page_size());
    }
}

fn activate_selected(state: &Rc<SearchState>) -> bool {
    let Some(row) = state.list.selected_row() else {
        return false;
    };
    activate_position(state, row.index());
    true
}

fn activate_position(state: &Rc<SearchState>, position: i32) {
    let Some(item) = usize::try_from(position)
        .ok()
        .and_then(|position| state.visible_results.borrow().get(position).cloned())
    else {
        return;
    };
    hide(state);
    (state.activate)(item);
}

fn hide(state: &SearchState) {
    state.generation.set(state.generation.get() + 1);
    record_interaction(state);
    state.search.borrow_mut().take();
    clear_results(state);
    state.truncated_hint.set_visible(false);
    state.indexing_spinner.stop();
    state.indexing_spinner.set_visible(false);
    if state.layer.has_css_class("dismissing") {
        return;
    }
    state.layer.add_css_class("dismissing");
    state.layer.set_sensitive(false);
    let layer = state.layer.clone();
    let dismiss = state.dismiss.clone();
    super::browser::animate_out(&state.layer, move || {
        layer.set_visible(false);
        layer.remove_css_class("dismissing");
        layer.set_sensitive(true);
        dismiss();
    });
}

fn clear_results(state: &SearchState) {
    state.navigation_started.set(false);
    state.reconciling_results.set(true);
    state.visible_results.borrow_mut().clear();
    state.rendered_query.borrow_mut().clear();
    state.positions.borrow_mut().clear();
    state.requested_thumbnails.borrow_mut().clear();
    state.scroller.vadjustment().set_value(0.0);
    while let Some(child) = state.list.first_child() {
        super::thumbnail::cancel_thumbnails_in(&child);
        state.list.remove(&child);
    }
    state.reconciling_results.set(false);
}

#[cfg(test)]
mod tests;

// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::{FileEntry, Location};
use crate::ui::browser::ViewState;
use crate::ui::browser::clipboard::install_directory_drop_target;
use crate::ui::browser::collection::{
    ViewMap, activate_recursive_search_result, apply_filter_query, apply_selection_plan,
    bitset_positions, deactivate_recursive_search, debounce_filter_entry, detach_collection_view,
    recursive_search_activation_key, scroll_collection_when_allocated,
    search_result_navigation_position,
};
use crate::ui::browser::context_menu::{install_folder_context_menu, install_item_context_menu};
use crate::ui::browser::entry::{entry_filter, entry_model_value, format_file_size};
use crate::ui::browser::inline_edit::update_basename_validation;
use crate::ui::browser::pane_header::{
    column_sort_direction_toggle, column_sort_menu, empty_trash_button, pane_new_folder_button,
    pane_refresh_button,
};
use crate::ui::browser::paths::is_trash_root;
use crate::ui::browser::presentation::LoadPresentation;
use crate::ui::browser_modes::{ClickActivation, ClickCount};
use crate::ui::entry_list_model::EntryListModel;
use crate::ui::motion::{animations_enabled, emphasized_deceleration};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(super) const COLUMN_WIDTH: i32 = 300;

const COLUMN_OFFSET: i32 = 24;

const COLUMN_TRANSITION: Duration = Duration::from_millis(220);

pub(super) struct BoundRow {
    pub(super) item: glib::WeakRef<gtk::ListItem>,
    pub(super) row: glib::WeakRef<gtk::Box>,
}

struct PendingPointerActivation {
    pub(super) position: usize,
    pub(super) location: Location,
    pub(super) press: (f64, f64),
    pub(super) moved: bool,
}

impl PendingPointerActivation {
    pub(super) fn update(&mut self, x: f64, y: f64, drag_threshold: i32) {
        let threshold = f64::from(drag_threshold);
        self.moved |= (x - self.press.0).abs() > threshold || (y - self.press.1).abs() > threshold;
    }

    fn can_activate(&self, location: &Location) -> bool {
        !self.moved && self.location == *location
    }
}

#[derive(Clone)]
pub(super) struct ColumnView {
    pub(super) shell: gtk::Box,
    pub(super) destination_hint: gtk::Label,
    pub(super) animation_generation: Rc<Cell<u64>>,
    pub(super) presentation: LoadPresentation,
    pub(super) model: EntryListModel,
    pub(super) filtered_model: gtk::FilterListModel,
    pub(super) map: ViewMap,
    pub(super) model_generation: Rc<Cell<u64>>,
    pub(super) header_actions: gtk::Box,
    pub(super) filter_entry: gtk::Entry,
    pub(super) filter_button: gtk::ToggleButton,
    pub(super) selection: gtk::MultiSelection,
    pub(super) syncing_selection: Rc<Cell<bool>>,
    pub(super) list: gtk::ListView,
    pub(super) marquee: crate::ui::marquee::Marquee,
    pub(super) bound_rows: Rc<RefCell<Vec<BoundRow>>>,
    pub(super) entry_count: Rc<Cell<usize>>,
    pub(super) spinner: gtk::Spinner,
    pub(super) spinner_delay: Rc<RefCell<Option<glib::SourceId>>>,
    pub(super) truncated_hint: gtk::Image,
    pub(super) empty_trash_button: Option<gtk::Button>,
    pub(super) new_entry_row: gtk::Box,
    pub(super) new_entry_icon: gtk::Image,
    pub(super) new_entry_entry: gtk::Entry,
    pub(super) show_hidden: Rc<Cell<bool>>,
    pub(super) filter: gtk::CustomFilter,
    pub(super) search_results: Rc<RefCell<Vec<crate::services::SearchItem>>>,
    pub(super) search_handle: Rc<RefCell<Option<crate::services::SearchHandle>>>,
    pub(super) search_generation: Rc<Cell<u64>>,
    pub(super) search_model: gtk::StringList,
}

pub(super) fn column_size_text(entry: Option<&FileEntry>) -> String {
    entry
        .filter(|entry| !entry.is_directory())
        .and_then(|entry| match entry.size {
            crate::model::MetadataValue::Known(bytes) => Some(format_file_size(bytes)),
            crate::model::MetadataValue::Unknown | crate::model::MetadataValue::Unavailable => None,
        })
        .unwrap_or_default()
}

const COLUMN_SPINNER_DELAY: std::time::Duration = std::time::Duration::from_millis(120);

fn arm_column_spinner(column: &ColumnView) {
    cancel_column_spinner(column);
    let spinner = column.spinner.clone();
    let delay = column.spinner_delay.clone();
    *column.spinner_delay.borrow_mut() = Some(glib::timeout_add_local_once(
        COLUMN_SPINNER_DELAY,
        move || {
            // Spent: disarm so a later cancel never removes a fired id
            // (GLib refuses, and the unwrap would abort the main loop).
            delay.borrow_mut().take();
            spinner.set_visible(true);
            spinner.start();
        },
    ));
}

fn cancel_column_spinner(column: &ColumnView) {
    if let Some(source) = column.spinner_delay.borrow_mut().take() {
        source.remove();
    }
}

pub(super) fn stop_column_spinner(column: &ColumnView) {
    cancel_column_spinner(column);
    column.spinner.stop();
    column.spinner.set_visible(false);
}

pub(super) fn set_column_busy(column: &ColumnView, busy: bool) {
    column
        .list
        .update_state(&[gtk::accessible::State::Busy(busy)]);
}

pub(super) fn set_filter_placeholder(column: &ColumnView, count: usize) {
    let noun = if count == 1 { "item" } else { "items" };
    column
        .filter_entry
        .set_placeholder_text(Some(&format!("Filter {count} {noun}…")));
}

pub(super) fn touch_source_model(column: &ColumnView) {
    column
        .model_generation
        .set(column.model_generation.get().saturating_add(1));
}

pub(super) fn scroll_column_to(column: &ColumnView, position: u32) {
    if position >= column.selection.n_items() {
        return;
    }
    scroll_collection_when_allocated(column.list.upcast_ref(), position);
}

pub(super) fn set_column_selection(column: &ColumnView, position: u32) {
    column.syncing_selection.set(true);
    column.selection.unselect_all();
    if position != gtk::INVALID_LIST_POSITION {
        column.selection.select_item(position, true);
    }
    column.syncing_selection.set(false);
}

pub(super) fn set_column_selections(column: &ColumnView, positions: &[u32]) {
    column.syncing_selection.set(true);
    apply_selection_plan(
        &column.selection,
        column.filtered_model.n_items(),
        positions,
    );
    column.syncing_selection.set(false);
}

fn should_activate_single_click(
    press_count: i32,
    is_directory: bool,
    activation: ClickActivation,
    control: bool,
    shift: bool,
    preserve_group: bool,
) -> bool {
    let configured = if is_directory {
        activation.folders
    } else {
        activation.files
    };
    press_count == 1 && configured == ClickCount::One && !control && !shift && !preserve_group
}

fn should_preview_pointer_press(
    press_count: i32,
    control: bool,
    shift: bool,
    preserve_group: bool,
) -> bool {
    press_count == 1 && !control && !shift && !preserve_group
}

pub(super) fn is_column_background(surface: &gtk::Widget, picked: &gtk::Widget) -> bool {
    let mut current = Some(picked.clone());
    while let Some(widget) = current {
        if widget == *surface {
            return true;
        }
        if widget.is::<gtk::Button>()
            || widget.is::<gtk::Editable>()
            || widget.is::<gtk::Range>()
            || widget.is::<gtk::Scrollbar>()
        {
            return false;
        }
        current = widget.parent();
    }
    false
}

fn should_preserve_drag_selection(clicked_selected: bool, selected_count: u64) -> bool {
    clicked_selected && selected_count > 1
}

pub(super) fn update_empty_trash_sensitivity(column: &ColumnView, count: usize) {
    if let Some(button) = &column.empty_trash_button {
        button.set_sensitive(count > 0);
    }
}

fn file_row_target(mut target: gtk::Widget) -> Option<gtk::Box> {
    loop {
        if target.has_css_class("file-row") {
            return target.downcast::<gtk::Box>().ok();
        }
        if target.is::<gtk::ListView>() {
            return None;
        }
        target = target.parent()?;
    }
}

fn is_file_row_target(target: gtk::Widget) -> bool {
    file_row_target(target).is_some()
}

fn set_active_path_style(row: &gtk::Box, active: bool) {
    if active {
        row.add_css_class("active-path");
    } else {
        row.remove_css_class("active-path");
    }
}

pub(super) fn set_cut_path_style(row: &gtk::Box, cut: bool) {
    if cut {
        row.add_css_class("cut");
    } else {
        row.remove_css_class("cut");
    }
}

fn animate_column_entry(shell: &gtk::Box, column: &gtk::Box, generation: &Rc<Cell<u64>>) {
    let animation_id = generation.get().saturating_add(1);
    generation.set(animation_id);
    if !animations_enabled() {
        column.set_opacity(1.0);
        column.set_margin_start(0);
        return;
    }

    column.set_opacity(0.0);
    column.set_margin_start(COLUMN_OFFSET);
    let started = Instant::now();
    let shell = shell.clone();
    let column = column.clone();
    let generation = generation.clone();
    let _tick = shell.add_tick_callback(move |_, _| {
        if generation.get() != animation_id {
            return glib::ControlFlow::Break;
        }
        let progress =
            (started.elapsed().as_secs_f64() / COLUMN_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = emphasized_deceleration(progress);
        column.set_opacity(eased);
        column.set_margin_start((f64::from(COLUMN_OFFSET) * (1.0 - eased)).round() as i32);
        if progress >= 1.0 {
            column.set_opacity(1.0);
            column.set_margin_start(0);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn resized_column_width(initial_width: i32, horizontal_offset: f64) -> i32 {
    (f64::from(initial_width) + horizontal_offset)
        .round()
        .max(f64::from(COLUMN_WIDTH)) as i32
}

pub(in crate::ui) fn max_child_natural_width(widget: &gtk::Widget) -> i32 {
    let (_, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
    let mut max_natural = natural;
    let mut child = widget.first_child();
    while let Some(c) = child {
        let child_max = max_child_natural_width(&c);
        if child_max > max_natural {
            max_natural = child_max;
        }
        child = c.next_sibling();
    }
    max_natural
}

fn horizontal_reveal_target(
    current: f64,
    page_size: f64,
    lower: f64,
    upper: f64,
    item_left: f64,
    item_right: f64,
) -> f64 {
    let viewport_right = current + page_size;
    let target = if item_right > viewport_right {
        item_right - page_size
    } else if item_left < current {
        item_left
    } else {
        current
    };
    target.clamp(lower, (upper - page_size).max(lower))
}

fn animate_horizontal_scroll(
    scroller: &gtk::ScrolledWindow,
    adjustment: &gtk::Adjustment,
    target: f64,
    generation: &Rc<Cell<u64>>,
    animation_id: u64,
) {
    let start = adjustment.value();
    if !animations_enabled() || (target - start).abs() < 0.5 {
        adjustment.set_value(target);
        return;
    }

    let started = Instant::now();
    let adjustment = adjustment.clone();
    let generation = generation.clone();
    let _tick = scroller.add_tick_callback(move |_, _| {
        if generation.get() != animation_id {
            return glib::ControlFlow::Break;
        }
        let progress =
            (started.elapsed().as_secs_f64() / COLUMN_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = emphasized_deceleration(progress);
        adjustment.set_value(start + (target - start) * eased);
        if progress >= 1.0 {
            adjustment.set_value(target);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

impl ViewState {
    pub(super) fn rebuild_columns(self: &Rc<Self>) {
        self.truncate(0);
        let snapshots = (0..)
            .map_while(|depth| self.browser.column_snapshot(depth))
            .collect::<Vec<_>>();

        for (depth, snapshot) in snapshots.iter().enumerate() {
            self.append_column(depth, &snapshot.location);
        }
        for (column, snapshot) in self.columns.borrow().iter().zip(snapshots) {
            touch_source_model(column);
            column.model.replace(snapshot.count as u32);
            column.entry_count.set(snapshot.count);
            set_filter_placeholder(column, snapshot.count);
            update_empty_trash_sensitivity(column, snapshot.count);
            column.truncated_hint.set_visible(snapshot.truncated);
            let positions = snapshot
                .selected_positions
                .into_iter()
                .filter_map(|position| column.map.view_position(position))
                .collect::<Vec<_>>();
            set_column_selections(column, &positions);
            if snapshot.loading {
                column.spinner.start();
                column.presentation.show_loading();
            } else {
                column.spinner.stop();
                column.spinner.set_visible(false);
                if let Some(message) = snapshot.error.as_deref() {
                    column
                        .presentation
                        .show_error(&format!("Unable to read this directory\n{message}"));
                } else if snapshot.count == 0 {
                    column.presentation.show_empty();
                } else {
                    column.presentation.show_content();
                }
            }
        }
        self.focus_rebuilt_active_column();
    }

    fn focus_rebuilt_active_column(&self) {
        let Some(depth) = self.browser.active_depth() else {
            return;
        };
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return;
        };
        let position = self
            .browser
            .focused_item()
            .and_then(|(focused_depth, position, _)| {
                (focused_depth == depth)
                    .then(|| column.map.view_position(position))
                    .flatten()
            });
        if let Some(position) = position {
            scroll_column_to(column, position);
        }
        column.list.grab_focus();
        let list = column.list.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(list) = list.upgrade() {
                list.grab_focus();
            }
        });
    }

    pub(super) fn refresh_active_path_rows(&self) {
        self.refresh_destination_style();
        for (depth, column) in self.columns.borrow().iter().enumerate() {
            let active = self
                .browser
                .active_child_position(depth)
                .and_then(|position| column.map.view_position(position));
            column.bound_rows.borrow_mut().retain(|bound| {
                let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                    return false;
                };
                set_active_path_style(&row, active == Some(item.position()));
                true
            });
        }
    }

    pub(super) fn append_column(self: &Rc<Self>, depth: usize, location: &Location) {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("directory-column");
        column.set_hexpand(true);
        column.set_vexpand(true);
        let pane_motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        pane_motion.connect_enter(move |_, _, _| {
            if let Some(state) = weak.upgrade() {
                state.hovered_column.set(Some(depth));
                state.refresh_destination_style();
            }
        });
        let weak = Rc::downgrade(self);
        pane_motion.connect_leave(move |_| {
            if let Some(state) = weak.upgrade()
                && state.hovered_column.get() == Some(depth)
            {
                state.hovered_column.set(None);
                state.refresh_destination_style();
            }
        });
        column.add_controller(pane_motion);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("column-header");
        let heading_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        heading_box.set_hexpand(true);
        let heading = gtk::Label::new(Some(&location.display_name()));
        heading.set_xalign(0.0);
        heading.set_tooltip_text(Some(&location.display_path()));
        let truncated_hint = crate::assets::primary_icon(crate::assets::icons::TRIANGLE_ALERT, 16);
        truncated_hint.set_tooltip_text(Some(
            "This directory has more entries than could be loaded; showing a partial listing.",
        ));
        truncated_hint.set_visible(false);
        heading_box.append(&heading);
        heading_box.append(&truncated_hint);
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        header.append(&heading_box);
        header.append(&spinner);
        let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header_actions.add_css_class("column-header-actions");
        let empty_trash = empty_trash_button(&self.browser);
        let is_trash = is_trash_root(location);
        empty_trash.set_visible(is_trash);
        empty_trash.set_sensitive(false);
        header_actions.append(&empty_trash);
        if !self.interactive {
            header_actions.append(&pane_new_folder_button(Rc::downgrade(self), depth));
        }
        header_actions.append(&pane_refresh_button(&self.browser, depth));
        header_actions.append(&column_sort_direction_toggle(&self.browser, depth));
        header_actions.append(&column_sort_menu(&self.browser, depth));

        let filter_entry = gtk::Entry::builder()
            .placeholder_text("Filter 0 items…")
            .has_frame(false)
            .hexpand(true)
            .build();
        filter_entry.add_css_class("column-filter-entry");
        let filter_icon = crate::assets::primary_icon(crate::assets::icons::FUNNEL, 16);
        let filter_control = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        filter_control.add_css_class("column-filter");
        filter_control.append(&filter_icon);
        filter_control.append(&filter_entry);
        let filter_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .child(&filter_control)
            .build();
        let filter_button = gtk::ToggleButton::builder()
            .tooltip_text("Filter this pane (Ctrl+F)")
            .build();
        filter_button.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::FUNNEL,
            16,
        )));
        filter_button.add_css_class("column-header-action");
        let shown_filter = filter_revealer.clone();
        let focused_filter = filter_entry.clone();
        filter_button.connect_toggled(move |button| {
            shown_filter.set_reveal_child(button.is_active());
            if button.is_active() {
                focused_filter.grab_focus();
            } else {
                focused_filter.set_text("");
            }
        });
        header_actions.append(&filter_button);
        if depth > 0 {
            let close = gtk::Button::builder()
                .tooltip_text("Close this pane")
                .build();
            close.set_child(Some(&crate::assets::primary_icon(
                crate::assets::icons::X,
                16,
            )));
            close.add_css_class("column-header-action");
            let weak_browser = Rc::downgrade(&self.browser);
            close.connect_clicked(move |_| {
                if let Some(browser) = weak_browser.upgrade() {
                    browser.close_column(depth);
                }
            });
            header_actions.append(&close);
        }
        header.append(&header_actions);
        column.append(&header);
        column.append(&filter_revealer);

        let entry_count = Rc::new(Cell::new(0));
        let browser_for_model = self.browser.clone();
        let model = EntryListModel::new(Rc::new(move |position| {
            let position = position as usize;
            browser_for_model
                .with_entries(depth, position..position.saturating_add(1), |entries| {
                    entries.first().map(entry_model_value)
                })
                .flatten()
        }));
        let filter_query = Rc::new(RefCell::new(String::new()));
        let initial_show_hidden = self
            .browser
            .column_preferences(depth)
            .map_or_else(|| self.browser.preferences().show_hidden, |p| p.show_hidden);
        let show_hidden = Rc::new(Cell::new(initial_show_hidden));
        let filter = entry_filter(show_hidden.clone(), filter_query.clone());
        let filtered_model = gtk::FilterListModel::new(Some(model.clone()), Some(filter.clone()));
        let map = ViewMap::new(
            filter_query.clone(),
            show_hidden.clone(),
            self.source_generation.clone(),
            model.clone(),
            filtered_model.clone(),
            None,
        );
        let selection = gtk::MultiSelection::new(Some(filtered_model.clone()));
        let recursive_search_active = Rc::new(Cell::new(false));
        let syncing_selection = Rc::new(Cell::new(false));
        let modified_selection = Rc::new(Cell::new(false));
        let focused_filtered = Rc::new(Cell::new(None::<u32>));
        let weak_selection_state = Rc::downgrade(self);
        let map_for_selection = map.clone();
        let syncing_selection_changed = syncing_selection.clone();
        let focused_filtered_changed = focused_filtered.clone();
        let multiple_selection = self.multiple_selection.clone();
        let filter_for_column = filter.clone();
        let search_active_for_selection = recursive_search_active.clone();
        selection.connect_selection_changed(move |selection, position, count| {
            if syncing_selection_changed.get() || search_active_for_selection.get() {
                return;
            }
            let mut filtered_positions = bitset_positions(&selection.selection());
            let changed_end = position.saturating_add(count);
            let focused = filtered_positions
                .iter()
                .rev()
                .copied()
                .find(|candidate| *candidate >= position && *candidate < changed_end)
                .or_else(|| {
                    focused_filtered_changed
                        .get()
                        .filter(|candidate| filtered_positions.contains(candidate))
                })
                .or_else(|| filtered_positions.last().copied());
            if !multiple_selection.get()
                && filtered_positions.len() > 1
                && let Some(focused) = focused
            {
                syncing_selection_changed.set(true);
                selection.select_item(focused, true);
                syncing_selection_changed.set(false);
                filtered_positions.clear();
                filtered_positions.push(focused);
            }
            let mapped_positions = map_for_selection.source_positions(&filtered_positions);
            let source_positions: Vec<_> = mapped_positions
                .iter()
                .map(|(_, source_position)| *source_position)
                .collect();
            focused_filtered_changed.set(focused);
            let focused_source = focused.and_then(|position| {
                mapped_positions
                    .iter()
                    .find_map(|(filtered, source)| (*filtered == position).then_some(*source))
            });
            if let Some(state) = weak_selection_state.upgrade() {
                state
                    .browser
                    .set_selection(depth, &source_positions, focused_source);
                state.refresh_destination_style();
            }
        });
        let search_results: Rc<RefCell<Vec<crate::services::SearchItem>>> =
            Rc::new(RefCell::new(Vec::new()));
        let search_handle: Rc<RefCell<Option<crate::services::SearchHandle>>> =
            Rc::new(RefCell::new(None));
        let search_generation: Rc<Cell<u64>> = Rc::new(Cell::new(0));
        let search_model = gtk::StringList::new(&[]);

        let weak_state_for_search = Rc::downgrade(self);
        let depth_for_search = depth;
        let filtered_model_for_search = filtered_model.clone();
        let model_for_search = model.clone();
        let search_model_for_changed = search_model.clone();
        let search_results_for_changed = search_results.clone();
        let search_handle_for_changed = search_handle.clone();
        let search_gen_for_changed = search_generation.clone();
        let search_active_for_changed = recursive_search_active.clone();
        let weak_filter_entry = filter_entry.downgrade();
        debounce_filter_entry(&filter_entry, move |text| {
            let query = text.trim().to_string();
            if query.is_empty() {
                search_gen_for_changed.set(search_gen_for_changed.get().saturating_add(1));
                search_handle_for_changed.borrow_mut().take();
                deactivate_recursive_search(
                    &search_active_for_changed,
                    &search_results_for_changed,
                    &search_model_for_changed,
                    &filtered_model_for_search,
                    &model_for_search,
                );
                apply_filter_query(
                    &filtered_model_for_search,
                    &filter,
                    &filter_query,
                    text.to_lowercase(),
                );
                return;
            }
            *filter_query.borrow_mut() = text.to_lowercase();
            search_active_for_changed.set(true);
            let weak_entry = weak_filter_entry.clone();
            let weak_state = weak_state_for_search.clone();
            let filtered = filtered_model_for_search.clone();
            let sm = search_model_for_changed.clone();
            let results = search_results_for_changed.clone();
            let handle = search_handle_for_changed.clone();
            let search_gen = search_gen_for_changed.clone();
            if handle.borrow().is_none() {
                let Some(state) = weak_state.upgrade() else {
                    return;
                };
                let Some(path) = state
                    .browser
                    .location_at(depth_for_search)
                    .and_then(|loc| loc.native_path().map(Path::to_path_buf))
                else {
                    return;
                };
                search_gen.set(search_gen.get().saturating_add(1));
                let poll_gen = search_gen.get();
                let show_hidden = state
                    .browser
                    .column_preferences(depth_for_search)
                    .unwrap_or_else(|| state.browser.preferences())
                    .show_hidden;
                let (h, receiver) = crate::services::index_tree(path, show_hidden);
                handle.replace(Some(h));
                filtered.set_filter(None::<&gtk::CustomFilter>);
                filtered.set_model(Some(&sm));
                let weak_entry = weak_entry.clone();
                let weak_sm = sm.downgrade();
                let weak_filtered = filtered.downgrade();
                let results = results.clone();
                let gen_check = search_gen.clone();
                let _poll = glib::timeout_add_local(Duration::from_millis(16), move || {
                    if gen_check.get() != poll_gen {
                        return glib::ControlFlow::Break;
                    }
                    let mut latest = None;
                    for _ in 0..8 {
                        match receiver.try_recv() {
                            Ok(event) => latest = Some(event),
                            Err(std::sync::mpsc::TryRecvError::Empty) => break,
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                return glib::ControlFlow::Break;
                            }
                        }
                    }
                    if let Some(crate::services::SearchEvent::Results { query, items, .. }) = latest
                        && let Some(entry) = weak_entry.upgrade()
                        && !query.is_empty()
                        && query == entry.text().trim()
                    {
                        let Some(sm) = weak_sm.upgrade() else {
                            return glib::ControlFlow::Break;
                        };
                        let labels: Vec<_> = items.iter().map(|item| item.name.clone()).collect();
                        results.replace(items);
                        let labels: Vec<_> = labels.iter().map(String::as_str).collect();
                        sm.splice(0, sm.n_items(), &labels);
                        if let Some(fm) = weak_filtered.upgrade() {
                            fm.items_changed(0, sm.n_items(), sm.n_items());
                        }
                    }
                    glib::ControlFlow::Continue
                });
            }
            if let Some(h) = handle.borrow().as_ref() {
                h.query(&query);
            }
        });

        let rows::ColumnRows {
            factory,
            bound_rows,
        } = rows::column_rows(
            self,
            depth,
            &map,
            &selection,
            &modified_selection,
            &recursive_search_active,
            &search_results,
        );

        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("file-list");
        list.set_enable_rubberband(false);
        list.set_single_click_activate(false);
        list.set_vexpand(true);

        let search_navigation = gtk::EventControllerKey::new();
        search_navigation.set_propagation_phase(gtk::PropagationPhase::Capture);
        let search_active_for_navigation = recursive_search_active.clone();
        let selection_for_navigation = selection.clone();
        let syncing_for_navigation = syncing_selection.clone();
        let list_for_navigation = list.clone();
        let browser_for_navigation = Rc::downgrade(&self.browser);
        let results_for_navigation = search_results.clone();
        search_navigation.connect_key_pressed(move |_, key, _, modifiers| {
            if !search_active_for_navigation.get()
                || modifiers.intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK
                        | gtk::gdk::ModifierType::ALT_MASK
                        | gtk::gdk::ModifierType::SUPER_MASK
                        | gtk::gdk::ModifierType::SHIFT_MASK,
                )
            {
                return glib::Propagation::Proceed;
            }
            let current = bitset_positions(&selection_for_navigation.selection())
                .last()
                .copied();
            if recursive_search_activation_key(key) {
                return if current.is_some_and(|position| {
                    activate_recursive_search_result(
                        &browser_for_navigation,
                        &results_for_navigation,
                        position,
                    )
                }) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                };
            }
            let direction = match key {
                gtk::gdk::Key::Down => 1,
                gtk::gdk::Key::Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let Some(next) = search_result_navigation_position(
                current,
                selection_for_navigation.n_items(),
                direction,
            ) else {
                return glib::Propagation::Stop;
            };
            syncing_for_navigation.set(true);
            selection_for_navigation.select_item(next, true);
            syncing_for_navigation.set(false);
            list_for_navigation.scroll_to(next, gtk::ListScrollFlags::empty(), None);
            glib::Propagation::Stop
        });
        filter_entry.add_controller(search_navigation);

        let selection_keys = gtk::EventControllerKey::new();
        selection_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let modified_for_key = modified_selection.clone();
        selection_keys.connect_key_pressed(move |_, _, _, modifiers| {
            modified_for_key.set(
                modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                    || modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK),
            );
            glib::Propagation::Proceed
        });
        let modified_for_key = modified_selection.clone();
        selection_keys.connect_key_released(move |_, _, _, _| {
            modified_for_key.set(false);
        });
        list.add_controller(selection_keys);

        let weak_browser = Rc::downgrade(&self.browser);
        let map_for_activation = map.clone();
        let search_handle_for_activate = search_handle.clone();
        let search_results_for_activate = search_results.clone();
        list.connect_activate(move |_, position| {
            if search_handle_for_activate.borrow().is_some() {
                activate_recursive_search_result(
                    &weak_browser,
                    &search_results_for_activate,
                    position,
                );
                return;
            }
            let source_position = map_for_activation.source_position(position);
            if let (Some(browser), Some(source_position)) =
                (weak_browser.upgrade(), source_position)
            {
                browser.activate(depth, source_position);
            }
        });

        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        scroll.add_css_class("fixed-scrollbar");
        crate::ui::scrolling::install_autoscroll(&scroll, &self.overlay);
        let rows_for_marquee = bound_rows.clone();
        let marquee = crate::ui::marquee::install(crate::ui::marquee::MarqueeSetup {
            view: list.clone().upcast(),
            scroll: scroll.clone(),
            overlay: self.overlay.clone(),
            targets: Rc::new(RefCell::new(vec![crate::ui::marquee::MarqueeTarget {
                selection: selection.clone(),
                visit_items: Rc::new(move |visit| {
                    rows_for_marquee.borrow_mut().retain(|bound| {
                        let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade())
                        else {
                            return false;
                        };
                        visit(item.position(), row.upcast_ref());
                        true
                    });
                }),
            }])),
            is_item: Rc::new(|widget| is_file_row_target(widget.clone())),
        });
        marquee.add_origin_surface(&header);

        let retry = gtk::Button::with_label("Retry");
        retry.add_css_class("retry-button");
        let weak_browser = Rc::downgrade(&self.browser);
        retry.connect_clicked(move |_| {
            if let Some(browser) = weak_browser.upgrade() {
                browser.retry_column(depth);
            }
        });
        let new_entry_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        new_entry_row.add_css_class("file-row");
        new_entry_row.add_css_class("new-entry-row");
        new_entry_row.set_visible(false);
        let new_entry_icon = crate::assets::primary_icon(crate::assets::icons::FOLDER, 17);
        new_entry_icon.add_css_class("file-icon");
        let new_entry_entry = gtk::Entry::new();
        new_entry_entry.add_css_class("inline-rename");
        new_entry_entry.set_hexpand(true);
        new_entry_entry.connect_changed(|field| {
            update_basename_validation(field);
        });
        new_entry_row.append(&new_entry_icon);
        new_entry_row.append(&new_entry_entry);
        let weak_state = Rc::downgrade(self);
        new_entry_entry.connect_activate(move |field| {
            if let Some(state) = weak_state.upgrade() {
                state.submit_new_entry(field);
            }
        });
        let new_entry_focus = gtk::EventControllerFocus::new();
        let weak_state = Rc::downgrade(self);
        let field = new_entry_entry.clone();
        new_entry_focus.connect_leave(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.submit_new_entry(&field);
            }
        });
        new_entry_entry.add_controller(new_entry_focus);

        let presentation = LoadPresentation::new(&scroll, Some(retry));
        presentation.stack.set_focusable(true);
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_enter(move |_| {
            if let Some(state) = weak.upgrade() {
                state.browser.set_active_column(depth);
                state.refresh_destination_style();
            }
        });
        column.add_controller(focus);
        let background = gtk::GestureClick::new();
        background.set_button(1);
        background.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        background.connect_pressed(move |gesture, _, x, y| {
            let Some(surface) = gesture.widget() else {
                return;
            };
            let Some(picked) = surface.pick(x, y, gtk::PickFlags::DEFAULT) else {
                return;
            };
            if is_file_row_target(picked.clone()) || !is_column_background(&surface, &picked) {
                return;
            }
            if let Some(state) = weak.upgrade() {
                state.browser.set_active_column(depth);
                state.browser.focus_active();
            }
        });
        presentation.stack.add_controller(background);
        if self.interactive {
            install_directory_drop_target(self, &presentation.stack, location.clone());
        }
        install_folder_context_menu(
            self,
            presentation.stack.upcast_ref(),
            {
                let entries = selection.downgrade();
                Rc::new(move || {
                    entries
                        .upgrade()
                        .is_some_and(|entries| entries.n_items() > 0)
                })
            },
            Rc::new(|picked| is_file_row_target(picked.clone())),
            depth,
            location.clone(),
        );
        let rows_for_context = bound_rows.clone();
        let pick_position = Rc::new(move |picked: &gtk::Widget| {
            let picked = file_row_target(picked.clone())?;
            rows_for_context.borrow().iter().find_map(|bound| {
                let row = bound.row.upgrade()?;
                let item = bound.item.upgrade()?;
                (row == picked).then_some(item.position())
            })
        });
        {
            let map_for_context = map.clone();
            let source_position =
                Rc::new(move |position| map_for_context.source_position(position));
            install_item_context_menu(
                self,
                list.upcast_ref(),
                &selection,
                pick_position,
                source_position,
                Rc::new(|| {}),
                depth,
            );
        }
        column.append(&new_entry_row);
        column.append(&presentation.stack);
        let destination_hint = gtk::Label::new(None);
        destination_hint.add_css_class("column-destination-hint");
        destination_hint.set_xalign(0.0);
        destination_hint.set_tooltip_text(Some(
            "Ctrl+V pastes into this directory. Move the pointer to target another column, or navigate with the keyboard to return control to keyboard focus.",
        ));
        column.append(&destination_hint);

        let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        shell.set_size_request(COLUMN_WIDTH, -1);
        shell.set_vexpand(true);
        shell.set_overflow(gtk::Overflow::Hidden);
        let column_overlay = gtk::Overlay::new();
        column_overlay.set_child(Some(&column));
        column_overlay.set_hexpand(true);
        column_overlay.set_vexpand(true);
        shell.append(&column_overlay);
        let resize_handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        resize_handle.add_css_class("column-resize-handle");
        resize_handle.set_width_request(7);
        resize_handle.set_cursor_from_name(Some("col-resize"));
        let resize = gtk::GestureDrag::new();
        resize.set_button(1);
        let resize_start = Rc::new(Cell::new(COLUMN_WIDTH));
        let pointer_start = Rc::new(Cell::new(None));
        let last_press = Rc::new(Cell::new(0u64));
        let shell_for_resize_start = shell.downgrade();
        let shell_for_autofit = shell.downgrade();
        let column_for_autofit = column.downgrade();
        let resize_start_for_begin = resize_start.clone();
        let pointer_start_for_begin = pointer_start.clone();
        let last_press_for_begin = last_press.clone();
        resize.connect_drag_begin(move |gesture, _, _| {
            let now = glib::monotonic_time() as u64;
            let prev = last_press_for_begin.get();
            last_press_for_begin.set(now);
            let Some(shell_for_autofit) = shell_for_autofit.upgrade() else {
                return;
            };
            let Some(shell_for_resize_start) = shell_for_resize_start.upgrade() else {
                return;
            };
            if now.wrapping_sub(prev) <= 400_000 {
                let max_natural = column_for_autofit
                    .upgrade()
                    .map(|column| max_child_natural_width(column.upcast_ref::<gtk::Widget>()))
                    .unwrap_or(COLUMN_WIDTH);
                shell_for_autofit.set_size_request(max_natural.max(COLUMN_WIDTH), -1);
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            resize_start_for_begin.set(shell_for_resize_start.width().max(COLUMN_WIDTH));
            if let Some((pointer_x, _)) = gesture.current_event().and_then(|event| event.position())
            {
                pointer_start_for_begin.set(Some(pointer_x));
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        let shell_for_resize = shell.downgrade();
        resize.connect_drag_update(move |gesture, fallback_offset_x, _| {
            let Some(shell_for_resize) = shell_for_resize.upgrade() else {
                return;
            };
            let pointer_x = gesture
                .current_event()
                .and_then(|event| event.position())
                .map(|(pointer_x, _)| pointer_x);
            let offset_x = pointer_start
                .get()
                .zip(pointer_x)
                .map_or(fallback_offset_x, |(start, current)| current - start);
            shell_for_resize
                .set_size_request(resized_column_width(resize_start.get(), offset_x), -1);
        });
        resize_handle.add_controller(resize);
        resize_handle.set_halign(gtk::Align::End);
        resize_handle.set_valign(gtk::Align::Fill);
        column_overlay.add_overlay(&resize_handle);
        let animation_generation = Rc::new(Cell::new(0));
        let previous = depth
            .checked_sub(1)
            .and_then(|previous| self.columns.borrow().get(previous).cloned())
            .map(|column| column.shell);
        self.columns_widget
            .insert_child_after(&shell, previous.as_ref());
        self.columns.borrow_mut().push(ColumnView {
            shell: shell.clone(),
            destination_hint,
            animation_generation: animation_generation.clone(),
            presentation,
            model,
            filtered_model,
            map,
            model_generation: self.source_generation.clone(),
            header_actions,
            filter_entry,
            filter_button,
            selection,
            syncing_selection,
            list,
            marquee,
            bound_rows,
            entry_count,
            spinner,
            spinner_delay: Rc::new(RefCell::new(None)),
            truncated_hint,
            empty_trash_button: is_trash.then_some(empty_trash),
            new_entry_row,
            new_entry_icon,
            new_entry_entry,
            show_hidden,
            filter: filter_for_column,
            search_results,
            search_handle,
            search_generation,
            search_model,
        });

        if let Some(column) = self.columns.borrow().last() {
            set_column_busy(column, true);
            arm_column_spinner(column);
        }
        self.refresh_active_path_rows();
        animate_column_entry(&shell, &column, &animation_generation);
        self.reveal_column(shell);
    }

    fn reveal_column(self: &Rc<Self>, shell: gtk::Box) {
        let animation_id = self.horizontal_scroll_generation.get().saturating_add(1);
        self.horizontal_scroll_generation.set(animation_id);
        let weak = Rc::downgrade(self);
        let measured_shell = shell.downgrade();
        let _tick = self.scroller.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(measured_shell) = measured_shell.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.horizontal_scroll_generation.get() != animation_id
                || measured_shell.parent().is_none()
            {
                return glib::ControlFlow::Break;
            }
            let adjustment = state.scroller.hadjustment();
            if measured_shell.width() <= 0 || adjustment.page_size() <= 0.0 {
                return glib::ControlFlow::Continue;
            }
            let Some(bounds) = measured_shell.compute_bounds(&state.columns_widget) else {
                return glib::ControlFlow::Continue;
            };
            let target = horizontal_reveal_target(
                adjustment.value(),
                adjustment.page_size(),
                adjustment.lower(),
                adjustment.upper(),
                f64::from(bounds.x()),
                f64::from(bounds.x() + bounds.width()),
            );
            animate_horizontal_scroll(
                &state.scroller,
                &adjustment,
                target,
                &state.horizontal_scroll_generation,
                animation_id,
            );
            glib::ControlFlow::Break
        });
    }

    pub(super) fn truncate(self: &Rc<Self>, len: usize) {
        self.close_peek_visual();
        if self.hovered_column.get().is_some_and(|depth| depth >= len) {
            self.hovered_column.set(None);
        }
        self.cancel_rename();
        self.cancel_new_entry();
        self.horizontal_scroll_generation
            .set(self.horizontal_scroll_generation.get().saturating_add(1));
        while self.columns.borrow().len() > len {
            let Some(column) = self.columns.borrow_mut().pop() else {
                break;
            };
            column
                .animation_generation
                .set(column.animation_generation.get().saturating_add(1));
            column.syncing_selection.set(true);
            column.selection.set_model(None::<&gio::ListModel>);
            column.filtered_model.set_model(None::<&gio::ListModel>);
            detach_collection_view(&column.list);
            self.columns_widget.remove(&column.shell);
            self.overlay.remove_overlay(&column.marquee.band());
        }
        let retained = self
            .columns
            .borrow()
            .last()
            .map(|column| column.shell.clone());
        if let Some(retained) = retained {
            self.reveal_column(retained);
        }
    }
}

mod rows;

#[cfg(test)]
mod tests;

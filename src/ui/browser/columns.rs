// SPDX-License-Identifier: MIT

use crate::model::{FileEntry, Location};
use crate::services::fold_for_search;
use crate::ui::browser::ViewState;
use crate::ui::browser::clipboard::install_directory_drop_target;
use crate::ui::browser::collection::{
    ActivePaneFilter, ViewMap, activate_recursive_search_result, apply_filter_query,
    apply_selection_plan, bind_filter_query, bitset_positions, cancel_source,
    deactivate_recursive_search, detach_collection_view, filter_placeholder,
    recursive_search_activation_key, restore_filter_controls, scroll_collection_when_allocated,
    search_result_navigation_position,
};
use crate::ui::browser::context_menu::{install_folder_context_menu, install_item_context_menu};
use crate::ui::browser::entry::{entry_filter, entry_model_value, format_file_size};
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

pub(in crate::ui) const COLUMN_WIDTH: i32 = 300;

pub(super) const COLUMN_TRANSITION: Duration = Duration::from_millis(220);

pub(super) fn install_horizontal_scroll(state: &Rc<ViewState>) {
    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    // Nested listing scrollers consume horizontal events even with horizontal scrolling disabled.
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_state = Rc::downgrade(state);
    controller.connect_scroll(move |controller, dx, dy| {
        let Some(state) = weak_state.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let shifted = controller
            .current_event_state()
            .contains(gtk::gdk::ModifierType::SHIFT_MASK);
        let (dx, dy) = if shifted { (dy, dx) } else { (dx, dy) };
        if dx.abs() <= dy.abs() {
            return glib::Propagation::Proceed;
        }
        let adjustment = state.scroller.hadjustment();
        let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        if max <= adjustment.lower() {
            return glib::Propagation::Proceed;
        }
        let scale = match controller.unit() {
            gtk::gdk::ScrollUnit::Wheel => adjustment.page_size().powf(2.0 / 3.0),
            gtk::gdk::ScrollUnit::Surface => 2.5,
            _ => 1.0,
        };
        state
            .horizontal_scroll_generation
            .set(state.horizontal_scroll_generation.get().saturating_add(1));
        adjustment.set_value((adjustment.value() + dx * scale).clamp(adjustment.lower(), max));
        glib::Propagation::Stop
    });
    state.scroller.add_controller(controller);
}

const RESIZE_EDGE_RIGHT: f64 = 6.0;

fn resize_edge(state: &ViewState, x: f64, y: f64) -> Option<gtk::Box> {
    state.columns.borrow().iter().find_map(|column| {
        let bounds = column.shell.compute_bounds(&state.scroller)?;
        let right = f64::from(bounds.x() + bounds.width());
        (x >= right - 1.0
            && x < right + RESIZE_EDGE_RIGHT
            && y >= f64::from(bounds.y())
            && y < f64::from(bounds.y() + bounds.height()))
        .then(|| column.shell.clone())
    })
}

pub(super) fn install_resize_edges(state: &Rc<ViewState>) {
    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    motion.connect_motion(move |_, x, y| {
        if let Some(state) = weak.upgrade() {
            let hovered = resize_edge(&state, x, y);
            for column in state.columns.borrow().iter() {
                if hovered.as_ref() == Some(&column.shell) {
                    column.resize_handle.add_css_class("resize-hover");
                } else {
                    column.resize_handle.remove_css_class("resize-hover");
                }
            }
            state
                .scroller
                .set_cursor_from_name(hovered.map(|_| "col-resize"));
        }
    });
    let weak = Rc::downgrade(state);
    motion.connect_leave(move |_| {
        if let Some(state) = weak.upgrade() {
            for column in state.columns.borrow().iter() {
                column.resize_handle.remove_css_class("resize-hover");
            }
            state.scroller.set_cursor(None);
        }
    });
    state.scroller.add_controller(motion);

    let resize = gtk::GestureDrag::new();
    resize.set_button(1);
    resize.set_propagation_phase(gtk::PropagationPhase::Capture);
    let active = Rc::new(RefCell::new(None::<(gtk::Box, i32, f64)>));
    let last_press = Rc::new(RefCell::new(None::<(gtk::Box, u64)>));
    let weak = Rc::downgrade(state);
    let active_for_begin = active.clone();
    resize.connect_drag_begin(move |gesture, x, y| {
        active_for_begin.borrow_mut().take();
        let Some(state) = weak.upgrade() else {
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        };
        let Some(shell) = resize_edge(&state, x, y) else {
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        };
        let now = glib::monotonic_time() as u64;
        let autofit = last_press
            .borrow()
            .as_ref()
            .is_some_and(|(previous, time)| {
                *previous == shell && now.wrapping_sub(*time) <= 400_000
            });
        *last_press.borrow_mut() = Some((shell.clone(), now));
        if autofit {
            let max_natural = shell
                .first_child()
                .and_downcast::<gtk::Overlay>()
                .and_then(|overlay| overlay.child())
                .map(|column| max_child_natural_width(&column))
                .unwrap_or(COLUMN_WIDTH);
            shell.set_size_request(max_natural.max(COLUMN_WIDTH), -1);
            gesture.set_state(gtk::EventSequenceState::Claimed);
            return;
        }
        let pointer_x = gesture
            .current_event()
            .and_then(|event| event.position())
            .map_or(x, |(pointer_x, _)| pointer_x);
        *active_for_begin.borrow_mut() =
            Some((shell.clone(), shell.width().max(COLUMN_WIDTH), pointer_x));
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    let active_for_update = active.clone();
    resize.connect_drag_update(move |gesture, fallback_offset_x, _| {
        let active = active_for_update.borrow();
        let Some((shell, initial, start)) = active.as_ref() else {
            return;
        };
        let offset_x = gesture
            .current_event()
            .and_then(|event| event.position())
            .map_or(fallback_offset_x, |(current, _)| current - start);
        shell.set_size_request(resized_column_width(*initial, offset_x), -1);
    });
    resize.connect_drag_end(move |_, _, _| {
        active.borrow_mut().take();
    });
    state.scroller.add_controller(resize);
}

pub(super) struct BoundRow {
    pub(super) item: glib::WeakRef<gtk::ListItem>,
    pub(super) row: glib::WeakRef<gtk::Box>,
    pub(super) rename_label: glib::WeakRef<gtk::Label>,
    pub(super) edit: crate::ui::collection_edit::EditWidgets,
    pub(super) spacer: gtk::Box,
    pub(super) size: gtk::Label,
}

#[derive(Clone, Copy)]
pub(super) enum PendingActivationKind {
    Standard { preview: bool },
    ChooserSearchNavigate,
    RecursiveSearch,
    Mapped,
}

struct PendingPointerActivation {
    // Source-model index for Standard/Mapped; search-result index otherwise.
    pub(super) position: usize,
    pub(super) location: Location,
    pub(super) press: (f64, f64),
    pub(super) moved: bool,
    pub(super) kind: PendingActivationKind,
}

impl PendingPointerActivation {
    pub(super) fn update(&mut self, x: f64, y: f64, drag_threshold: i32) {
        self.moved |=
            crate::ui::pointer::exceeds_drag_threshold(self.press, (x, y), drag_threshold);
    }

    fn can_activate(&self, location: &Location) -> bool {
        !self.moved && self.location == *location
    }
}

#[derive(Clone)]
pub(super) struct ColumnView {
    pub(super) shell: gtk::Box,
    pub(super) resize_handle: gtk::Box,
    pub(super) reveal_button: gtk::Button,
    pub(super) animation_generation: Rc<Cell<u64>>,
    pub(super) presentation: LoadPresentation,
    pub(super) model: EntryListModel,
    pub(super) filtered_model: gtk::FilterListModel,
    pub(super) map: ViewMap,
    pub(super) model_generation: Rc<Cell<u64>>,
    pub(super) header_actions: gtk::Box,
    pub(super) sort_direction_button: gtk::Button,
    pub(super) header_actions_stack: gtk::Stack,
    pub(super) filter_entry: gtk::Entry,
    pub(super) filter_button: gtk::ToggleButton,
    pub(super) selection: gtk::MultiSelection,
    pub(super) syncing_selection: Rc<Cell<bool>>,
    pub(super) list: gtk::ListView,
    pub(super) listing_scroll: gtk::ScrolledWindow,
    pub(super) marquee: crate::ui::marquee::Marquee,
    pub(super) bound_rows: Rc<RefCell<Vec<BoundRow>>>,
    pub(super) folder_context_trigger: Rc<dyn Fn(f64, f64)>,
    pub(super) item_context_trigger: Rc<dyn Fn(f64, f64)>,
    pub(super) entry_count: Rc<Cell<usize>>,
    pub(super) spinner: gtk::Spinner,
    pub(super) spinner_delay: Rc<RefCell<Option<glib::SourceId>>>,
    pub(super) truncated_hint: gtk::Image,
    pub(super) empty_trash_button: Option<gtk::Button>,
    pub(super) show_hidden: Rc<Cell<bool>>,
    pub(super) filter: gtk::CustomFilter,
    pub(super) search_results: Rc<RefCell<Vec<crate::services::SearchItem>>>,
    pub(super) search_session: crate::ui::search_session::SearchSession,
    query_binding: Rc<RefCell<Option<super::collection::FilterQueryBinding>>>,
    pub(super) search_model: gtk::StringList,
    pub(super) recursive_search_active: Rc<Cell<bool>>,
}

impl ColumnView {
    pub(super) fn context_menu_target(
        &self,
        position: Option<usize>,
    ) -> Option<crate::ui::browser::ContextMenuTarget> {
        let position = if self.recursive_search_active.get() {
            bitset_positions(&self.selection.selection())
                .last()
                .copied()
        } else {
            position.and_then(|position| self.map.view_position(position))
        };
        if let Some(position) = position {
            let row = self.bound_rows.borrow().iter().find_map(|bound| {
                let item = bound.item.upgrade()?;
                (item.position() == position).then(|| bound.row.upgrade())?
            })?;
            let bounds = row.compute_bounds(&self.list)?;
            return Some((
                self.item_context_trigger.clone(),
                f64::from(bounds.x() + bounds.width()),
                f64::from(bounds.center().y()),
            ));
        }
        let width = f64::from(self.presentation.stack.width());
        let height = f64::from(self.presentation.stack.height());
        (width > 0.0 && height > 0.0).then(|| {
            (
                self.folder_context_trigger.clone(),
                width / 2.0,
                height / 2.0,
            )
        })
    }
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

pub(super) fn refresh_source_filter(column: &ColumnView, browser: &crate::app::Browser) -> bool {
    if !column.search_session.is_active() {
        return false;
    }
    let items = column
        .search_results
        .borrow()
        .iter()
        .filter(|item| browser.allows_entry(&super::search_result_entry(item)))
        .cloned()
        .collect();
    let changed = search::update_results(
        &column.search_model,
        &column.search_results,
        &column.selection,
        &column.syncing_selection,
        items,
    );
    column
        .search_session
        .query(column.filter_entry.text().trim());
    changed
}

pub(super) fn prune_missing_search_results(column: &ColumnView) -> bool {
    if !column.search_session.is_active() {
        return false;
    }
    let items: Vec<_> = column
        .search_results
        .borrow()
        .iter()
        .filter(|item| crate::ui::inline_search::search_path_present(&item.path))
        .cloned()
        .collect();
    if items.len() == column.search_results.borrow().len() {
        return false;
    }
    search::update_results(
        &column.search_model,
        &column.search_results,
        &column.selection,
        &column.syncing_selection,
        items,
    )
}

pub(super) fn set_filter_placeholder(column: &ColumnView, count: usize) {
    column
        .filter_entry
        .set_placeholder_text(Some(&filter_placeholder(count)));
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

pub(super) fn restore_column_cursor(column: &ColumnView, position: u32) {
    let list = column.list.downgrade();
    let rows = column.bound_rows.clone();
    glib::idle_add_local_once(move || {
        let Some(list) = list.upgrade() else { return };
        let frames = Cell::new(0u8);
        list.add_tick_callback(move |list, _| {
            let focused = list.root().and_then(|root| root.focus());
            if !focused
                .as_ref()
                .is_some_and(|focused| focused == list || list.is_ancestor(focused))
            {
                return glib::ControlFlow::Break;
            }
            let cursor = rows.borrow().iter().find_map(|bound| {
                let item = bound.item.upgrade()?;
                (item.position() == position)
                    .then(|| bound.row.upgrade()?.parent())
                    .flatten()
                    .filter(|cursor| cursor.is_mapped())
            });
            if let Some(cursor) = cursor {
                cursor.grab_focus();
                return glib::ControlFlow::Break;
            }
            frames.set(frames.get().saturating_add(1));
            if frames.get() >= 8 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    });
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
            || widget.is::<gtk::MenuButton>()
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

pub(super) fn set_active_path_style(row: &gtk::Box, active: bool, immediate: bool) {
    if active {
        row.add_css_class("active-path");
        if immediate {
            row.add_css_class("active-parent");
        } else {
            row.remove_css_class("active-parent");
        }
    } else {
        row.remove_css_class("active-path");
        row.remove_css_class("active-parent");
    }
}

pub(super) fn set_cut_path_style(row: &gtk::Box, cut: bool) {
    if cut {
        row.add_css_class("cut");
    } else {
        row.remove_css_class("cut");
    }
    if let Some(icon) = row
        .first_child()
        .and_downcast::<crate::ui::thumbnail::ThumbnailSlot>()
    {
        icon.set_cut(cut);
    }
}

fn animate_column_entry(column: &gtk::Box, generation: &Rc<Cell<u64>>) {
    let animation_id = generation.get().saturating_add(1);
    generation.set(animation_id);
    column.remove_css_class("column-entering");
    if !animations_enabled() {
        return;
    }

    column.add_css_class("column-entering");
    let column = column.downgrade();
    let generation = generation.clone();
    glib::timeout_add_local_once(COLUMN_TRANSITION, move || {
        if generation.get() == animation_id
            && let Some(column) = column.upgrade()
        {
            column.remove_css_class("column-entering");
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
    pub(super) fn clear_column_selections(&self) {
        let active = self.browser.active_depth();
        let selections: Vec<_> = self
            .columns
            .borrow()
            .iter()
            .map(|column| column.selection.clone())
            .collect();
        for selection in selections {
            selection.unselect_all();
        }
        if let Some(depth) = active {
            self.browser.set_active_column(depth);
        }
        self.refresh_destination_style();
    }

    pub(super) fn capture_active_column_filter(&self) -> ActivePaneFilter {
        let Some(depth) = self.browser.active_depth() else {
            return ActivePaneFilter::default();
        };
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return ActivePaneFilter::default();
        };
        ActivePaneFilter {
            query: column.filter_entry.text().to_string(),
            revealed: column.filter_button.is_active(),
        }
    }

    pub(super) fn restore_active_column_filter(&self, filter: &ActivePaneFilter) {
        let Some(depth) = self.browser.active_depth() else {
            return;
        };
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return;
        };
        restore_filter_controls(&column.filter_button, &column.filter_entry, filter);
    }

    pub(super) fn rebuild_columns_from(self: &Rc<Self>, from_depth: usize) {
        self.truncate(from_depth);
        let snapshots = (from_depth..)
            .map_while(|depth| self.browser.column_snapshot(depth))
            .collect::<Vec<_>>();

        for (offset, snapshot) in snapshots.iter().enumerate() {
            self.append_column(from_depth + offset, &snapshot.location);
        }
        for (column, snapshot) in self.columns.borrow().iter().skip(from_depth).zip(snapshots) {
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
                cancel_column_spinner(column);
                column.spinner.set_visible(true);
                column.spinner.start();
                if snapshot.count > 0 && snapshot.location.is_camera_photo_root() {
                    column.presentation.show_content();
                    set_column_busy(column, false);
                } else {
                    column.presentation.show_loading();
                }
            } else {
                // Rebuilt, already-loaded columns receive no finish event to cancel the timer.
                stop_column_spinner(column);
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
    }

    pub(super) fn focus_rebuilt_active_column(&self) {
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
        let active_depth = self.browser.active_depth();
        for (depth, column) in self.columns.borrow().iter().enumerate() {
            let active = self
                .browser
                .active_child_position(depth)
                .and_then(|position| column.map.view_position(position));
            let immediate = active_depth == Some(depth + 1);
            column.bound_rows.borrow_mut().retain(|bound| {
                let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                    return false;
                };
                set_active_path_style(&row, active == Some(item.position()), immediate);
                true
            });
        }
    }

    pub(super) fn append_column(self: &Rc<Self>, depth: usize, location: &Location) {
        let column = crate::ui::accessibility::pane_box();
        column.add_css_class("directory-column");
        crate::ui::accessibility::describe_pane(
            &column,
            &location.display_name(),
            crate::ui::browser_modes::BrowserMode::Columns,
        );
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
        heading_box.set_valign(gtk::Align::Center);
        let heading = gtk::Label::new(Some(&location.display_name()));
        heading.add_css_class("column-heading");
        heading.set_xalign(0.0);
        heading.set_yalign(0.5);
        heading.set_valign(gtk::Align::Center);
        heading.set_hexpand(true);
        heading.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        heading.set_max_width_chars(1);
        heading.set_tooltip_text(Some(&location.display_path()));
        let truncated_hint = crate::assets::primary_icon(crate::assets::icons::TRIANGLE_ALERT, 16);
        truncated_hint.add_css_class("column-truncated-hint");
        truncated_hint.set_tooltip_text(Some(
            "This directory has more entries than could be loaded; showing a partial listing.",
        ));
        truncated_hint.set_visible(false);
        heading_box.append(&heading);
        heading_box.append(&truncated_hint);
        let spinner = gtk::Spinner::new();
        spinner.set_valign(gtk::Align::Center);
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
        let sort_direction_button = column_sort_direction_toggle(&self.browser, depth);
        header_actions.append(&sort_direction_button);
        header_actions.append(&column_sort_menu(&self.browser, depth));

        let (filter_entry, filter_revealer, filter_button) =
            crate::ui::browser_modes::filter_controls("Filter this pane (Ctrl+F)");
        let weak_self = Rc::downgrade(self);
        filter_button.connect_toggled(move |button| {
            if button.is_active()
                && let Some(state) = weak_self.upgrade()
            {
                for (other_depth, other) in state.columns.borrow().iter().enumerate() {
                    if other_depth != depth && other.filter_button.is_active() {
                        other.filter_button.set_active(false);
                    }
                }
            }
        });
        header_actions.append(&filter_button);
        if depth > 0 {
            let close = gtk::Button::builder()
                .tooltip_text("Close this pane")
                .build();
            close.set_child(Some(&crate::assets::chrome_icon(crate::assets::icons::X)));
            crate::ui::controls::pane_header_action(&close);
            let weak_browser = Rc::downgrade(&self.browser);
            close.connect_clicked(move |_| {
                if crate::ui::tenxer_mode::chrome_suppressed() {
                    return;
                }
                if let Some(browser) = weak_browser.upgrade() {
                    browser.close_column(depth);
                }
            });
            crate::ui::tenxer_mode::hide_while_enabled(&close);
            header_actions.append(&close);
        }
        // Homogeneous pages keep column geometry stable as the action target changes.
        let header_actions_stack = gtk::Stack::new();
        header_actions_stack.set_valign(gtk::Align::Center);
        header_actions_stack.add_named(&header_actions, Some("actions"));
        header_actions_stack.add_named(
            &gtk::Box::new(gtk::Orientation::Horizontal, 0),
            Some("hidden"),
        );
        header.append(&header_actions_stack);
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
            if syncing_selection_changed.get() {
                return;
            }
            if search_active_for_selection.get() {
                if !multiple_selection.get() && selection.selection().size() > 1 {
                    let focused = (position..position.saturating_add(count))
                        .rev()
                        .find(|position| selection.is_selected(*position));
                    if let Some(focused) = focused {
                        syncing_selection_changed.set(true);
                        selection.select_item(focused, true);
                        syncing_selection_changed.set(false);
                    }
                }
                if let Some(state) = weak_selection_state.upgrade() {
                    state.notify_search_selection_changed();
                }
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
        let search_session = crate::ui::search_session::SearchSession::default();
        let search_model = gtk::StringList::new(&[]);

        let weak_state_for_search = Rc::downgrade(self);
        let depth_for_search = depth;
        let filtered_model_for_search = filtered_model.clone();
        let model_for_search = model.clone();
        let search_model_for_changed = search_model.clone();
        let search_results_for_changed = search_results.clone();
        let session_for_changed = search_session.clone();
        let search_active_for_changed = recursive_search_active.clone();
        let selection_for_search = selection.clone();
        let syncing_for_search = syncing_selection.clone();
        let filter_query_for_search = filter_query.clone();
        let query_binding = bind_filter_query(
            &filter_entry,
            &search_session,
            move |text, recursive, restart| {
                if restart {
                    session_for_changed.cancel();
                    let changed = search::update_results(
                        &search_model_for_changed,
                        &search_results_for_changed,
                        &selection_for_search,
                        &syncing_for_search,
                        Vec::new(),
                    );
                    if changed && let Some(state) = weak_state_for_search.upgrade() {
                        state.notify_search_selection_changed();
                    }
                }
                let query = text.trim().to_string();
                if query.is_empty() {
                    session_for_changed.cancel();
                    // Keep the hidden-file filter installed while swapping back to the directory
                    // model; GTK's synchronous model notifications otherwise leave a stale row.
                    apply_filter_query(
                        &filtered_model_for_search,
                        &filter,
                        &filter_query_for_search,
                        fold_for_search(&text),
                    );
                    deactivate_recursive_search(
                        &search_active_for_changed,
                        &search_results_for_changed,
                        &search_model_for_changed,
                        &filtered_model_for_search,
                        &model_for_search,
                    );
                    return;
                }
                let Some(state) = weak_state_for_search.upgrade() else {
                    return;
                };
                let Some(path) = state
                    .browser
                    .location_at(depth_for_search)
                    .and_then(|loc| loc.native_path().map(Path::to_path_buf))
                else {
                    session_for_changed.cancel();
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
                        &filter_query_for_search,
                        fold_for_search(&text),
                    );
                    return;
                };
                *filter_query_for_search.borrow_mut() = fold_for_search(&text);
                search_active_for_changed.set(true);
                if !session_for_changed.is_active() {
                    filtered_model_for_search.set_filter(None::<&gtk::CustomFilter>);
                    filtered_model_for_search.set_model(Some(&search_model_for_changed));
                }
                let show_hidden = state
                    .browser
                    .column_preferences(depth_for_search)
                    .unwrap_or_else(|| state.browser.preferences())
                    .show_hidden;
                let sm = search_model_for_changed.clone();
                let results = search_results_for_changed.clone();
                let selection = selection_for_search.clone();
                let syncing = syncing_for_search.clone();
                let browser = Rc::downgrade(&state.browser);
                let weak_state = weak_state_for_search.clone();
                session_for_changed.update(
                    crate::ui::search_session::SearchInput {
                        root: path,
                        show_hidden,
                        recursive,
                    },
                    &query,
                    restart,
                    Rc::new(move |batch| {
                        let (Some(browser), Some(state)) =
                            (browser.upgrade(), weak_state.upgrade())
                        else {
                            return;
                        };
                        let session = state
                            .columns
                            .borrow()
                            .get(depth_for_search)
                            .map(|column| column.search_session.clone());
                        let Some(session) = session else {
                            return;
                        };
                        let items = crate::ui::inline_search::eligible_results(
                            &browser,
                            &session,
                            &batch.query,
                            batch.items,
                            batch.has_more,
                        );
                        if search::update_results(&sm, &results, &selection, &syncing, items) {
                            state.notify_search_selection_changed();
                        }
                    }),
                );
            },
        );

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
        crate::ui::accessibility::describe_entry_container(&list, &location.display_name());

        let search_navigation = gtk::EventControllerKey::new();
        search_navigation.set_propagation_phase(gtk::PropagationPhase::Capture);
        let search_active_for_navigation = recursive_search_active.clone();
        let query_for_navigation = filter_query.clone();
        let map_for_navigation = map.clone();
        let depth_for_navigation = depth;
        let selection_for_navigation = selection.clone();
        let syncing_for_navigation = syncing_selection.clone();
        let list_for_navigation = list.clone();
        let browser_for_navigation = Rc::downgrade(&self.browser);
        let results_for_navigation = search_results.clone();
        let navigation_state = Rc::downgrade(self);
        search_navigation.connect_key_pressed(move |_, key, _, modifiers| {
            let filter_active = search_active_for_navigation.get()
                || !query_for_navigation.borrow().trim().is_empty();
            if !filter_active
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
                if search_active_for_navigation.get() {
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
                return if let Some(position) = current
                    && let Some(browser) = browser_for_navigation.upgrade()
                    && let Some(source_position) = map_for_navigation.source_position(position)
                {
                    browser.activate(depth_for_navigation, source_position);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                };
            }
            match key {
                gtk::gdk::Key::Down => {}
                gtk::gdk::Key::Up => return glib::Propagation::Stop,
                _ => return glib::Propagation::Proceed,
            }
            let Some(next) = current.or_else(|| {
                search_result_navigation_position(None, selection_for_navigation.n_items(), 1)
            }) else {
                return glib::Propagation::Stop;
            };
            let is_recursive = search_active_for_navigation.get();
            if is_recursive {
                syncing_for_navigation.set(true);
            }
            list_for_navigation.grab_focus();
            selection_for_navigation.select_item(next, true);
            if is_recursive {
                syncing_for_navigation.set(false);
            }
            list_for_navigation.scroll_to(next, gtk::ListScrollFlags::FOCUS, None);
            if let Some(state) = navigation_state.upgrade() {
                super::BrowserView { state }.keyboard_navigation();
            }
            glib::Propagation::Stop
        });
        filter_entry.add_controller(search_navigation);

        let return_to_filter = gtk::EventControllerKey::new();
        return_to_filter.set_propagation_phase(gtk::PropagationPhase::Capture);
        let search_active = recursive_search_active.clone();
        let query_for_return = filter_query.clone();
        let result_selection = selection.clone();
        let query = filter_entry.downgrade();
        let navigation_state = Rc::downgrade(self);
        return_to_filter.connect_key_pressed(move |controller, key, _, modifiers| {
            let filter_active = search_active.get() || !query_for_return.borrow().trim().is_empty();
            if !filter_active
                || !matches!(key, gtk::gdk::Key::Up | gtk::gdk::Key::Down)
                || modifiers.intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK
                        | gtk::gdk::ModifierType::SHIFT_MASK
                        | gtk::gdk::ModifierType::ALT_MASK
                        | gtk::gdk::ModifierType::SUPER_MASK,
                )
            {
                return glib::Propagation::Proceed;
            }
            if controller
                .widget()
                .and_then(|widget| widget.root())
                .and_then(|root| root.focus())
                .and_then(|focus| focus.ancestor(gtk::Popover::static_type()))
                .is_some()
            {
                return glib::Propagation::Proceed;
            }
            if let Some(state) = navigation_state.upgrade() {
                super::BrowserView { state }.keyboard_navigation();
            }
            if key != gtk::gdk::Key::Up
                || bitset_positions(&result_selection.selection()).as_slice() != [0]
            {
                return glib::Propagation::Proceed;
            }
            query
                .upgrade()
                .filter(|entry| entry.grab_focus_without_selecting())
                .map_or(glib::Propagation::Proceed, |_| glib::Propagation::Stop)
        });
        list.add_controller(return_to_filter);

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
        let weak_state_for_activate = Rc::downgrade(self);
        let map_for_activation = map.clone();
        let search_active_for_activate = recursive_search_active.clone();
        let search_results_for_activate = search_results.clone();
        list.connect_activate(move |_, position| {
            if let Some(state) = weak_state_for_activate.upgrade() {
                state.cancel_click_rename();
            }
            if search_active_for_activate.get() {
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
        scroll.add_css_class("browser-listing-scroll");
        crate::ui::scrolling::install_autoscroll(&scroll, &self.overlay);
        let retry = gtk::Button::with_label("Retry");
        retry.add_css_class("retry-button");
        let weak_browser = Rc::downgrade(&self.browser);
        retry.connect_clicked(move |_| {
            if let Some(browser) = weak_browser.upgrade() {
                browser.retry_column(depth);
            }
        });
        let presentation = LoadPresentation::new(&scroll, Some(retry));
        let rows_for_marquee = bound_rows.clone();
        let weak_for_clear = Rc::downgrade(self);
        let returning_to_column = Rc::new(Cell::new(false));
        let returning_for_clear = returning_to_column.clone();
        let search_active_for_clear = recursive_search_active.clone();
        let marquee_targets: Rc<RefCell<Vec<crate::ui::marquee::MarqueeTarget>>> =
            Rc::new(RefCell::new(vec![crate::ui::marquee::MarqueeTarget {
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
            }]));
        let marquee = crate::ui::marquee::install(crate::ui::marquee::MarqueeSetup {
            view: list.clone().upcast(),
            surface: presentation.stack.clone().upcast(),
            scroll: scroll.clone(),
            overlay: self.overlay.clone(),
            targets: marquee_targets.clone(),
            is_item: crate::ui::marquee::item_bounds_predicate(marquee_targets),
            clear_selection: Rc::new(move || {
                if let Some(state) = weak_for_clear.upgrade() {
                    state.clear_column_selections();
                    state.browser.close_column(depth + 1);
                    if returning_for_clear.replace(false) && !search_active_for_clear.get() {
                        let first = state.columns.borrow().get(depth).and_then(|column| {
                            (0..column.selection.n_items())
                                .find_map(|position| column.map.source_position(position))
                        });
                        if let Some(position) = first {
                            state.browser.select(depth, position);
                        }
                    }
                }
            }),
            allow_drag: self.multiple_selection.clone(),
        });
        marquee.add_origin_surface(&header);
        // GTK prepends controllers. Install these last so their capture-phase
        // press marks returning_to_column before the marquee clears selection.
        let background_clicks: Vec<_> = [
            presentation.stack.upcast_ref::<gtk::Widget>(),
            header.upcast_ref(),
        ]
        .into_iter()
        .map(|surface| {
            self.install_column_background_focus(surface, depth, returning_to_column.clone())
        })
        .collect();
        for click in &background_clicks {
            marquee.group_background_click(click);
        }

        presentation.stack.set_focusable(true);
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_enter(move |_| {
            if let Some(state) = weak.upgrade()
                && state
                    .context_menu_column
                    .get()
                    .is_none_or(|owner| owner == depth)
                && state.drop_active_depths.get().is_none()
            {
                state.browser.set_active_column(depth);
                state.refresh_destination_style();
            }
        });
        column.add_controller(focus);
        if self.interactive {
            install_directory_drop_target(self, &column, location.clone());
            install_directory_drop_target(self, &presentation.stack, location.clone());
        }
        let folder_context_trigger = install_folder_context_menu(
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
        let item_context_trigger = {
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
            )
        };
        column.append(&presentation.stack);

        let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        // Focus may be unset during transfer; inspect its destination at idle.
        let filter_button_for_blur = filter_button.clone();
        let shell_for_blur = shell.downgrade();
        let weak_browser_for_blur = Rc::downgrade(&self.browser);
        let filter_focus = gtk::EventControllerFocus::new();
        filter_focus.connect_leave(move |controller| {
            let Some(widget) = controller.widget() else {
                return;
            };
            let shell_for_blur = shell_for_blur.clone();
            let filter_button_for_blur = filter_button_for_blur.clone();
            let weak_browser = weak_browser_for_blur.clone();
            glib::idle_add_local_once(move || {
                let Some(shell) = shell_for_blur.upgrade() else {
                    return;
                };
                let Some(root) = widget.root() else {
                    return;
                };
                // Result dialogs must retain the query for restoration after dismissal.
                if root
                    .downcast_ref::<gtk::Window>()
                    .is_some_and(|window| crate::ui::window::visible_modal_layer(window).is_some())
                {
                    return;
                }
                let left_column = root.focus().is_some_and(|focused| {
                    !focused.is_ancestor(&shell)
                        && focused != shell.clone().upcast::<gtk::Widget>()
                        && focused.ancestor(gtk::Popover::static_type()).is_none()
                });
                if left_column
                    && weak_browser
                        .upgrade()
                        .is_some_and(|browser| !browser.is_chooser_mode())
                {
                    filter_button_for_blur.set_active(false);
                }
            });
        });
        shell.add_controller(filter_focus);

        shell.set_size_request(COLUMN_WIDTH, -1);
        let previous_scale = Cell::new(1.0);
        crate::ui::preferences::PreferenceManager::shared().bind_interface_scale(
            &shell,
            move |shell, scale| {
                let ratio = scale / previous_scale.replace(scale);
                shell.set_width_request((f64::from(shell.width_request()) * ratio).round() as i32);
            },
        );
        shell.set_vexpand(true);
        shell.set_overflow(gtk::Overflow::Hidden);
        let column_overlay = gtk::Overlay::new();
        column_overlay.set_child(Some(&column));
        column_overlay.set_hexpand(true);
        column_overlay.set_vexpand(true);
        // The column paints an opaque background, so a drop highlight needs an
        // overlay on top of it; picking falls through to the column's targets.
        let drop_veil = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        drop_veil.add_css_class("column-drop-veil");
        drop_veil.set_can_target(false);
        column_overlay.add_overlay(&drop_veil);
        shell.append(&column_overlay);
        let resize_handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        resize_handle.add_css_class("column-resize-handle");
        resize_handle.set_width_request(1);
        resize_handle.set_can_target(false);
        resize_handle.set_halign(gtk::Align::End);
        resize_handle.set_valign(gtk::Align::Fill);
        column_overlay.add_overlay(&resize_handle);
        let reveal_button = gtk::Button::new();
        reveal_button.add_css_class("column-peek-target");
        reveal_button.set_focusable(false);
        reveal_button.set_focus_on_click(false);
        // Let row drag sources receive presses through the peek overlay.
        reveal_button.set_can_target(false);
        reveal_button.set_cursor_from_name(Some("pointer"));
        reveal_button.set_visible(false);
        crate::ui::accessibility::set_label(
            &reveal_button,
            &format!("Reveal {} column", location.display_name()),
        );
        reveal_button.set_tooltip_text(Some(&format!("Reveal {}", location.display_path())));
        let weak = Rc::downgrade(self);
        let revealed_location = location.clone();
        reveal_button.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.reveal_column_only(depth, &revealed_location);
            }
        });
        let peek_motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        peek_motion.connect_enter(move |_, _, _| {
            if let Some(state) = weak.upgrade() {
                state.hovered_column.set(Some(depth));
                state.refresh_destination_style();
            }
        });
        let weak = Rc::downgrade(self);
        peek_motion.connect_leave(move |_| {
            if let Some(state) = weak.upgrade()
                && state.hovered_column.get() == Some(depth)
            {
                state.hovered_column.set(None);
                state.refresh_destination_style();
            }
        });
        reveal_button.add_controller(peek_motion);
        if self.interactive {
            install_directory_drop_target(self, &reveal_button, location.clone());
            install_directory_drop_target(self, &resize_handle, location.clone());
        }
        column_overlay.add_overlay(&reveal_button);
        let animation_generation = Rc::new(Cell::new(0));
        let previous = depth
            .checked_sub(1)
            .and_then(|previous| self.columns.borrow().get(previous).cloned())
            .map(|column| column.shell);
        self.columns_widget
            .insert_child_after(&shell, previous.as_ref());
        self.columns.borrow_mut().push(ColumnView {
            shell: shell.clone(),
            resize_handle: resize_handle.clone(),
            reveal_button,
            animation_generation: animation_generation.clone(),
            presentation,
            model,
            filtered_model,
            map,
            model_generation: self.source_generation.clone(),
            header_actions,
            sort_direction_button,
            header_actions_stack,
            filter_entry,
            filter_button,
            selection,
            syncing_selection,
            list,
            listing_scroll: scroll,
            marquee,
            bound_rows,
            folder_context_trigger,
            item_context_trigger,
            entry_count,
            spinner,
            spinner_delay: Rc::new(RefCell::new(None)),
            truncated_hint,
            empty_trash_button: is_trash.then_some(empty_trash),
            show_hidden,
            filter: filter_for_column,
            search_results,
            search_session,
            query_binding: Rc::new(RefCell::new(Some(query_binding))),
            search_model,
            recursive_search_active,
        });

        if let Some(column) = self.columns.borrow().last() {
            set_column_busy(column, true);
            arm_column_spinner(column);
        }
        self.refresh_active_path_rows();
        animate_column_entry(&column, &animation_generation);
        self.reveal_column(shell);
    }

    fn install_column_background_focus(
        self: &Rc<Self>,
        surface: &gtk::Widget,
        depth: usize,
        returning_to_column: Rc<Cell<bool>>,
    ) -> gtk::GestureClick {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let origin = Rc::new(Cell::new(None));
        let pressed_origin = origin.clone();
        let weak_for_press = Rc::downgrade(self);
        click.connect_pressed(move |gesture, _, x, y| {
            returning_to_column.set(
                weak_for_press
                    .upgrade()
                    .is_some_and(|state| state.browser.active_depth() != Some(depth)),
            );
            pressed_origin.set(None);
            let Some(surface) = gesture.widget() else {
                return;
            };
            let Some(picked) = surface.pick(x, y, gtk::PickFlags::DEFAULT) else {
                return;
            };
            if !is_file_row_target(picked.clone()) && is_column_background(&surface, &picked) {
                pressed_origin.set(Some((x, y)));
            }
        });
        let stopped_origin = origin.clone();
        click.connect_stopped(move |_| stopped_origin.set(None));
        let weak = Rc::downgrade(self);
        click.connect_released(move |gesture, _, x, y| {
            let (Some((start_x, start_y)), Some(surface)) = (origin.take(), gesture.widget())
            else {
                return;
            };
            if surface.drag_check_threshold(start_x as i32, start_y as i32, x as i32, y as i32) {
                return;
            }
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.drop_active_depths.get().is_some() {
                return;
            }
            state.suppress_focus_scroll.set(true);
            state.browser.set_active_column(depth);
            state.browser.focus_active();
            state.suppress_focus_scroll.set(false);
            let shell = state
                .columns
                .borrow()
                .get(depth)
                .map(|column| column.shell.clone());
            if let Some(shell) = shell {
                state.reveal_column(shell);
            }
            state.flash_column_parent(depth);
        });
        surface.add_controller(click.clone());
        click
    }

    pub(super) fn flash_column_parent(&self, child_depth: usize) {
        if child_depth == 0 {
            return;
        }
        let parent_depth = child_depth - 1;
        let columns = self.columns.borrow();
        let Some(column) = columns.get(parent_depth) else {
            return;
        };
        let active = self
            .browser
            .active_child_position(parent_depth)
            .and_then(|position| column.map.view_position(position));
        let Some(active_pos) = active else {
            return;
        };
        for bound in column.bound_rows.borrow().iter() {
            if bound.item.upgrade().map(|item| item.position()) == Some(active_pos) {
                if let Some(row) = bound.row.upgrade() {
                    row.remove_css_class("flash-active-path");
                    row.add_css_class("flash-active-path");
                    let weak_row = row.downgrade();
                    glib::timeout_add_local_once(Duration::from_millis(420), move || {
                        if let Some(row) = weak_row.upgrade() {
                            row.remove_css_class("flash-active-path");
                        }
                    });
                }
                break;
            }
        }
    }

    pub(super) fn reveal_column(self: &Rc<Self>, shell: gtk::Box) {
        let animation_id = self.horizontal_scroll_generation.get().saturating_add(1);
        self.horizontal_scroll_generation.set(animation_id);
        self.columns_widget.set_margin_end(0);
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
            let depth = state
                .columns
                .borrow()
                .iter()
                .position(|column| column.shell == measured_shell);
            let Some(span) = depth.and_then(|depth| state.column_span(depth)) else {
                return glib::ControlFlow::Break;
            };
            let target = span.reveal_target(
                adjustment.value(),
                adjustment.page_size(),
                adjustment.lower(),
                adjustment.upper(),
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
        self.columns_widget.set_margin_end(0);
        cancel_source(&self.pending_peek);
        self.peek_anchor.take();
        self.close_peek_visual();
        if self.hovered_column.get().is_some_and(|depth| depth >= len) {
            self.hovered_column.set(None);
        }
        if self
            .context_menu_column
            .get()
            .is_some_and(|depth| depth >= len)
        {
            self.context_menu_column.set(None);
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
            column.query_binding.take();
            column.search_session.cancel();
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

pub(super) mod drag_scroll;
mod reveal;
mod rows;
mod search;

pub(super) use reveal::ColumnSpan;

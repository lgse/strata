// SPDX-License-Identifier: MIT

use crate::model::{FileEntry, Location};
use crate::ui::browser::ViewState;
use crate::ui::browser::collection::{
    ViewMap, apply_selection_plan, bitset_positions, filter_placeholder,
};
use crate::ui::browser::find::{HitRanges, highlight_hit_name, highlight_listing_name};
use crate::ui::browser::presentation::LoadPresentation;
use crate::ui::browser_modes::{ClickActivation, ClickCount};
use crate::ui::entry_list_model::EntryListModel;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

pub(in crate::ui) const COLUMN_WIDTH: i32 = 300;

pub(super) const COLUMN_TRANSITION: Duration = Duration::from_millis(220);

#[derive(Clone)]
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
    Standard {
        preview: bool,
    },
    ChooserSearchNavigate,
    RecursiveSearch,
    /// A filtered file whose single click previews instead of launching.
    RecursivePreview,
    Mapped,
}

struct PendingPointerActivation {
    // Source-model index for Standard/Mapped; search-result index otherwise.
    pub(super) position: usize,
    pub(super) location: Location,
    pub(super) press: (f64, f64),
    pub(super) moved: bool,
    pub(super) kind: PendingActivationKind,
    pub(super) location_hold: Option<super::tab_location::TabLocationHold>,
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
    /// The intended width in unscaled pixels: the saved default, then the last
    /// drag or autofit. A text-size change scales this, not the capped width.
    pub(super) unscaled_width: Rc<Cell<f64>>,
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
    /// Bumped by each cursor restore so an older pending restore cannot land late.
    pub(super) cursor_restore_generation: Rc<Cell<u64>>,
    pub(super) folder_context_trigger: Rc<dyn Fn(f64, f64)>,
    pub(super) item_context_trigger: Rc<dyn Fn(f64, f64)>,
    pub(super) entry_count: Rc<Cell<usize>>,
    pub(super) spinner: gtk::Spinner,
    pub(super) spinner_delay: Rc<RefCell<Option<glib::SourceId>>>,
    pub(super) truncated_hint: gtk::Image,
    pub(super) not_responding: gtk::Box,
    pub(super) empty_trash_button: Option<gtk::Button>,
    pub(super) leftovers_button: gtk::Button,
    pub(super) show_hidden: Rc<Cell<bool>>,
    pub(super) filter: gtk::CustomFilter,
    pub(super) search_results: Rc<RefCell<Vec<crate::services::SearchItem>>>,
    pub(super) search_session: crate::ui::search_session::SearchSession,
    query_binding: Rc<RefCell<Option<super::collection::FilterQueryBinding>>>,
    pub(super) search_model: gtk::StringList,
    pub(super) recursive_search_active: Rc<Cell<bool>>,
    pub(super) renamed_result: crate::ui::inline_search::RenamedResult,
    hits: Rc<RefCell<ColumnHits>>,
}

#[derive(Default)]
pub(super) struct ColumnHits {
    pub(super) ranges: HitRanges,
    pub(super) recursive: bool,
}

impl ColumnView {
    fn remember_width(&self, width: i32) {
        let scale = crate::ui::preferences::PreferenceManager::shared().interface_scale();
        self.unscaled_width.set(f64::from(width) / scale);
    }

    /// Focuses the list, or the column surface while it shows an empty, error or
    /// loading page.
    pub(super) fn focus_surface(&self) -> bool {
        crate::ui::loading_skeleton::focus_surface_or(&self.presentation.stack, &self.list)
    }

    /// The bound row GTK shows at `position`. A reload during an inline rename can leave
    /// a detached row behind that still reports the position it had.
    pub(super) fn shown_row_at(&self, position: u32) -> Option<(gtk::Box, BoundRow)> {
        self.bound_rows.borrow().iter().find_map(|bound| {
            let row = bound.row.upgrade()?;
            (bound.item.upgrade()?.position() == position
                && row.is_mapped()
                && row.is_ancestor(&self.list))
            .then(|| (row, bound.clone()))
        })
    }

    fn result_lists(&self) -> search::ResultLists {
        search::ResultLists {
            model: self.search_model.clone(),
            results: self.search_results.clone(),
            selection: self.selection.clone(),
            syncing: self.syncing_selection.clone(),
            renamed: self.renamed_result.clone(),
        }
    }

    pub(super) fn refresh_name_highlights(&self, find: Option<&str>) {
        let searching = self.recursive_search_active.get();
        let results = self.search_results.borrow();
        let hits = self.hits.borrow();
        let filter = self.map.query();
        for bound in self.bound_rows.borrow().iter() {
            let (Some(label), Some(item)) = (bound.rename_label.upgrade(), bound.item.upgrade())
            else {
                continue;
            };
            if searching {
                let hit = results
                    .get(item.position() as usize)
                    .and_then(|hit| hits.ranges.get(&hit.path));
                highlight_hit_name(label.upcast_ref(), find, hit.map(Vec::as_slice));
            } else {
                highlight_listing_name(label.upcast_ref(), find, &filter);
            }
        }
    }

    pub(super) fn flush_filter_query(&self) {
        if let Some(binding) = self.query_binding.borrow().as_ref() {
            binding.flush();
        }
    }

    pub(super) fn with_query_binding<T>(
        &self,
        apply: impl FnOnce(&super::collection::FilterQueryBinding) -> T,
    ) -> Option<T> {
        self.query_binding.borrow().as_ref().map(apply)
    }

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
            let (row, _) = self.shown_row_at(position)?;
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
    entry.map_or_else(String::new, crate::ui::browser_modes::entry_size)
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
    let update = column.result_lists().update(items, true);
    if let Some(position) = update.renamed {
        focus_renamed_result(&column.list, position);
    }
    column
        .search_session
        .query(column.filter_entry.text().trim());
    update.selection_changed
}

impl ViewState {
    pub(super) fn focus_renamed_column_result(&self, depth: usize, position: u32) {
        let list = self
            .columns
            .borrow()
            .get(depth)
            .map(|column| column.list.clone());
        if let Some(list) = list {
            focus_renamed_result(&list, position);
        }
    }
}

/// Moves the cursor onto a renamed result while the column's results hold focus,
/// or while focus sits on a row the update removed.
fn focus_renamed_result(list: &gtk::ListView, position: u32) {
    let focused = list.root().and_then(|root| root.focus());
    if focused.as_ref().is_none_or(|focused| {
        focused.root().is_none() || focused == list || focused.is_ancestor(list)
    }) {
        list.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    }
}

pub(super) fn prune_missing_search_results(
    column: &ColumnView,
    browser: &Rc<crate::app::Browser>,
    selection_changed: impl Fn() + 'static,
) {
    if !column.search_session.is_active() {
        return;
    }
    let paths = column
        .search_results
        .borrow()
        .iter()
        .map(|item| item.path.clone())
        .collect();
    let lists = column.result_lists();
    let list = column.list.downgrade();
    let show_hidden = column.show_hidden.clone();
    let browser = Rc::downgrade(browser);
    column.search_session.prune_missing(paths, move |gone| {
        let Some(browser) = browser.upgrade() else {
            return;
        };
        let items = lists.results.borrow().clone();
        if !items.iter().any(|item| gone.contains(&item.path)) {
            return;
        }
        let items = lists.renamed.prune(items, &gone, |item| {
            (show_hidden.get() || !item.name.starts_with('.'))
                && browser.allows_entry(&super::search_result_entry(item))
        });
        let update = lists.update(items, true);
        if let Some(position) = update.renamed
            && let Some(list) = list.upgrade()
        {
            focus_renamed_result(&list, position);
        }
        if update.selection_changed {
            selection_changed();
        }
    });
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

pub(super) fn set_column_selections(column: &ColumnView, positions: &[u32]) {
    column.syncing_selection.set(true);
    apply_selection_plan(
        &column.selection,
        column.filtered_model.n_items(),
        positions,
    );
    column.syncing_selection.set(false);
}

pub(super) fn select_all_in_column(column: &ColumnView) {
    column.syncing_selection.set(true);
    column.selection.select_all();
    column.syncing_selection.set(false);
}

/// `plain_press` is a single press without Control or Shift.
fn should_activate_single_click(
    plain_press: bool,
    is_directory: bool,
    activation: ClickActivation,
) -> bool {
    let configured = if is_directory {
        activation.folders
    } else {
        activation.files
    };
    plain_press && configured == ClickCount::One
}

fn should_preview_pointer_press(press_count: i32, control: bool, shift: bool) -> bool {
    press_count == 1 && !control && !shift
}

/// Controls take their own presses, so they are not the column background.
fn is_column_control(widget: &gtk::Widget) -> bool {
    widget.is::<gtk::Button>()
        || widget.is::<gtk::MenuButton>()
        || widget.is::<gtk::Editable>()
        || widget.is::<gtk::Range>()
        || widget.is::<gtk::Scrollbar>()
}

pub(super) fn is_column_background(surface: &gtk::Widget, picked: &gtk::Widget) -> bool {
    let mut current = Some(picked.clone());
    while let Some(widget) = current {
        if widget == *surface {
            return true;
        }
        if is_column_control(&widget) {
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

pub(super) fn set_mark_path_style(row: &gtk::Box, mark: super::clipboard::ClipboardMark) {
    if mark == super::clipboard::ClipboardMark::Cut {
        row.add_css_class("cut");
    } else {
        row.remove_css_class("cut");
    }
    if let Some(icon) = row
        .first_child()
        .and_downcast::<crate::ui::thumbnail::ThumbnailSlot>()
    {
        icon.set_mark(mark);
    }
}

mod append;
mod cursor;
mod double_click;
pub(super) mod drag_scroll;
mod lifecycle;
mod resize;
mod reveal;
mod rows;
mod search;
mod state;
mod strip_scroll;

#[cfg(test)]
mod tests;

pub(super) use cursor::{restore_column_cursor, scroll_column_to};
pub(super) use double_click::{FolderPress, install_double_click_guard};
use resize::resize_edge;
pub(super) use resize::{column_floor, install_resize_edges};
pub(super) use reveal::ColumnSpan;
pub(super) use strip_scroll::install_horizontal_scroll;

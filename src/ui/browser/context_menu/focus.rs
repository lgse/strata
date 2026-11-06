// SPDX-License-Identifier: MIT

use crate::model::{FileEntry, Location};
use crate::ui::browser::ViewState;
use crate::ui::browser_modes::BrowserMode;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

fn context_menu_still_open_for(state: &ViewState, generation: u64, depth: usize) -> bool {
    state.context_menu_generation.get() == generation
        && state.context_menu_column.get() == Some(depth)
}

fn release_context_column(state: &ViewState) {
    state.context_menu_column.set(None);
    state.context_menu_focus.borrow_mut().take();
    state.refresh_destination_style();
}

/// Runs `release` on idle unless another menu took the column meanwhile.
fn release_context_column_later(state: &Rc<ViewState>, depth: usize, restore_focus: bool) {
    let generation = state.context_menu_generation.get();
    let weak = Rc::downgrade(state);
    glib::idle_add_local_once(move || {
        if let Some(state) = weak.upgrade()
            && context_menu_still_open_for(&state, generation, depth)
        {
            if restore_focus {
                restore_context_focus(&state, depth);
            }
            release_context_column(&state);
        }
    });
}

fn native_menu_closed(state: &Rc<ViewState>, depth: usize) {
    if gtk::minor_version() >= 22 {
        restore_context_focus(state, depth);
        release_context_column(state);
        return;
    }
    // Older GTK finishes the native menu grab after emitting closed.
    release_context_column_later(state, depth, true);
}

pub(in crate::ui::browser) fn bind_column_context_owner(
    state: &Rc<ViewState>,
    popover: &gtk::Popover,
    depth: usize,
) {
    let weak = Rc::downgrade(state);
    popover.connect_closed(move |popover| {
        let Some(state) = weak.upgrade() else {
            return;
        };
        if state.context_menu_column.get() != Some(depth) {
            return;
        }
        if popover.is::<gtk::PopoverMenu>() {
            native_menu_closed(&state, depth);
            return;
        }
        // Unmapping can focus the window's first control (for example Home).
        // Restore before another key is dispatched, not from a later idle callback.
        restore_context_focus(&state, depth);
        release_context_column_later(&state, depth, false);
    });
}

fn restore_context_focus(state: &ViewState, depth: usize) {
    if state.browser.active_depth() != Some(depth) {
        return;
    }
    if !context_search_active(state, depth) {
        state.browser.focus_active();
        return;
    }
    if let Some(focus) = state
        .context_menu_focus
        .borrow()
        .as_ref()
        .and_then(glib::WeakRef::upgrade)
        .filter(|focus| focus.is_mapped())
    {
        focus.grab_focus();
    }
}

pub(super) fn schedule_new_entry(
    state: &Rc<ViewState>,
    depth: usize,
    location: Location,
    is_directory: bool,
) {
    let weak = Rc::downgrade(state);
    glib::idle_add_local_once(move || {
        if let Some(state) = weak.upgrade() {
            state.begin_new_entry(depth, location, is_directory);
        }
    });
}

pub(in crate::ui::browser) fn context_search_active(state: &ViewState, depth: usize) -> bool {
    if state.mode_views.borrow().mode() != BrowserMode::Columns {
        return state
            .mode_views
            .borrow()
            .selected_search_results()
            .is_some();
    }
    state
        .columns
        .borrow()
        .get(depth)
        .is_some_and(|column| column.recursive_search_active.get())
}

pub(super) fn context_filter_or_search_active(state: &ViewState, depth: usize) -> bool {
    if state.mode_views.borrow().mode() != BrowserMode::Columns {
        return state.mode_views.borrow().filter_active();
    }
    state.columns.borrow().get(depth).is_some_and(|column| {
        column.recursive_search_active.get()
            || column.map.has_query()
            || !column.filter_entry.text().trim().is_empty()
    })
}

pub(in crate::ui::browser) fn focus_context_column(state: &Rc<ViewState>, depth: usize) {
    state
        .context_menu_generation
        .set(state.context_menu_generation.get().wrapping_add(1));
    state.context_menu_column.set(Some(depth));
    state.browser.set_active_column(depth);
    // GTK restores pre-popup focus on dismissal; make that the menu's own column.
    if !context_search_active(state, depth) {
        state.browser.focus_active();
    }
    state.context_menu_focus.replace(
        state
            .overlay
            .root()
            .and_then(|root| root.focus())
            .map(|focus| focus.downgrade()),
    );
    state.pointer_navigation();
}

pub(in crate::ui::browser) fn focus_context_entry(
    state: &Rc<ViewState>,
    depth: usize,
    position: Option<usize>,
    entry: &FileEntry,
) {
    if let Some(position) = current_context_position(state, depth, position, entry) {
        let positions = state.browser.selected_positions(depth);
        state.browser.commit_selection();
        state
            .browser
            .set_selection(depth, &positions, Some(position));
    } else {
        focus_search_result(state, depth, entry);
    }
    focus_context_column(state, depth);
}

pub(super) fn current_context_position(
    state: &ViewState,
    depth: usize,
    position: Option<usize>,
    entry: &FileEntry,
) -> Option<usize> {
    position.filter(|position| {
        state
            .browser
            .entry_at(depth, *position)
            .is_some_and(|current| current.location == entry.location)
    })
}

fn select_search_result_after_reflow(state: &Rc<ViewState>, path: std::path::PathBuf) {
    let weak = Rc::downgrade(state);
    // Grid reflow after opening the preview can replace search selection.
    let frames = std::cell::Cell::new(0);
    state.overlay.add_tick_callback(move |_, _| {
        let frame = frames.get() + 1;
        frames.set(frame);
        if frame < 2 {
            return glib::ControlFlow::Continue;
        }
        if let Some(state) = weak.upgrade() {
            state.mode_views.borrow().select_search_result(&path);
        }
        glib::ControlFlow::Break
    });
}

pub(in crate::ui::browser) fn preview_context_entry(
    state: &Rc<ViewState>,
    depth: usize,
    position: Option<usize>,
    entry: FileEntry,
) {
    if let Some(position) = current_context_position(state, depth, position, &entry) {
        state.browser.preview(depth, position);
        return;
    }
    let path = entry
        .location
        .native_path()
        .map(std::path::Path::to_path_buf);
    state.browser.request_preview(entry);
    if let Some(path) = path {
        select_search_result_after_reflow(state, path);
    }
}

fn column_search_position(
    column: &crate::ui::browser::columns::ColumnView,
    path: &std::path::Path,
) -> Option<u32> {
    if !column.recursive_search_active.get() {
        return None;
    }
    let position = column
        .search_results
        .borrow()
        .iter()
        .position(|item| item.path == path)?;
    Some(position as u32)
}

fn focus_search_result(state: &ViewState, depth: usize, entry: &FileEntry) {
    let Some(path) = entry.location.native_path() else {
        return;
    };
    if state.mode_views.borrow().mode() != BrowserMode::Columns {
        state.mode_views.borrow().focus_search_result(path);
        return;
    }
    let Some(column) = state.columns.borrow().get(depth).cloned() else {
        return;
    };
    let Some(position) = column_search_position(&column, path) else {
        return;
    };
    if let Some((row, _)) = column.shown_row_at(position) {
        if !column.selection.is_selected(position) {
            column.selection.select_item(position, true);
        }
        if let Some(item) = row.parent() {
            item.grab_focus();
        }
    }
}

pub(in crate::ui::browser) fn rename_context_entry(
    state: &Rc<ViewState>,
    depth: usize,
    position: Option<usize>,
    entry: FileEntry,
) {
    if let Some(position) = current_context_position(state, depth, position, &entry) {
        state.browser.select(depth, position);
        let weak = Rc::downgrade(state);
        // Selection queues collection focus; enter the editor after it settles.
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.begin_rename();
            }
        });
        return;
    }
    state.begin_search_result_rename(depth, &entry);
}

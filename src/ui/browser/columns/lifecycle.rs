// SPDX-License-Identifier: MIT

//! Rebuilding, truncating and animating columns in and out of the strip.

use super::{
    COLUMN_TRANSITION, ColumnView, cancel_column_spinner, set_column_busy, set_column_selections,
    set_filter_placeholder, stop_column_spinner, touch_source_model,
    update_empty_trash_sensitivity,
};
use crate::app::BrowserColumnSnapshot;
use crate::ui::browser::ViewState;
use crate::ui::browser::collection::{cancel_source, detach_collection_view};
use crate::ui::motion::{animations_enabled, emphasized_acceleration};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;
use std::time::Instant;

/// Runs once per column, so its timeout owns the class outright; width
/// animations keep their own generation.
pub(super) fn animate_column_entry(column: &gtk::Box, swapped: bool) {
    if !animations_enabled() {
        return;
    }
    // A swapped-in sibling takes the old column's place, so it only fades.
    let class = if swapped {
        "column-swapping"
    } else {
        "column-entering"
    };
    column.add_css_class(class);
    let column = column.downgrade();
    glib::timeout_add_local_once(COLUMN_TRANSITION, move || {
        if let Some(column) = column.upgrade() {
            column.remove_css_class(class);
        }
    });
}

fn animate_column_exit(state: &Rc<ViewState>, column: ColumnView, animation_id: u64) {
    let ColumnView {
        shell,
        marquee,
        animation_generation: generation,
        ..
    } = column;
    let band = marquee.band();
    shell.add_css_class("column-exiting");
    // Its depth may already belong to a new column, so it must not take clicks or drops.
    shell.set_can_target(false);
    let weak = Rc::downgrade(state);
    // Width and opacity are driven from the same tick (not a separate CSS
    // keyframe) so they never drift out of lockstep.
    let start_width = shell.width().max(shell.width_request()).max(0);
    let started = Instant::now();
    let weak_shell = shell.downgrade();
    let _tick = shell.add_tick_callback(move |_, _| {
        if generation.get() != animation_id {
            return glib::ControlFlow::Break;
        }
        let Some(widget) = weak_shell.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let progress =
            (started.elapsed().as_secs_f64() / COLUMN_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = emphasized_acceleration(progress);
        let width = (f64::from(start_width) * (1.0 - eased)).round().max(0.0) as i32;
        widget.set_size_request(width, -1);
        widget.set_opacity(1.0 - eased);
        if progress < 1.0 {
            return glib::ControlFlow::Continue;
        }
        widget.set_size_request(0, -1);
        if let Some(state) = weak.upgrade() {
            state.columns_widget.remove(&widget);
            state.overlay.remove_overlay(&band);
        }
        glib::ControlFlow::Break
    });
}

fn show_loaded_snapshot(column: &ColumnView, snapshot: &BrowserColumnSnapshot) {
    if let Some(message) = snapshot.error.as_deref() {
        column.presentation.show_error(
            &rust_i18n::t!(
                "Unable to read this directory\n%{message}",
                message = message
            ),
            snapshot.administrator.is_some(),
        );
    } else if snapshot.count == 0 {
        column.presentation.show_empty();
    } else {
        column.presentation.show_content();
    }
}

fn show_loading_snapshot(column: &ColumnView, snapshot: &BrowserColumnSnapshot) {
    cancel_column_spinner(column);
    column.spinner.set_visible(true);
    column.spinner.start();
    if snapshot.count > 0 && snapshot.location.is_camera_photo_root() {
        column.presentation.show_content();
        set_column_busy(column, false);
    } else {
        column.presentation.show_loading();
    }
}

/// Restores a rebuilt column to the listing state its snapshot recorded.
fn restore_snapshot(column: &ColumnView, snapshot: BrowserColumnSnapshot) {
    touch_source_model(column);
    column.model.replace(snapshot.count as u32);
    column.entry_count.set(snapshot.count);
    set_filter_placeholder(column, snapshot.count);
    update_empty_trash_sensitivity(column, snapshot.count);
    column.truncated_hint.set_visible(snapshot.truncated);
    column.not_responding.set_visible(snapshot.not_responding);
    let positions = snapshot
        .selected_positions
        .iter()
        .filter_map(|position| column.map.view_position(*position))
        .collect::<Vec<_>>();
    set_column_selections(column, &positions);
    if snapshot.loading {
        show_loading_snapshot(column, &snapshot);
    } else {
        // Rebuilt, already-loaded columns receive no finish event to cancel the timer.
        stop_column_spinner(column);
        show_loaded_snapshot(column, &snapshot);
    }
}

impl ViewState {
    pub(in crate::ui::browser) fn rebuild_columns_from(self: &Rc<Self>, from_depth: usize) {
        self.truncate_for_replacement(from_depth);
        let snapshots = (from_depth..)
            .map_while(|depth| self.browser.column_snapshot(depth))
            .collect::<Vec<_>>();

        for (offset, snapshot) in snapshots.iter().enumerate() {
            self.append_column(from_depth + offset, &snapshot.location);
        }
        for (column, snapshot) in self.columns.borrow().iter().skip(from_depth).zip(snapshots) {
            restore_snapshot(column, snapshot);
        }
    }

    pub(in crate::ui::browser) fn truncate_for_replacement(self: &Rc<Self>, len: usize) {
        self.truncate_impl(len, None);
    }

    pub(in crate::ui::browser) fn truncate(self: &Rc<Self>, len: usize) {
        self.truncate_impl(len, Some(len));
    }

    /// Makes room for a sibling opening at `len`. The column in that slot goes at
    /// once so its replacement fades in where it stood; deeper columns of the old
    /// branch shrink away, so the strip slides instead of snapping.
    pub(in crate::ui::browser) fn swap_columns_from(self: &Rc<Self>, len: usize) {
        let replaced = self.columns.borrow().len() > len;
        self.truncate_impl(len, Some(len.saturating_add(1)));
        if replaced {
            self.swap_slot.set(Some(len));
        }
    }

    /// Ends the peek, hover, context menu and edits that belong to columns past `len`.
    fn release_truncated_state(&self, len: usize) {
        self.swap_slot.set(None);
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
        if self.rename_in_retained_column(len) {
            self.cancel_click_rename();
        } else {
            self.cancel_rename();
        }
        self.cancel_new_entry();
    }

    /// Renaming a folder whose child column is open relocates that column, which
    /// must not end the rename that Tab has meanwhile opened in the parent column.
    fn rename_in_retained_column(&self, len: usize) -> bool {
        let Some(field) = self
            .active_rename
            .borrow()
            .as_ref()
            .map(|active| active.field.clone())
        else {
            return false;
        };
        self.columns
            .borrow()
            .iter()
            .take(len)
            .any(|column| field.is_ancestor(&column.shell))
    }

    fn remove_column(self: &Rc<Self>, column: ColumnView, animate: bool) {
        let animation_id = column.animation_generation.get().saturating_add(1);
        column.animation_generation.set(animation_id);
        column.query_binding.take();
        column.search_session.cancel();
        column.syncing_selection.set(true);
        column.selection.set_model(None::<&gio::ListModel>);
        column.filtered_model.set_model(None::<&gio::ListModel>);
        detach_collection_view(&column.list);
        if animate && animations_enabled() {
            animate_column_exit(self, column, animation_id);
        } else {
            self.columns_widget.remove(&column.shell);
            self.overlay.remove_overlay(&column.marquee.band());
        }
    }

    fn truncate_impl(self: &Rc<Self>, len: usize, animate_from: Option<usize>) {
        self.release_truncated_state(len);
        self.horizontal_scroll_generation
            .set(self.horizontal_scroll_generation.get().saturating_add(1));
        while self.columns.borrow().len() > len {
            let Some(column) = self.columns.borrow_mut().pop() else {
                break;
            };
            let animate = animate_from.is_some_and(|from| self.columns.borrow().len() >= from);
            self.remove_column(column, animate);
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

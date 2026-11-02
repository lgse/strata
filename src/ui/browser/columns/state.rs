// SPDX-License-Identifier: MIT

//! View-state queries and updates spanning every open column.

use super::{scroll_column_to, set_active_path_style, set_column_selections};
use crate::ui::browser::ViewState;
use crate::ui::browser::collection::{ActivePaneFilter, restore_filter_controls};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::Duration;

impl ViewState {
    pub(in crate::ui::browser) fn clear_column_selections(&self) {
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

    pub(in crate::ui::browser) fn capture_active_column_filter(&self) -> ActivePaneFilter {
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

    pub(in crate::ui::browser) fn restore_active_column_filter(&self, filter: &ActivePaneFilter) {
        let Some(depth) = self.browser.active_depth() else {
            return;
        };
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return;
        };
        restore_filter_controls(&column.filter_button, &column.filter_entry, filter);
    }

    /// Reapplies the directory selection to rows that return unselected when a filter ends.
    pub(super) fn resync_column_selection(&self, depth: usize) {
        let Ok(columns) = self.columns.try_borrow() else {
            return;
        };
        let Some(column) = columns.get(depth) else {
            return;
        };
        let positions: Vec<_> = self
            .browser
            .selected_positions(depth)
            .into_iter()
            .filter_map(|position| column.map.view_position(position))
            .collect();
        set_column_selections(column, &positions);
    }

    pub(in crate::ui::browser) fn focus_rebuilt_active_column(&self) {
        if !crate::ui::focus_navigation::browser_can_take_focus(&self.overlay) {
            return;
        }
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
        column.focus_surface();
        let column = column.clone();
        glib::idle_add_local_once(move || {
            // A Ctrl+F, popover or dialog opened since the rebuild keeps focus.
            let root = column.list.root();
            let focused = root.as_ref().and_then(|root| root.focus());
            if focused.is_some_and(|focused| {
                crate::ui::focus_navigation::editable(&focused)
                    || crate::ui::focus_navigation::in_popover(&focused)
            }) || root
                .and_downcast::<gtk::Window>()
                .is_some_and(|window| crate::ui::window::visible_modal_layer(&window).is_some())
            {
                return;
            }
            column.focus_surface();
        });
    }

    pub(in crate::ui::browser) fn refresh_active_path_rows(&self) {
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

    pub(in crate::ui::browser) fn request_columns_folder_counts(&self) {
        if self.mode_views.borrow().mode() != crate::ui::browser_modes::BrowserMode::Columns {
            return;
        }
        let requests = self
            .columns
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, column)| !column.recursive_search_active.get())
            .flat_map(|(depth, column)| {
                column
                    .bound_rows
                    .borrow()
                    .iter()
                    .filter_map(|bound| {
                        let item = bound.item.upgrade()?;
                        let row = bound.row.upgrade()?;
                        let icon = row
                            .first_child()
                            .and_downcast::<crate::ui::thumbnail::ThumbnailSlot>()?;
                        let position = column.map.source_position(item.position())?;
                        let entry = self.browser.entry_at(depth, position)?;
                        (entry.is_directory()
                            && entry.child_count == crate::model::MetadataValue::Unknown)
                            .then_some((depth, position, row, icon, entry.location))
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (depth, position, row, icon, location) in requests {
            crate::ui::thumbnail::request_metadata(
                &icon,
                &row,
                &self.browser,
                depth,
                position,
                location,
                true,
            );
        }
    }

    pub(in crate::ui::browser) fn flash_column_parent(&self, child_depth: usize) {
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
        if let Some((row, _)) = column.shown_row_at(active_pos) {
            row.remove_css_class("flash-active-path");
            row.add_css_class("flash-active-path");
            let weak_row = row.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(420), move || {
                if let Some(row) = weak_row.upgrade() {
                    row.remove_css_class("flash-active-path");
                }
            });
        }
    }

    /// Reopening an open folder lands where opening it did: its column focused and in view.
    pub(in crate::ui::browser) fn activate_column_entry(
        self: &Rc<Self>,
        depth: usize,
        position: usize,
    ) {
        let reopening = self.browser.entry_at(depth, position).is_some_and(|entry| {
            entry.is_directory() && self.browser.is_open_child(depth, &entry.location)
        });
        self.browser.activate(depth, position);
        if !reopening {
            return;
        }
        let shell = self
            .columns
            .borrow()
            .get(depth + 1)
            .map(|column| column.shell.clone());
        if let Some(shell) = shell {
            self.reveal_column(shell);
        }
    }
}

// SPDX-License-Identifier: MIT

//! Revealing entries a transfer, archive or typed request produced once their folder lists them.

use super::MAX_BULK_REVEAL_SELECTION;
use crate::model::Location;
use crate::ui::browser::ViewState;
use crate::ui::browser::columns::prune_missing_search_results;
use crate::ui::browser_modes::BrowserMode;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

impl ViewState {
    /// Only a folder without a monitor needs a reload to list the result.
    pub(super) fn show_archive_result(
        self: &Rc<Self>,
        destination: Option<Location>,
        name: String,
    ) {
        let active = self.browser.active_location();
        let Some(folder) = destination.or_else(|| active.clone()) else {
            return;
        };
        let Some(depth) = self.browser.open_depth(&folder) else {
            return;
        };
        if active.as_ref() == Some(&folder) {
            self.pending_archive_destination.replace(Some(folder));
            self.pending_select.borrow_mut().push(name);
        }
        if self.browser.column_has_monitor(depth) {
            self.reveal_pending_archive_at(depth);
        } else {
            self.browser.retry_column(depth);
        }
    }

    pub(super) fn reveal_pending_archive_at(self: &Rc<Self>, depth: usize) {
        if self
            .pending_archive_destination
            .borrow()
            .as_ref()
            .is_none_or(|destination| self.browser.location_at(depth).as_ref() != Some(destination))
        {
            return;
        }
        let names = self.pending_select.take();
        if names.is_empty() {
            return;
        }
        if self.browser.select_entries_by_name_at(depth, &names) {
            if self.mode_views.borrow().mode() == BrowserMode::Columns {
                self.browser.set_active_column(depth);
            }
            self.reveal_focused_entry();
            self.pending_archive_destination.take();
        } else {
            self.pending_select.borrow_mut().extend(names);
        }
    }

    pub(in crate::ui::browser) fn apply_transfer_reveal(
        self: &Rc<Self>,
        destination: Location,
        mut locations: Vec<Location>,
    ) {
        if locations.len() > MAX_BULK_REVEAL_SELECTION {
            locations.truncate(1);
        }
        if let Some(depth) = self.browser.open_depth(&destination) {
            self.reveal_in_open_folder(depth, destination, locations);
            return;
        }
        let parent_depth = (self.mode_views.borrow().mode() == BrowserMode::Columns
            && self.browser.open_depth(&destination).is_none())
        .then(|| {
            destination
                .parent()
                .and_then(|parent| self.browser.open_depth(&parent))
        })
        .flatten();
        match parent_depth {
            Some(parent_depth) => {
                self.clear_pending_reveal_requests();
                self.browser
                    .descend_revealing(parent_depth, destination, locations);
            }
            None => self.reveal_locations(destination, locations, false),
        }
    }

    fn reveal_in_open_folder(
        self: &Rc<Self>,
        depth: usize,
        destination: Location,
        locations: Vec<Location>,
    ) {
        self.prepare_reveal(&destination);
        if !self.browser.column_has_monitor(depth) {
            self.browser
                .reveal_reloaded_locations(&destination, locations);
            return;
        }
        if self
            .browser
            .reveal_monitored_locations(destination.clone(), &locations)
        {
            self.reveal_focused_entry();
        }
        if !self.browser.lists_every_target(depth, &locations) {
            self.pending_location_selection
                .replace(Some((destination, locations)));
        }
    }

    pub(super) fn complete_transfer_ui(self: &Rc<Self>) {
        self.suppress_scroll_after_drop.set(false);
        if let Some(finished) = self.finished_send_to_completion.take()
            && !finished.progress_shown
        {
            self.show_send_to_success(
                &finished.completion.device_name,
                finished.completion.item_count,
            );
        }
        if let Some((source_depth, destination_depth)) = self.drop_active_depths.replace(None) {
            let weak = Rc::downgrade(self);
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    state.return_focus_to_drop_source(source_depth, destination_depth);
                }
            });
        }
        if let Some(destination) = self.pending_navigate.take() {
            self.browser.navigate(destination);
        }
    }

    fn return_focus_to_drop_source(&self, source_depth: usize, destination_depth: usize) {
        if self.browser.active_depth() == Some(destination_depth)
            && self.browser.location_at(source_depth).is_some()
        {
            self.browser.set_active_column(source_depth);
            self.browser.focus_active();
        }
    }

    pub(super) fn reload_archive_destination(&self, destination: Location) {
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            let depth = (0..self.columns.borrow().len())
                .find(|depth| self.browser.location_at(*depth).as_ref() == Some(&destination));
            if let Some(depth) = depth {
                self.browser.set_active_column(depth);
                self.browser.retry_column(depth);
            } else {
                self.browser.navigate(destination);
            }
        } else if self.browser.active_location().as_ref() == Some(&destination) {
            self.browser.reload_active();
        } else {
            self.browser.navigate(destination);
        }
    }

    fn clear_pending_reveal_requests(&self) {
        self.pending_archive_destination.take();
        self.pending_navigate.take();
        self.pending_select.take();
        self.pending_properties.take();
        self.pending_location_selection.take();
    }

    fn prepare_reveal(&self, directory: &Location) {
        self.clear_pending_reveal_requests();
        if let Some(depth) = self.browser.open_depth(directory) {
            if let Some(column) = self.columns.borrow().get(depth) {
                column.filter_entry.set_text("");
            }
            self.mode_views.borrow().clear_filter(depth);
        }
    }

    pub(in crate::ui::browser) fn reveal_locations(
        self: &Rc<Self>,
        directory: Location,
        targets: Vec<Location>,
        properties: bool,
    ) {
        self.prepare_reveal(&directory);
        if properties {
            self.pending_properties.replace(Some(directory.clone()));
        }
        if !self.browser.reveal_locations(directory, targets) {
            return;
        }
        // In-place selections have no load completion to trigger these actions.
        self.reveal_focused_entry();
        if self.pending_properties.take().is_some()
            && let Some((depth, _, entry)) = self.browser.focused_item()
        {
            self.show_entry_properties_at(entry, depth);
        }
    }

    pub(in crate::ui::browser) fn abandon_deferred_reveal(&self) {
        self.browser.cancel_deferred_reveal();
        self.pending_properties.take();
    }

    pub(super) fn reveal_pending_transfer_at(self: &Rc<Self>, depth: usize) -> bool {
        let (selected, complete) = {
            let pending = self.pending_location_selection.borrow();
            let Some((destination, locations)) = pending.as_ref() else {
                return false;
            };
            if self.browser.location_at(depth).as_ref() != Some(destination) {
                return false;
            }
            (
                self.browser.select_entries_by_location_at(depth, locations),
                self.browser.lists_every_target(depth, locations),
            )
        };
        if complete {
            self.pending_location_selection.take();
        }
        if selected {
            self.reveal_focused_entry();
        }
        selected
    }

    pub(in crate::ui::browser) fn reveal_focused_entry(self: &Rc<Self>) {
        let Some((depth, position, _)) = self.browser.focused_item() else {
            return;
        };
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            self.mode_views
                .borrow()
                .reveal_selected_entry(depth, position);
            return;
        }
        let column = self.columns.borrow().get(depth).cloned();
        let Some(column) = column else {
            return;
        };
        let Some(position) = column.map.view_position(position) else {
            return;
        };
        let rows = column.bound_rows.clone();
        crate::ui::browser::collection::reveal_collection_after_layout(
            column.list.upcast_ref(),
            position,
            Rc::new(move |visit| {
                rows.borrow_mut().retain(|bound| {
                    let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade())
                    else {
                        return false;
                    };
                    visit(item.position(), row.upcast_ref());
                    true
                });
            }),
        );
        self.reveal_column(column.shell);
    }

    pub(in crate::ui::browser) fn prune_stale_search_results(self: &Rc<Self>) {
        let columns = self.columns.borrow().clone();
        for column in &columns {
            let weak = Rc::downgrade(self);
            prune_missing_search_results(column, move || {
                if let Some(state) = weak.upgrade() {
                    state.notify_search_selection_changed();
                }
            });
        }
        self.mode_views.borrow().prune_stale_search_results();
    }
}

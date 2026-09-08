// SPDX-License-Identifier: MIT

use gtk::{gio, prelude::*};

use super::{
    BrowserMode, ModeViews, Pane, pane_holds_keyboard_focus, reconnect_pane_model, replace_entries,
    set_selections, show_count, update_bound_list_metadata,
};
use crate::{
    app::{Browser, BrowserEvent, EntryInsertion, EntrySplice},
    ui::browser::entry_model_value,
};

impl ModeViews {
    pub fn handle(&mut self, event: &BrowserEvent) {
        if self.handle_structure_event(event) {
            return;
        }
        if self.handle_rows_event(event) {
            return;
        }
        if self.handle_loading_event(event) {
            return;
        }
        self.handle_selection_event(event);
    }

    fn handle_structure_event(&mut self, event: &BrowserEvent) -> bool {
        match event {
            BrowserEvent::Reset => {
                self.clear_icons();
                self.clear_list();
            }
            BrowserEvent::ColumnsTruncated { .. } => self.rebuild_active_mode(),
            BrowserEvent::ColumnAdded { depth, .. } => {
                if self.browser.active_depth() == Some(*depth) {
                    self.browser.select_first_on_load(*depth);
                    self.rebuild_active_mode();
                }
            }
            _ => return false,
        }
        true
    }

    fn rebuild_active_mode(&mut self) {
        match self.mode {
            BrowserMode::Columns => {}
            BrowserMode::Icons => self.rebuild_icons(),
            BrowserMode::List => self.rebuild_list(),
        }
    }

    fn handle_rows_event(&self, event: &BrowserEvent) -> bool {
        match event {
            BrowserEvent::EntriesInserted { depth, insertions } => {
                self.update_panes(*depth, |pane| pane.insert_rows(insertions));
            }
            BrowserEvent::EntriesReplaced { depth, count } => {
                self.update_panes(*depth, |pane| pane.replace_rows(&self.browser, *count));
            }
            BrowserEvent::EntriesPublished {
                depth,
                position,
                count,
            } => {
                self.update_panes(*depth, |pane| {
                    pane.publish_rows(&self.browser, *position, *count)
                });
            }
            BrowserEvent::EntriesSpliced { depth, splices, .. } => {
                self.update_panes(*depth, |pane| pane.splice_rows(splices));
            }
            BrowserEvent::MetadataFilled { depth, updates } => {
                if self.mode == BrowserMode::List {
                    self.update_panes(*depth, |pane| update_bound_list_metadata(pane, updates));
                }
            }
            _ => return false,
        }
        true
    }

    fn handle_loading_event(&self, event: &BrowserEvent) -> bool {
        match event {
            BrowserEvent::SortingStarted { depth } => {
                self.update_panes(*depth, Pane::start_sorting)
            }
            BrowserEvent::SortingFinished { depth } => {
                self.update_panes(*depth, Pane::finish_sorting)
            }
            BrowserEvent::ColumnReloaded { depth } => self.update_panes(*depth, Pane::reload_rows),
            BrowserEvent::LoadFinished { depth, truncated } => {
                self.update_panes(*depth, |pane| pane.finish_loading(*truncated));
            }
            BrowserEvent::LoadFailed { depth, message } => {
                self.update_panes(*depth, |pane| pane.fail_loading(message));
            }
            _ => return false,
        }
        true
    }

    fn handle_selection_event(&self, event: &BrowserEvent) {
        match event {
            BrowserEvent::SelectionSetChanged {
                depth,
                positions,
                take_focus,
                ..
            } => {
                self.update_selection(*depth, positions, *take_focus);
            }
            BrowserEvent::FocusChanged { depth, position } => {
                self.update_panes(*depth, |pane| {
                    set_selections(pane, &position.iter().copied().collect::<Vec<_>>())
                });
                self.focus_visible_pane(*depth);
            }
            _ => {}
        }
    }

    fn update_selection(&self, depth: usize, positions: &[usize], take_focus: bool) {
        let view_has_focus = self
            .panes_at(depth)
            .iter()
            .any(|pane| pane_holds_keyboard_focus(pane));
        self.update_panes(depth, |pane| set_selections(pane, positions));
        if take_focus || (view_has_focus && !positions.is_empty()) {
            self.focus_visible_pane(depth);
        }
    }

    fn update_panes(&self, depth: usize, update: impl FnMut(&Pane)) {
        self.panes_at(depth).into_iter().for_each(update);
    }
}

impl Pane {
    fn insert_rows(&self, insertions: &[EntryInsertion]) {
        for insertion in insertions {
            let values: Vec<_> = insertion.entries.iter().map(entry_model_value).collect();
            self.splice_values(insertion.position as u32, 0, &values);
        }
        self.show_count_when_idle();
    }

    fn replace_rows(&self, browser: &Browser, count: usize) {
        if count > 0 {
            self.hide_spinner();
        }
        replace_entries(self, browser, count);
    }

    fn publish_rows(&self, browser: &Browser, position: usize, count: usize) {
        // Finish borrowing authoritative entries before GTK model notifications.
        let values = browser
            .with_entries(
                self.depth,
                position..position.saturating_add(count),
                |entries| entries.iter().map(entry_model_value).collect::<Vec<_>>(),
            )
            .unwrap_or_default();
        self.splice_values(position as u32, 0, &values);
        self.show_count_when_idle();
    }

    fn splice_rows(&self, splices: &[EntrySplice]) {
        for splice in splices {
            let values: Vec<_> = splice.entries.iter().map(entry_model_value).collect();
            self.splice_values(splice.position as u32, splice.removed as u32, &values);
        }
        show_count(self);
    }

    pub(super) fn splice_values(&self, position: u32, removed: u32, values: &[String]) {
        let values: Vec<_> = values.iter().map(String::as_str).collect();
        self.model.splice(position, removed, &values);
    }

    fn show_count_when_idle(&self) {
        if !self.spinner.is_spinning() {
            show_count(self);
        }
    }

    fn hide_spinner(&self) {
        self.spinner.stop();
        self.spinner.set_visible(false);
    }

    fn start_sorting(&self) {
        self.spinner.set_tooltip_text(Some("Sorting…"));
        self.spinner.set_visible(true);
        self.spinner.start();
    }

    fn finish_sorting(&self) {
        self.hide_spinner();
        self.spinner.set_tooltip_text(None);
    }

    fn reload_rows(&self) {
        // Reload disconnects selection/filter models, not the collection views:
        // teardown's detach_pane_models also detaches those views.
        self.detached.set(true);
        for section in self.all_sections() {
            section.syncing.set(true);
            section.selection.set_model(None::<&gio::ListModel>);
        }
        if let Some(filtered) = self.filter_model.as_ref() {
            filtered.set_model(None::<&gio::ListModel>);
        }
        self.model.splice(0, self.model.n_items(), &[]);
        self.truncated_hint.set_visible(false);
        self.spinner.set_visible(true);
        self.spinner.start();
        self.loading.start();
    }

    fn finish_loading(&self, truncated: bool) {
        reconnect_pane_model(self);
        self.hide_spinner();
        self.truncated_hint.set_visible(truncated);
        show_count(self);
    }

    fn fail_loading(&self, message: &str) {
        reconnect_pane_model(self);
        self.spinner.stop();
        self.status
            .set_label(&format!("Unable to read this directory\n{message}"));
        self.status.add_css_class("error");
        self.loading.show("status");
    }
}

#[cfg(test)]
mod tests;

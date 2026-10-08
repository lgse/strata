// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{gio, prelude::*};

use super::{
    BrowserMode, ModeViews, Pane, pane_holds_keyboard_focus, reconnect_pane_model, replace_entries,
    select_all, set_selections, show_count, tree, update_bound_icons_metadata,
    update_bound_list_metadata,
};
use crate::{
    app::{Browser, BrowserEvent, EntryInsertion, EntrySplice, SelectionUpdate},
    ui::browser::entry_model_value,
};

impl ModeViews {
    pub(crate) fn handle_with_deferred_empty(&mut self, event: &BrowserEvent, defer_empty: bool) {
        if self.handle_structure_event(event) {
            return;
        }
        if self.handle_rows_event(event, defer_empty) {
            return;
        }
        if self.handle_loading_event(event, defer_empty) {
            return;
        }
        self.handle_selection_event(event);
    }

    fn handle_structure_event(&mut self, event: &BrowserEvent) -> bool {
        match event {
            BrowserEvent::NavigationStarting => {
                if self.mode == BrowserMode::List
                    && let Some(pane) = self.list_pane.as_ref()
                {
                    self.list_navigation
                        .borrow_mut()
                        .capture(pane, &self.browser);
                }
            }
            BrowserEvent::Reset => {
                self.clear_icons();
                self.clear_list();
                self.clear_tree();
            }
            BrowserEvent::ColumnsTruncated { .. } => self.rebuild_active_mode(),
            BrowserEvent::ColumnsRelocated { from_depth } => {
                if self.is_tree_active() {
                    if *from_depth == 0 {
                        self.rebuild_tree();
                    }
                    return true;
                }
                if let Some(depth) = self
                    .browser
                    .active_depth()
                    .filter(|depth| depth >= from_depth)
                {
                    let refocus = self
                        .panes_at(depth)
                        .iter()
                        .any(|pane| pane_holds_keyboard_focus(pane));
                    self.rebuild_active_mode();
                    if refocus {
                        self.focus_visible_pane(depth);
                    }
                }
            }
            BrowserEvent::ColumnAdded { depth, .. } => {
                if self.browser.active_depth() == Some(*depth) {
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
            BrowserMode::Tree => self.rebuild_tree(),
        }
    }

    fn handle_rows_event(&self, event: &BrowserEvent, defer_empty: bool) -> bool {
        match event {
            BrowserEvent::EntriesInserted { depth, insertions } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    for insertion in insertions {
                        tree.insert_root(
                            insertion.position,
                            insertion
                                .entries
                                .iter()
                                .map(|entry| Rc::new(entry.clone()))
                                .collect(),
                        );
                    }
                }
                let camera = self
                    .browser
                    .location_at(*depth)
                    .is_some_and(|location| location.is_camera_photo_root());
                self.update_panes(*depth, |pane| {
                    let top = camera
                        .then(|| {
                            crate::ui::browser::camera_scroll::CameraTopAnchor::capture(
                                &pane.section.view,
                            )
                        })
                        .flatten();
                    pane.insert_rows(insertions);
                    if camera && pane.model.n_items() > 0 {
                        reconnect_pane_model(pane);
                        for section in pane.all_sections() {
                            section.syncing.set(false);
                        }
                        show_count(pane);
                    }
                    if let Some(top) = top {
                        top.restore();
                    }
                });
            }
            BrowserEvent::EntriesReplaced { depth, count } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    tree.replace_root_from_browser(&self.browser, *count);
                }
                self.update_panes(*depth, |pane| {
                    pane.replace_rows(&self.browser, *count, defer_empty)
                });
            }
            BrowserEvent::EntriesPublished {
                depth,
                position,
                count,
            } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    let entries = self
                        .browser
                        .with_entries(
                            *depth,
                            *position..position.saturating_add(*count),
                            |entries| {
                                entries
                                    .iter()
                                    .map(|entry| Rc::new(entry.clone()))
                                    .collect::<Vec<_>>()
                            },
                        )
                        .unwrap_or_default();
                    tree.insert_root(*position, entries);
                }
                self.update_panes(*depth, |pane| {
                    pane.publish_rows(&self.browser, *position, *count)
                });
            }
            BrowserEvent::EntriesSpliced { depth, splices, .. } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    for splice in splices {
                        tree.splice_root(
                            splice.position,
                            splice.removed,
                            splice
                                .entries
                                .iter()
                                .map(|entry| Rc::new(entry.clone()))
                                .collect(),
                        );
                    }
                }
                let restore_cursor = self
                    .panes_at(*depth)
                    .iter()
                    .any(|pane| pane_holds_keyboard_focus(pane));
                let positions = self.browser.selected_positions(*depth);
                self.update_panes(*depth, |pane| {
                    pane.splice_rows(splices, defer_empty);
                    set_selections(pane, &positions);
                });
                if restore_cursor && !positions.is_empty() && !self.rename_is_active() {
                    let target = self.browser.focused_item().map(|(_, position, _)| position);
                    let missing_cursor = !self.panes_at(*depth).iter().any(|pane| {
                        pane.item_sections().iter().any(|section| {
                            let Some(focused) = section.view.root().and_then(|root| root.focus())
                            else {
                                return false;
                            };
                            if self.mode == BrowserMode::Icons && focused.is_ancestor(&section.view)
                            {
                                return true;
                            }
                            section.bound_items.borrow().iter().any(|bound| {
                                let Some(item) = bound.item.upgrade() else {
                                    return false;
                                };
                                let source = item
                                    .item()
                                    .and_then(|item| pane.source_index.of_item(&item));
                                source == target
                                    && bound
                                        .widget
                                        .upgrade()
                                        .and_then(|widget| widget.parent())
                                        .as_ref()
                                        == Some(&focused)
                            })
                        })
                    });
                    if missing_cursor {
                        self.suppress_focus_scroll();
                        self.focus_visible_pane(*depth);
                    }
                }
            }
            BrowserEvent::MetadataFilled { depth, updates } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    tree.apply_metadata(updates);
                }
                if self.mode == BrowserMode::List {
                    self.update_panes(*depth, |pane| update_bound_list_metadata(pane, updates));
                } else if self.mode == BrowserMode::Icons {
                    self.update_panes(*depth, |pane| update_bound_icons_metadata(pane, updates));
                }
            }
            _ => return false,
        }
        true
    }

    fn handle_loading_event(&mut self, event: &BrowserEvent, defer_empty: bool) -> bool {
        if let BrowserEvent::SortingFinished { depth } | BrowserEvent::ColumnReloaded { depth } =
            event
        {
            let preferences = self.browser.column_preferences(*depth);
            self.update_panes(*depth, |pane| {
                if let Some(button) = &pane.sort_direction_button {
                    super::super::browser::sync_column_sort_direction(
                        &self.browser,
                        *depth,
                        button,
                    );
                }
                if let (Some(sorting), Some(preferences)) = (&pane.sorting, preferences) {
                    sorting.show(preferences.sort_key, preferences.sort_direction);
                }
            });
        }
        let changed_depth = match event {
            BrowserEvent::ColumnReloaded { depth }
            | BrowserEvent::SortingFinished { depth }
            | BrowserEvent::LoadFinished { depth, .. }
            | BrowserEvent::LoadFailed { depth, .. } => Some(*depth),
            _ => None,
        };
        if let Some(depth) = changed_depth
            && self.mode == BrowserMode::List
            && let Some(snapshot) = self.browser.column_snapshot(depth)
            && self.list_pane.as_ref().is_some_and(|pane| {
                pane.depth == depth
                    && pane.group_by_type != self.grouping_for_snapshot(depth, &snapshot)
            })
        {
            self.update_camera_grouping(self.grouping_for_snapshot(depth, &snapshot));
        }
        match event {
            BrowserEvent::SortingStarted { depth } => {
                self.update_panes(*depth, Pane::start_sorting)
            }
            BrowserEvent::SortingFinished { depth } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    tree.resort_branches();
                }
                self.update_panes(*depth, Pane::finish_sorting)
            }
            BrowserEvent::ColumnReloaded { depth } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    let count = self
                        .browser
                        .column_snapshot(*depth)
                        .map(|snapshot| snapshot.count)
                        .unwrap_or_default();
                    tree.replace_root_from_browser(&self.browser, count);
                    tree.resort_branches();
                }
                self.update_panes(*depth, Pane::reload_rows)
            }
            BrowserEvent::LoadFinished { depth, truncated } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    tree.update_status();
                }
                let restore_cursor = self
                    .panes_at(*depth)
                    .iter()
                    .any(|pane| pane.stack.is_focus());
                let positions = self.browser.selected_positions(*depth);
                self.update_panes(*depth, |pane| {
                    pane.finish_loading(*truncated, defer_empty, &positions)
                });
                if self.mode == BrowserMode::List
                    && let Some(pane) = self.list_pane.as_ref().filter(|pane| pane.depth == *depth)
                {
                    self.list_navigation
                        .borrow_mut()
                        .restore(pane, &self.browser);
                }
                if restore_cursor {
                    self.focus_visible_pane(*depth);
                }
            }
            BrowserEvent::LoadFailed { depth, message } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    tree.load_failed(message);
                }
                self.update_panes(*depth, |pane| pane.fail_loading(message));
                if self
                    .list_pane
                    .as_ref()
                    .is_some_and(|pane| pane.depth == *depth)
                {
                    self.list_navigation.borrow_mut().cancel();
                }
            }
            _ => return false,
        }
        true
    }

    fn update_camera_grouping(&mut self, grouped: bool) {
        let Some(pane) = self.list_pane.as_mut() else {
            return;
        };
        let Some(list) = pane.section.view.downcast_ref::<gtk::ListView>() else {
            return;
        };
        let Some(model) = pane.section.view_model.downcast_ref::<gtk::SortListModel>() else {
            return;
        };
        let top = crate::ui::browser::camera_scroll::CameraTopAnchor::capture(&pane.section.view);
        let was_syncing = pane.section.syncing.replace(true);
        // Remove section widgets before changing their model. Re-enable them only
        // after the complete camera snapshot is sorted. Keep the existing view,
        // selection model and both scrollers instead of resetting the viewport.
        list.set_header_factory(None::<&gtk::ListItemFactory>);
        let sorter =
            grouped.then(|| super::pane_type_group_sorter(&self.browser, pane.depth, &pane.model));
        model.set_section_sorter(sorter.as_ref());
        model.set_sorter(sorter.as_ref());
        if grouped {
            list.set_header_factory(Some(&super::type_group_header_factory()));
        }
        pane.group_by_type = grouped;
        pane.section.syncing.set(was_syncing);
        if let Some(top) = top {
            top.restore();
        }
    }

    fn handle_selection_event(&self, event: &BrowserEvent) {
        if self.mode == BrowserMode::List && self.list_navigation.borrow().is_restoring() {
            return;
        }
        match event {
            BrowserEvent::SelectionSetChanged {
                depth,
                selection,
                take_focus,
                ..
            } => {
                if let Some(tree) = self.tree_for_depth(*depth) {
                    match selection {
                        SelectionUpdate::All => {
                            tree.sync_root_selection(&self.browser.selected_positions(*depth))
                        }
                        SelectionUpdate::Positions(positions) => {
                            tree.sync_root_selection(positions)
                        }
                    }
                    if *take_focus
                        && let Some(position) = self.browser.selected_positions(*depth).first()
                    {
                        tree.focus_source_row(*position);
                    }
                }
                self.update_selection(*depth, selection, *take_focus);
            }
            BrowserEvent::FocusChanged { depth, .. } => {
                if let Some(tree) = self.tree_for_depth(*depth)
                    && !self.cursor_keeps_focus.get()
                    && let Some((_, position, _)) = self.browser.focused_item()
                {
                    tree.focus_source_row(position);
                }
                let positions = self.browser.selected_positions(*depth);
                self.update_panes(*depth, |pane| set_selections(pane, &positions));
                if !self.cursor_keeps_focus.get() {
                    self.focus_visible_pane(*depth);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn show_empty_if_empty(&self, depth: usize) {
        if let Some(tree) = self.tree_for_depth(depth) {
            tree.update_status();
        }
        self.update_panes(depth, |pane| {
            let showing_error = pane.stack.visible_child_name().as_deref() == Some("status")
                && pane.status.has_css_class("error");
            if pane.model.n_items() == 0 && !pane.spinner.is_spinning() && !showing_error {
                show_count(pane);
            }
        });
    }

    fn update_selection(&self, depth: usize, selection: &SelectionUpdate, take_focus: bool) {
        let view_has_focus = self
            .panes_at(depth)
            .iter()
            .any(|pane| pane_holds_keyboard_focus(pane));
        let has_selection = match selection {
            SelectionUpdate::All => {
                self.update_panes(depth, select_all);
                true
            }
            SelectionUpdate::Positions(positions) => {
                self.update_panes(depth, |pane| set_selections(pane, positions));
                !positions.is_empty()
            }
        };
        let camera_loading = self
            .browser
            .column_snapshot(depth)
            .is_some_and(|snapshot| snapshot.loading && snapshot.location.is_camera_photo_root());
        // Incoming photos shift source positions without a user selection change.
        // Re-focusing on every such update pulls scrolling back to the selected row.
        if take_focus || (view_has_focus && has_selection && !camera_loading) {
            self.focus_visible_pane(depth);
        }
    }

    fn update_panes(&self, depth: usize, update: impl FnMut(&Pane)) {
        self.panes_at(depth).into_iter().for_each(update);
    }

    /// The tree pane when it mirrors this depth. The tree root always tracks
    /// depth 0; nested branches load outside the column model.
    fn tree_for_depth(&self, depth: usize) -> Option<tree::TreePane> {
        (self.is_tree_active() && depth == 0)
            .then(|| self.tree_pane.clone())
            .flatten()
    }

    fn clear_tree(&self) {
        if let Some(tree) = self.tree_pane.as_ref() {
            tree.set_root(None, Vec::new());
        }
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

    fn replace_rows(&self, browser: &Browser, count: usize, defer_empty: bool) {
        if count > 0 {
            self.hide_spinner();
        }
        replace_entries(self, browser, count);
        self.show_count_after_update(defer_empty);
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

    fn splice_rows(&self, splices: &[EntrySplice], defer_empty: bool) {
        for splice in splices {
            let values: Vec<_> = splice.entries.iter().map(entry_model_value).collect();
            self.splice_values(splice.position as u32, splice.removed as u32, &values);
        }
        self.show_count_after_update(defer_empty);
    }

    fn show_count_after_update(&self, defer_empty: bool) {
        if defer_empty && self.model.n_items() == 0 {
            if let Some(button) = &self.empty_trash_button {
                button.set_sensitive(false);
            }
            return;
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
        crate::ui::accessibility::set_description(&self.spinner, Some("Sorting…"));
        self.spinner.set_visible(true);
        self.spinner.start();
    }

    fn finish_sorting(&self) {
        self.hide_spinner();
        crate::ui::accessibility::set_description(&self.spinner, None);
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

    fn finish_loading(&self, truncated: bool, defer_empty: bool, positions: &[usize]) {
        reconnect_pane_model(self);
        set_selections(self, positions);
        for section in self.all_sections() {
            section.syncing.set(false);
        }
        self.hide_spinner();
        self.truncated_hint.set_visible(truncated);
        self.show_count_after_update(defer_empty);
    }

    fn fail_loading(&self, message: &str) {
        reconnect_pane_model(self);
        for section in self.all_sections() {
            section.syncing.set(false);
        }
        self.spinner.stop();
        self.status
            .set_label(&format!("Unable to read this directory\n{message}"));
        self.status.add_css_class("error");
        self.loading.show("status");
    }
}

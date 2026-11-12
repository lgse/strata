// SPDX-License-Identifier: MIT

use gtk::{gio, prelude::*};

use super::{
    BrowserMode, ModeViews,
    focus::{pane_contains_focus, reload_focus_fell_back},
    metadata::{update_bound_icons_metadata, update_bound_list_metadata},
    navigation::{FocusOwner, RestoreFocus},
    pane::{
        BoundModeItem, Pane, PaneSection, STATUS_PAGE, reconnect_pane_model, replace_entries,
        show_count,
    },
    selection::{select_all, set_selections},
    type_groups::{pane_type_group_sorter, type_group_header_factory},
};
use crate::{
    app::{Browser, BrowserEvent, EntryInsertion, EntrySplice, SelectionUpdate},
    model::FileEntry,
    ui::browser::{camera_scroll::CameraTopAnchor, entry_model_value},
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
            BrowserEvent::NavigationStarting { history } => self.on_navigation_starting(*history),
            BrowserEvent::ColumnReloading { depth } => self.on_column_reloading(*depth),
            BrowserEvent::Reset => {
                self.clear_icons();
                self.clear_list();
            }
            BrowserEvent::ColumnsTruncated { .. } => self.rebuild_active_mode(),
            BrowserEvent::ColumnsRelocated { from_depth } => self.on_columns_relocated(*from_depth),
            BrowserEvent::ColumnAdded { depth, .. } => {
                if self.browser.active_depth() == Some(*depth) {
                    self.pane_navigation.borrow_mut().arrive();
                    self.rebuild_active_mode();
                    self.pane_navigation.borrow_mut().arrived();
                }
            }
            _ => return false,
        }
        true
    }

    fn on_navigation_starting(&self, history: bool) {
        let pane = self.single_pane();
        // An explicit navigation focuses the new listing, as a first visit does.
        // Back, Forward and Up leave focus on a header button or the sidebar, but
        // focus inside the pane, its filter included, goes away with it.
        let takes_focus = !history
            || pane.is_none_or(|pane| {
                pane_contains_focus(pane) || self.listing_may_take_focus(pane.depth)
            });
        let mut navigation = self.pane_navigation.borrow_mut();
        navigation.leave(takes_focus);
        if let Some(pane) = pane {
            navigation.capture(pane, &self.browser, self.mode);
        }
    }

    fn on_column_reloading(&self, depth: usize) {
        if let Some(pane) = self.mode_pane(depth) {
            let focus = self.focus_owner(pane);
            self.pane_navigation
                .borrow_mut()
                .capture_reload(pane, &self.browser, self.mode, focus);
        }
    }

    fn on_columns_relocated(&mut self, from_depth: usize) {
        let Some(depth) = self
            .browser
            .active_depth()
            .filter(|depth| *depth >= from_depth)
        else {
            return;
        };
        // The rebuild replaces the panes, including a focused filter field.
        let refocus = self
            .panes_at(depth)
            .iter()
            .any(|pane| pane_contains_focus(pane));
        self.rebuild_active_mode();
        if refocus {
            self.focus_visible_pane(depth);
        }
    }

    fn rebuild_active_mode(&mut self) {
        match self.mode {
            BrowserMode::Columns => {}
            BrowserMode::Icons => self.rebuild_icons(),
            BrowserMode::List => self.rebuild_list(),
            BrowserMode::Compact => self.rebuild_compact(),
        }
    }

    fn handle_rows_event(&self, event: &BrowserEvent, defer_empty: bool) -> bool {
        match event {
            BrowserEvent::EntriesInserted { depth, insertions } => {
                self.on_entries_inserted(*depth, insertions);
            }
            BrowserEvent::EntriesReplaced { depth, count } => {
                self.update_panes(*depth, |pane| {
                    pane.replace_rows(&self.browser, *count, defer_empty)
                });
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
                self.on_entries_spliced(*depth, splices, defer_empty);
            }
            BrowserEvent::MetadataFilled { depth, updates } => {
                self.on_metadata_filled(*depth, updates);
            }
            _ => return false,
        }
        true
    }

    fn on_entries_inserted(&self, depth: usize, insertions: &[EntryInsertion]) {
        let camera = self
            .browser
            .location_at(depth)
            .is_some_and(|location| location.is_camera_photo_root());
        self.update_panes(depth, |pane| pane.insert_rows_at_top(insertions, camera));
    }

    fn on_entries_spliced(&self, depth: usize, splices: &[EntrySplice], defer_empty: bool) {
        let restore_cursor = self.listing_holds_focus(depth);
        let positions = self.browser.selected_positions(depth);
        self.update_panes(depth, |pane| {
            pane.splice_rows(splices, defer_empty);
            set_selections(pane, &positions);
        });
        let keeps_cursor = restore_cursor && !positions.is_empty();
        if !keeps_cursor || self.rename_is_active() {
            return;
        }
        if self.cursor_missing(depth) {
            self.suppress_focus_scroll();
            self.focus_visible_pane(depth);
        }
    }

    /// No view of `depth` still has focus on the browser's cursor row.
    fn cursor_missing(&self, depth: usize) -> bool {
        let target = self.browser.focused_item().map(|(_, position, _)| position);
        !self.panes_at(depth).iter().any(|pane| {
            pane.item_sections()
                .iter()
                .any(|section| self.section_holds_cursor(pane, section, target))
        })
    }

    fn section_holds_cursor(
        &self,
        pane: &Pane,
        section: &PaneSection,
        target: Option<usize>,
    ) -> bool {
        let Some(focused) = section.view.root().and_then(|root| root.focus()) else {
            return false;
        };
        if self.mode.is_grid() && focused.is_ancestor(&section.view) {
            return true;
        }
        section
            .bound_items
            .borrow()
            .iter()
            .any(|bound| bound_holds_cursor(pane, bound, target, &focused))
    }

    fn on_metadata_filled(&self, depth: usize, updates: &[(usize, FileEntry)]) {
        match self.mode {
            BrowserMode::List => {
                self.update_panes(depth, |pane| update_bound_list_metadata(pane, updates));
            }
            BrowserMode::Icons | BrowserMode::Compact => {
                self.update_panes(depth, |pane| update_bound_icons_metadata(pane, updates));
            }
            BrowserMode::Columns => {}
        }
    }

    fn handle_loading_event(&mut self, event: &BrowserEvent, defer_empty: bool) -> bool {
        if let BrowserEvent::SortingFinished { depth } | BrowserEvent::ColumnReloaded { depth } =
            event
        {
            self.refresh_sort_indicators(*depth);
        }
        if let Some(depth) = changed_depth(event) {
            self.follow_camera_grouping(depth);
        }
        match event {
            BrowserEvent::ColumnNotResponding {
                depth,
                not_responding,
            } => self.update_panes(*depth, |pane| {
                pane.not_responding.set_visible(*not_responding);
            }),
            BrowserEvent::SortingStarted { depth } => {
                self.update_panes(*depth, Pane::start_sorting)
            }
            BrowserEvent::SortingFinished { depth } => {
                self.update_panes(*depth, Pane::finish_sorting)
            }
            BrowserEvent::ColumnReloaded { depth } => self.on_column_reloaded(*depth),
            BrowserEvent::LoadFinished { depth, truncated } => {
                self.on_load_finished(*depth, *truncated, defer_empty);
            }
            BrowserEvent::LoadFailed {
                depth,
                message,
                administrator,
            } => self.on_load_failed(*depth, message, administrator.is_some()),
            _ => return false,
        }
        true
    }

    fn refresh_sort_indicators(&self, depth: usize) {
        let preferences = self.browser.column_preferences(depth);
        self.update_panes(depth, |pane| {
            if let Some(button) = &pane.sort_direction_button {
                crate::ui::browser::sync_column_sort_direction(&self.browser, depth, button);
            }
            if let (Some(sorting), Some(preferences)) = (&pane.sorting, preferences) {
                sorting.show(preferences.sort_key, preferences.sort_direction);
            }
            if let Some(button) = &pane.sort_menu {
                crate::ui::browser::sync_list_sort_menu(&self.browser, depth, button);
            }
            if let Some(sorter) = &pane.type_group_sorter {
                sorter.refresh(preferences);
            }
        });
    }

    /// A camera folder groups its List rows only once the whole snapshot is in.
    fn follow_camera_grouping(&mut self, depth: usize) {
        if self.mode != BrowserMode::List {
            return;
        }
        let grouped = self.browser.shows_type_groups(depth);
        let stale = self
            .list_pane
            .as_ref()
            .is_some_and(|pane| pane.depth == depth && pane.group_by_type != grouped);
        if stale {
            self.update_camera_grouping(grouped);
        }
    }

    fn on_column_reloaded(&self, depth: usize) {
        // A reload that reveals a target is not announced; the target wins.
        let reveals_target = self.mode_pane(depth).is_some()
            && self
                .browser
                .column_snapshot(depth)
                .is_some_and(|snapshot| snapshot.reveal_pending);
        if reveals_target {
            self.pane_navigation.borrow_mut().cancel();
        }
        self.update_panes(depth, Pane::reload_rows);
    }

    fn on_load_finished(&self, depth: usize, truncated: bool, defer_empty: bool) {
        let restore_cursor = self
            .panes_at(depth)
            .iter()
            .any(|pane| pane.stack.is_focus());
        let positions = self.browser.selected_positions(depth);
        self.update_panes(depth, |pane| {
            pane.finish_loading(truncated, defer_empty, &positions)
        });
        let first_visit = self
            .pane_navigation
            .borrow_mut()
            .take_untouched_first_visit(depth);
        // After the rows are back, so focus parked on the pane surface counts.
        let restored = self.mode_pane(depth).and_then(|pane| {
            let allowed = self.restore_focus_allowed(pane, depth);
            self.pane_navigation
                .borrow_mut()
                .restore(pane, &self.browser, self.mode, allowed)
        });
        if restored.is_some() {
            return;
        }
        if first_visit && self.grouped_focus_drifted(depth) {
            self.focus_first_row(depth);
        } else if restore_cursor {
            self.focus_visible_pane(depth);
        }
    }

    /// Rows publish in listing order, so a group that sorts first can arrive above the
    /// row GTK focused when the first rows came in. Without a cursor to hold, as after
    /// a pointer opened the folder, focus goes back to the first row shown.
    fn grouped_focus_drifted(&self, depth: usize) -> bool {
        let has_cursor = self
            .browser
            .focused_item()
            .is_some_and(|(focused_depth, _, _)| focused_depth == depth);
        !has_cursor
            && self.mode_pane(depth).is_some_and(|pane| pane.group_by_type)
            && self.listing_holds_focus(depth)
    }

    fn restore_focus_allowed(&self, pane: &Pane, depth: usize) -> RestoreFocus {
        // A hidden tab never takes focus.
        if self.rename_is_active() || !pane.shell.is_mapped() {
            return RestoreFocus::default();
        }
        RestoreFocus {
            listing: self.listing_may_take_focus(depth),
            filter_entry: self.focus_owner(pane) == FocusOwner::FilterEntry
                || reload_focus_fell_back(pane),
        }
    }

    fn on_load_failed(&self, depth: usize, message: &str, offers_administrator: bool) {
        self.update_panes(depth, |pane| {
            pane.fail_loading(message, offers_administrator);
        });
        if self.mode_pane(depth).is_some() {
            self.pane_navigation.borrow_mut().cancel();
        }
    }

    fn update_camera_grouping(&mut self, grouped: bool) {
        let Some(pane) = self.list_pane.as_mut() else {
            return;
        };
        let Some(list) = pane.section.view.downcast_ref::<gtk::ListView>() else {
            return;
        };
        let Some(model) = pane
            .section
            .view_model
            .downcast_ref::<super::grouped_model::GroupedModel>()
        else {
            return;
        };
        let top = CameraTopAnchor::capture(&pane.section.view);
        let was_syncing = pane.section.syncing.replace(true);
        // Remove section widgets before changing their model. Re-enable them only
        // after the complete camera snapshot is sorted. Keep the existing view,
        // selection model and both scrollers instead of resetting the viewport.
        list.set_header_factory(None::<&gtk::ListItemFactory>);
        let sorter = grouped.then(|| pane_type_group_sorter(&self.browser, pane.depth));
        model.set_sorter(sorter.as_ref().map(|sorter| &sorter.sorter));
        pane.type_group_sorter = sorter;
        if grouped {
            list.set_header_factory(Some(&type_group_header_factory()));
        }
        pane.group_by_type = grouped;
        pane.section.syncing.set(was_syncing);
        if let Some(top) = top {
            top.restore();
        }
    }

    fn handle_selection_event(&self, event: &BrowserEvent) {
        let restoring =
            self.mode != BrowserMode::Columns && self.pane_navigation.borrow().is_restoring();
        if restoring && self.restore_absorbs(event) {
            return;
        }
        match event {
            BrowserEvent::SelectionSetChanged {
                depth,
                selection,
                take_focus,
                ..
            } => {
                self.update_selection(*depth, selection, *take_focus);
            }
            BrowserEvent::FocusChanged { depth, .. } => self.on_focus_changed(*depth),
            _ => {}
        }
    }

    /// Whether a pending viewport restore consumes `event`. A restore that is
    /// cancelled here lets the event through.
    fn restore_absorbs(&self, event: &BrowserEvent) -> bool {
        let (takes_focus, settling) = {
            let navigation = self.pane_navigation.borrow();
            (navigation.history_takes_focus(), navigation.is_settling())
        };
        let depth = match event {
            BrowserEvent::FocusChanged { depth, .. } => *depth,
            // Rows that change after the load, a re-sort for one, outdate the saved
            // viewport.
            BrowserEvent::SelectionSetChanged { .. } if settling => {
                self.pane_navigation.borrow_mut().cancel();
                return false;
            }
            _ => return true,
        };
        if takes_focus {
            // The rebuilt listing takes over focus from the one being left, as it
            // does when there is no position to restore.
            if self.may_refocus(depth) {
                self.focus_visible_pane(depth);
            }
            return true;
        }
        // A restore that left focus elsewhere must not swallow a later request to
        // focus the listing; one that holds focus keeps its viewport.
        if !settling || self.listing_holds_focus(depth) {
            return true;
        }
        self.pane_navigation.borrow_mut().cancel();
        false
    }

    fn on_focus_changed(&self, depth: usize) {
        let positions = self.browser.selected_positions(depth);
        self.update_panes(depth, |pane| set_selections(pane, &positions));
        if !self.may_refocus(depth) {
            return;
        }
        // A result rename keeps focus, as `focus_visible_pane` keeps a listing's.
        let results = !self.rename_is_active()
            && self
                .visible_panes()
                .into_iter()
                .any(|pane| pane.depth == depth && pane.search.hits().is_some());
        if results {
            self.results_take_focus.set(Some(depth));
        } else {
            self.focus_visible_pane(depth);
        }
    }

    /// A background change may move focus to the listing's cursor.
    fn may_refocus(&self, depth: usize) -> bool {
        !self.cursor_keeps_focus.get()
            && !self.outside_change_keeps_focus()
            && self.background_load_may_take_focus(depth)
    }

    fn background_load_may_take_focus(&self, depth: usize) -> bool {
        !self.browser.focus_follows_background_load() || self.listing_may_take_focus(depth)
    }

    fn outside_change_keeps_focus(&self) -> bool {
        self.browser.focus_follows_external_change()
            && (self.filter_focus().is_some()
                || self
                    .context_state
                    .borrow()
                    .as_ref()
                    .and_then(std::rc::Weak::upgrade)
                    .is_some_and(|state| state.footer_filter_has_focus()))
    }

    pub(crate) fn show_empty_if_empty(&self, depth: usize) {
        self.update_panes(depth, |pane| {
            let showing_error = pane.stack.visible_child_name().as_deref() == Some(STATUS_PAGE)
                && pane.status.has_css_class("error");
            let idle_and_empty = pane.model.n_items() == 0 && !pane.spinner.is_spinning();
            if idle_and_empty && !showing_error {
                show_count(pane);
            }
        });
    }

    fn update_selection(&self, depth: usize, selection: &SelectionUpdate, take_focus: bool) {
        let view_has_focus = self.listing_holds_focus(depth);
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
        let follows_selection = view_has_focus && has_selection && !camera_loading;
        if take_focus || follows_selection {
            self.focus_visible_pane(depth);
        }
    }

    fn update_panes(&self, depth: usize, update: impl FnMut(&Pane)) {
        self.panes_at(depth).into_iter().for_each(update);
    }
}

fn changed_depth(event: &BrowserEvent) -> Option<usize> {
    match event {
        BrowserEvent::ColumnReloaded { depth }
        | BrowserEvent::SortingFinished { depth }
        | BrowserEvent::LoadFinished { depth, .. }
        | BrowserEvent::LoadFailed { depth, .. } => Some(*depth),
        _ => None,
    }
}

/// The bound row of the cursor's entry is the one GTK focused.
fn bound_holds_cursor(
    pane: &Pane,
    bound: &BoundModeItem,
    target: Option<usize>,
    focused: &gtk::Widget,
) -> bool {
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
            == Some(focused)
}

impl Pane {
    /// Camera folders keep the top row in place while photos arrive.
    fn insert_rows_at_top(&self, insertions: &[EntryInsertion], camera: bool) {
        let top = camera
            .then(|| CameraTopAnchor::capture(&self.section.view))
            .flatten();
        self.insert_rows(insertions);
        if camera && self.model.n_items() > 0 {
            reconnect_pane_model(self);
            for section in self.all_sections() {
                section.syncing.set(false);
            }
            show_count(self);
        }
        if let Some(top) = top {
            top.restore();
        }
    }

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
        crate::ui::accessibility::set_description(
            &self.spinner,
            Some(&crate::i18n::tr("Sorting…")),
        );
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

    fn fail_loading(&self, message: &str, offers_administrator: bool) {
        reconnect_pane_model(self);
        for section in self.all_sections() {
            section.syncing.set(false);
        }
        self.spinner.stop();
        super::pane_state::show_load_error(self, message, offers_administrator);
    }
}

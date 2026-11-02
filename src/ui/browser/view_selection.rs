// SPDX-License-Identifier: MIT

//! Selection, visual ranges, displayed-order cursor moves and search-hit selection.

use super::*;

/// Focus is in the column's filter field or its listing.
fn column_filter_holds_focus(column: &ColumnView, focused: &gtk::Widget) -> bool {
    let in_filter = focused.is_ancestor(&column.filter_entry)
        || *focused == column.filter_entry.clone().upcast::<gtk::Widget>();
    let in_list = focused.is_ancestor(&column.list)
        || *focused == column.list.clone().upcast::<gtk::Widget>();
    in_filter || in_list
}

impl BrowserView {
    pub fn select_all(&self) {
        self.keyboard_navigation();
        if self.view_mode() == BrowserMode::Columns {
            if let Some(depth) = self
                .state
                .focused_column_depth()
                .or_else(|| self.state.browser.active_depth())
            {
                self.state.select_all(depth);
            }
        } else if !self.state.mode_views.borrow().select_all_search_results()
            && let Some(depth) = self.state.browser.active_depth()
        {
            self.state.browser.select_all(depth);
        }
    }

    pub fn select_focused_pane(&self) -> bool {
        self.keyboard_navigation();
        let Some(depth) = self.focused_listing_depth() else {
            return false;
        };
        self.state.browser.set_active_column(depth);
        self.state.browser.select_visible(depth)
    }

    pub fn invert_focused_pane(&self) -> bool {
        self.keyboard_navigation();
        let Some(depth) = self.focused_listing_depth() else {
            return false;
        };
        self.state.browser.set_active_column(depth);
        self.state.browser.invert_visible(depth)
    }

    pub fn toggle_cursor_and_advance(&self) -> bool {
        if let Some(toggled) = self.toggle_result_and_advance() {
            return toggled;
        }
        self.keyboard_navigation();
        let Some(depth) = self.focused_listing_depth() else {
            return false;
        };
        self.state.browser.set_active_column(depth);
        if self.state.browser.visual_kind().is_some() {
            let order = self.displayed_order(depth);
            return self.state.browser.toggle_visual_cursor(order.as_deref());
        }
        if self.state.browser.toggle_cursor_fill() == crate::app::CursorToggle::Empty {
            return false;
        }
        self.move_displayed_cursor(1, 1);
        true
    }

    pub fn toggle_visual(&self, kind: crate::app::VisualKind) -> bool {
        if let Some(toggled) = self.toggle_result_visual(kind) {
            return toggled;
        }
        self.keyboard_navigation();
        let Some(depth) = self.focused_listing_depth() else {
            return false;
        };
        self.state.browser.set_active_column(depth);
        let order = self.displayed_order(depth);
        self.state.browser.toggle_visual(kind, order.as_deref())
    }

    pub fn refresh_visual(&self) {
        if let Some(depth) = self.state.browser.active_depth() {
            let order = self.displayed_order(depth);
            self.state.browser.refresh_visual(order.as_deref());
        }
    }

    pub fn leave_visual(&self) -> bool {
        self.leave_result_visual() || self.state.browser.leave_visual()
    }

    pub(in crate::ui) fn end_tenxer_session(&self) {
        self.leave_result_visual();
        self.state.browser.leave_visual();
        self.state.browser.close_peek();
        self.clear_find();
        self.forget_listing_search();
        self.clear_hidden_filters();
        self.release_forced_recursion();
    }

    /// Source positions of one pane in the order it displays them, including
    /// type groups and a Columns filter. `None` falls back to source order.
    pub(super) fn displayed_order(&self, depth: usize) -> Option<Vec<usize>> {
        let order: Vec<usize> = if self.view_mode() == BrowserMode::Columns {
            let columns = self.state.columns.borrow();
            let column = columns.get(depth)?;
            (0..column.selection.n_items())
                .filter_map(|position| column.map.source_position(position))
                .collect()
        } else {
            self.state.mode_views.borrow().visual_order(depth)
        };
        (!order.is_empty()).then_some(order)
    }

    pub(crate) fn displayed_entries_matching(
        &self,
        depth: usize,
        keep: impl Fn(&FileEntry) -> bool,
    ) -> Vec<FileEntry> {
        if let Some(results) = self.filter_target().and_then(|target| target.results()) {
            return results
                .iter()
                .map(search_result_entry)
                .filter(keep)
                .collect();
        }
        let order = self.displayed_order(depth);
        self.state
            .browser
            .with_column_entries(depth, |entries| match order {
                Some(order) => order
                    .iter()
                    .filter_map(|position| entries.get(*position))
                    .filter(|entry| keep(entry))
                    .cloned()
                    .collect(),
                None => entries
                    .iter()
                    .filter(|entry| keep(entry))
                    .cloned()
                    .collect(),
            })
            .unwrap_or_default()
    }

    pub(crate) fn displayed_cursor_entry(&self, depth: usize) -> Option<FileEntry> {
        if let Some(target) = self
            .filter_target()
            .filter(|target| target.results_view().is_some())
        {
            return target.current_result().as_ref().map(search_result_entry);
        }
        self.state.browser.cursor_entry(depth)
    }

    /// Steps the results cursor to the hit at `location`; `None` without filter results.
    fn step_to_filter_result(&self, location: &Location, key: Option<i32>) -> Option<bool> {
        let target = self.filter_target()?;
        let results = target.results()?;
        let Some(position) = results
            .iter()
            .position(|item| search_result_entry(item).location == *location)
        else {
            return Some(false);
        };
        let cursor = target.hits().and_then(|hits| hits.cursor);
        let delta = position as i32 - cursor.unwrap_or(0) as i32;
        if key.is_some() {
            self.keyboard_navigation();
        }
        if cursor.is_none() {
            target.step(1, 0, key.is_some());
        }
        Some(self.step_filter_results(delta.signum(), delta.unsigned_abs() as usize, key.is_some()))
    }

    /// 10xer places the cursor without selecting; otherwise the step selects the entry.
    fn place_stepped_cursor(&self, depth: usize, position: usize) {
        if crate::ui::preferences::PreferenceManager::shared().tenxer_mode() {
            let order = self.displayed_order(depth);
            self.state
                .browser
                .place_cursor(depth, position, order.as_deref());
        } else {
            self.state.browser.select(depth, position);
        }
    }

    /// `key` supplies the keyboard scroll direction; `None` preserves pointer navigation.
    pub(crate) fn step_to(&self, depth: usize, location: &Location, key: Option<i32>) -> bool {
        if let Some(stepped) = self.step_to_filter_result(location, key) {
            return stepped;
        }
        let Some(position) = self
            .state
            .browser
            .with_column_entries(depth, |entries| {
                entries.iter().position(|entry| entry.location == *location)
            })
            .flatten()
        else {
            return false;
        };
        if key.is_some() {
            self.keyboard_navigation();
        }
        let collection = self
            .state
            .overlay
            .root()
            .and_then(|root| root.focus())
            .as_ref()
            .and_then(crate::ui::scrolling::focused_collection);
        self.place_stepped_cursor(depth, position);
        self.state
            .mirror_focused_folder(depth, Some(position), false);
        if let (Some(direction), Some((view, scroll))) = (key, collection) {
            let position = self.cursor_view_position(&view);
            crate::ui::scrolling::reveal_cursor(
                &view,
                &scroll,
                direction,
                crate::ui::scrolling::CursorMotion::Step,
                position,
            );
        }
        true
    }

    pub(super) fn focused_listing_depth(&self) -> Option<usize> {
        if self.view_mode() == BrowserMode::Columns {
            self.state
                .focused_column_depth()
                .or_else(|| self.state.browser.active_depth())
        } else {
            self.state.browser.active_depth()
        }
    }

    /// Recursive results have their own selection, independent of the directory's selection.
    pub fn selected_search_result(&self) -> Option<FileEntry> {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().selected_search_result();
        }
        let focused = self.state.overlay.root()?.focus()?;
        self.state
            .columns
            .borrow()
            .iter()
            .enumerate()
            .find_map(|(depth, column)| {
                if !column_filter_holds_focus(column, &focused) {
                    return None;
                }
                self.column_search_selection(depth, column)
            })
    }

    /// The hit under a column's results cursor, or the newest selected filtered entry.
    fn column_search_selection(&self, depth: usize, column: &ColumnView) -> Option<FileEntry> {
        if column.recursive_search_active.get() {
            let cursor = listing_filter::column_cursor(column)?;
            return column
                .search_results
                .borrow()
                .get(cursor as usize)
                .map(search_result_entry);
        }
        if !column.map.has_query() {
            return None;
        }
        let selected = column.selection.selection();
        if selected.is_empty() {
            return None;
        }
        let source_position = column.map.source_position(selected.maximum())?;
        self.state.browser.entry_at(depth, source_position)
    }

    pub(in crate::ui) fn search_result_for_space(&self) -> Option<FileEntry> {
        self.filter_focus()?;
        self.selected_search_result()
    }

    pub(in crate::ui) fn connect_search_selection_changed(&self, handler: Rc<dyn Fn()>) {
        self.state
            .search_selection_handlers
            .borrow_mut()
            .push(handler.clone());
        self.state
            .mode_views
            .borrow()
            .connect_search_selection_changed(Rc::new(move |_| handler()));
    }

    pub fn selected_search_results(&self) -> Option<Vec<FileEntry>> {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().selected_search_results();
        }
        let columns = self.state.columns.borrow();
        let depth = self.state.destination_depth();
        let (depth, column) = depth
            .and_then(|depth| columns.get(depth).map(|column| (depth, column)))
            .filter(|(_, column)| column.recursive_search_active.get() || column.map.has_query())
            .or_else(|| {
                columns.iter().enumerate().find(|(_, column)| {
                    column.recursive_search_active.get() || column.map.has_query()
                })
            })?;
        if column.recursive_search_active.get() {
            let results = column.search_results.borrow();
            return Some(
                collection::bitset_positions(&column.selection.selection())
                    .into_iter()
                    .filter_map(|position| results.get(position as usize).map(search_result_entry))
                    .collect(),
            );
        }
        if column.map.has_query() {
            let positions = collection::bitset_positions(&column.selection.selection());
            let mapped = column.map.source_positions(&positions);
            return Some(
                mapped
                    .into_iter()
                    .filter_map(|(_, source_pos)| self.state.browser.entry_at(depth, source_pos))
                    .collect(),
            );
        }
        None
    }

    pub fn page_selection(&self, direction: i32, extend: bool) -> bool {
        self.step_selection_by_page(direction, extend, false)
    }

    pub(in crate::ui) fn half_page_selection(&self, direction: i32) -> bool {
        self.step_selection_by_page(direction, false, true)
    }

    pub(super) fn step_selection_by_page(&self, direction: i32, extend: bool, half: bool) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let Some((view, scroll)) = focused
            .as_ref()
            .and_then(crate::ui::scrolling::focused_collection)
        else {
            return false;
        };
        let page = crate::ui::scrolling::page(&view, &scroll);
        let items = if half {
            (page.items / 2).max(1)
        } else {
            page.items
        };
        self.state.mode_views.borrow().suppress_focus_scroll();
        if extend {
            self.extend_selection_by(&view, &scroll, direction, items);
            return true;
        }
        let target = self.state.browser.active_depth().and_then(|depth| {
            self.state
                .mode_views
                .borrow()
                .page_target(depth, direction, items)
                .map(|position| (depth, position))
        });
        if let Some((depth, position)) = target {
            self.state.browser.select(depth, position);
        } else {
            let order = self
                .state
                .browser
                .active_depth()
                .map(|depth| self.state.mode_views.borrow().visual_order(depth))
                .filter(|order| !order.is_empty());
            self.state
                .browser
                .page_along(direction, items, order.as_deref());
        }
        self.anchor_native_range();
        crate::ui::scrolling::reveal_selection(&view, &scroll, direction, &page);
        true
    }

    /// Shift+Home and Shift+End in Columns, which extend from the anchor as its
    /// Shift+arrows do; List and Icons leave them to GTK.
    pub fn extend_selection_to_edge(&self, direction: i32) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let Some((view, scroll)) = focused
            .as_ref()
            .and_then(crate::ui::scrolling::focused_collection)
        else {
            return false;
        };
        self.keyboard_navigation();
        self.extend_selection_by(&view, &scroll, direction, usize::MAX);
        true
    }

    fn extend_selection_by(
        &self,
        view: &gtk::Widget,
        scroll: &gtk::ScrolledWindow,
        direction: i32,
        items: usize,
    ) {
        // A full jump reveals like Home and End; a page reveals by page.
        let motion = if items == usize::MAX {
            crate::ui::scrolling::CursorMotion::Jump
        } else {
            crate::ui::scrolling::CursorMotion::Page
        };
        let order = self
            .state
            .browser
            .active_depth()
            .and_then(|depth| self.displayed_order(depth));
        self.state
            .browser
            .extend_page_selection(direction, items, order.as_deref());
        let position = self.cursor_view_position(view);
        crate::ui::scrolling::reveal_cursor(view, scroll, direction, motion, position);
    }

    /// The model moved the cursor, selection and anchor without GTK.
    fn anchor_native_range(&self) {
        if let Some(depth) = self.state.browser.active_depth() {
            self.state.mode_views.borrow().anchor_native_range(depth);
        }
    }

    pub fn move_displayed_cursor(&self, direction: i32, steps: usize) {
        let motion = if steps == usize::MAX {
            crate::ui::scrolling::CursorMotion::Jump
        } else {
            crate::ui::scrolling::CursorMotion::Step
        };
        self.shift_displayed_cursor(direction, steps, motion);
    }

    pub(super) fn shift_displayed_cursor(
        &self,
        direction: i32,
        steps: usize,
        motion: crate::ui::scrolling::CursorMotion,
    ) {
        if direction == 0 {
            return;
        }
        self.keyboard_navigation();
        let steps = steps.max(1);
        // Filter results replace the directory; its hidden cursor stays put.
        if self.step_filter_results(direction, steps, true) {
            return;
        }
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let collection = focused
            .as_ref()
            .and_then(crate::ui::scrolling::focused_collection);
        let depth = self.state.browser.active_depth();
        let target = depth.and_then(|depth| {
            self.state
                .mode_views
                .borrow()
                .page_target(depth, direction, steps)
                .map(|position| (depth, position))
        });
        let order = depth.and_then(|depth| self.displayed_order(depth));
        let cursor = || {
            self.state
                .browser
                .focused_item()
                .map(|(depth, position, _)| (depth, position))
        };
        let before = cursor();
        if let Some((depth, position)) = target {
            self.state
                .browser
                .place_cursor(depth, position, order.as_deref());
        } else {
            self.state
                .browser
                .page_cursor(direction, steps, order.as_deref());
        }
        // A move that clears the selection reports a fill, not a focus change.
        if let Some((depth, position)) = cursor()
            && Some((depth, position)) != before
        {
            self.state
                .mirror_focused_folder(depth, Some(position), false);
        }
        if let Some((view, scroll)) = collection {
            let position = self.cursor_view_position(&view);
            crate::ui::scrolling::reveal_cursor(&view, &scroll, direction, motion, position);
        }
    }

    pub(super) fn cursor_view_position(&self, view: &gtk::Widget) -> Option<u32> {
        let (depth, source, _) = self.state.browser.focused_item()?;
        let columns = self.state.columns.borrow();
        if let Some(column) = columns.get(depth) {
            let list = column.list.clone().upcast::<gtk::Widget>();
            if &list == view {
                return column.map.view_position(source);
            }
        }
        drop(columns);
        self.state
            .mode_views
            .borrow()
            .view_position_in(view, depth, source)
    }

    pub fn page_displayed_cursor(&self, direction: i32, half: bool) {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let items = focused
            .as_ref()
            .and_then(crate::ui::scrolling::focused_collection)
            .map(|(view, scroll)| crate::ui::scrolling::page(&view, &scroll).items)
            .unwrap_or(1)
            .max(1);
        let steps = if half { (items / 2).max(1) } else { items };
        self.shift_displayed_cursor(direction, steps, crate::ui::scrolling::CursorMotion::Page);
    }

    /// Moves the focus to the first or last visible entry of the active pane, for
    /// `Ctrl+Up`/`Ctrl+Down` and plain `Home`/`End`. Works from a focused row or
    /// tile, from the collection view itself, and from the pane surface while it
    /// shows a loading page.
    pub fn jump_selection(&self, direction: i32) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let collection = focused
            .as_ref()
            .and_then(crate::ui::scrolling::focused_collection);
        let parked = focused
            .as_ref()
            .is_some_and(|focused| focused.is::<gtk::Stack>())
            && self.item_view_has_focus();
        if collection.is_none() && !parked {
            return false;
        }
        self.keyboard_navigation();
        let order = self
            .state
            .browser
            .active_depth()
            .map(|depth| self.state.mode_views.borrow().visual_order(depth))
            .filter(|order| !order.is_empty());
        self.state
            .browser
            .page_along(direction, usize::MAX, order.as_deref());
        self.anchor_native_range();
        if let Some((view, scroll)) = collection {
            crate::ui::scrolling::reveal_jump(&view, &scroll, direction);
        }
        true
    }
}

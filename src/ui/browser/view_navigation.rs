// SPDX-License-Identifier: MIT

//! Keyboard navigation between columns, pane headers and the location bar.

use super::*;

fn header_holds_focus(column: &ColumnView, focused: Option<&gtk::Widget>) -> bool {
    focused.is_some_and(|focused| {
        focused == column.header_actions.upcast_ref::<gtk::Widget>()
            || focused.is_ancestor(&column.header_actions)
    })
}

impl BrowserView {
    pub fn activate_focused(&self) {
        if self.view_mode() != BrowserMode::Columns {
            self.state.browser.activate_focused_in_place();
        } else {
            self.state.browser.activate_focused();
        }
    }

    pub(in crate::ui) fn activate_directory_on_space(&self) -> bool {
        if let Some(entry) = self.selected_search_result() {
            if entry.is_directory() {
                self.state.browser.navigate(entry.location);
                return true;
            }
        } else if self
            .state
            .browser
            .focused_entry()
            .is_some_and(|entry| entry.is_directory())
        {
            self.activate_focused();
            return true;
        }
        false
    }

    pub fn commit_selection(&self) {
        self.state.browser.commit_selection();
    }

    pub fn resume_native_selection(&self) -> bool {
        self.state.mode_views.borrow().resume_native_selection()
    }

    pub fn navigate_left(&self) {
        if self.view_mode() != BrowserMode::Columns {
            self.state.browser.parent();
        } else {
            self.state.browser.focus_parent();
        }
    }

    pub fn synchronize_native_selection(&self, extend: bool) {
        let position = self.state.mode_views.borrow().focused_position();
        if let Some((depth, focused)) = position {
            self.select_native_target(depth, focused, extend);
        }
    }

    pub fn cross_type_group(&self, _direction: gtk::DirectionType, _extend: bool) -> bool {
        false
    }

    pub(super) fn select_native_target(&self, depth: usize, focused: usize, extend: bool) {
        if extend {
            let order = self.state.mode_views.borrow().visual_order(depth);
            self.state
                .browser
                .extend_visual_selection(depth, focused, &order);
        } else {
            self.state.browser.select(depth, focused);
        }
    }

    pub fn item_at_sidebar_edge(&self) -> bool {
        if self.view_mode() == BrowserMode::Columns {
            self.first_column_has_focus()
        } else {
            self.state.mode_views.borrow().item_at_left_edge()
        }
    }

    pub fn at_left_edge(&self) -> bool {
        self.state.mode_views.borrow().at_left_edge()
    }

    pub fn first_column_has_focus(&self) -> bool {
        self.view_mode() == BrowserMode::Columns && self.state.focused_column_depth() == Some(0)
    }

    pub fn focus_header_from_top_item(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self
                .state
                .mode_views
                .borrow()
                .focus_header_from_top_item(!self.state.interactive);
        }
        let Some((depth, position, _)) = self.state.browser.focused_item() else {
            return false;
        };
        let columns = self.state.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return false;
        };
        if column.map.view_position(position) != Some(0) {
            return false;
        }
        let focused = focus_header_action(&column.header_actions, gtk::DirectionType::Right);
        if focused {
            self.show_focus_visible();
        }
        focused
    }

    fn show_focus_visible(&self) {
        if let Some(window) = self.state.overlay.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
    }

    fn focused_widget(&self) -> Option<gtk::Widget> {
        self.state.overlay.root().and_then(|root| root.focus())
    }

    pub fn header_actions_have_focus(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().header_has_focus();
        }
        let focused = self.focused_widget();
        self.state
            .columns
            .borrow()
            .iter()
            .any(|column| header_holds_focus(column, focused.as_ref()))
    }

    pub fn move_header_focus(&self, direction: gtk::DirectionType) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().move_header_focus(direction);
        }
        let focused = self.focused_widget();
        let columns = self.state.columns.borrow();
        let Some(index) = columns
            .iter()
            .position(|column| header_holds_focus(column, focused.as_ref()))
        else {
            return false;
        };
        if columns[index].header_actions.child_focus(direction) {
            return true;
        }
        let adjacent = match direction {
            gtk::DirectionType::Left => index.checked_sub(1),
            gtk::DirectionType::Right => (index + 1 < columns.len()).then_some(index + 1),
            _ => None,
        };
        let Some(column) = adjacent.and_then(|index| columns.get(index)) else {
            return false;
        };
        column
            .header_actions_stack
            .set_visible_child_name("actions");
        let moved = focus_header_action(&column.header_actions, direction);
        self.state.refresh_destination_style();
        if moved {
            self.show_focus_visible();
        }
        moved
    }

    pub fn focus_items_from_header(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().focus_items_from_header();
        }
        let focused = self.focused_widget();
        self.state
            .columns
            .borrow()
            .iter()
            .find(|column| header_holds_focus(column, focused.as_ref()))
            .is_some_and(ColumnView::focus_surface)
    }

    /// Columns only: the focused column becomes the first one. False when
    /// there is nothing to the left of it or the strip was not re-rooted.
    pub fn make_focused_column_first(&self) -> bool {
        if self.view_mode() != BrowserMode::Columns {
            return false;
        }
        let Some(depth) = self
            .state
            .focused_column_depth()
            .or_else(|| self.state.browser.active_depth())
            .filter(|depth| *depth > 0)
        else {
            return false;
        };
        self.state.browser.make_first_column(depth)
    }

    pub fn navigate_up(&self) {
        if self.view_mode() != BrowserMode::Columns {
            self.state.browser.parent();
            return;
        }

        match self
            .state
            .focused_column_depth()
            .or_else(|| self.state.browser.active_depth())
        {
            Some(0) => {
                if let Some(parent) = self
                    .state
                    .browser
                    .location_at(0)
                    .and_then(|location| location.parent())
                {
                    self.state.browser.navigate_to_ancestor(parent);
                }
            }
            Some(depth) => self.state.browser.close_column(depth),
            None => {}
        }
    }

    pub fn location_widget(&self) -> gtk::Widget {
        self.state.location_control.clone().upcast()
    }

    /// Shows the shared header spinner until the returned activity guard is dropped.
    pub fn begin_global_activity(&self, label: impl Into<String>) -> GlobalActivity {
        self.state.begin_global_activity(label)
    }

    pub fn begin_location_edit(&self) {
        self.state.begin_location_edit();
    }

    pub fn location_has_focus(&self) -> bool {
        self.state.location_entry_has_focus()
    }

    pub(in crate::ui) fn location_edit_is_active(&self) -> bool {
        self.state.location_stack.visible_child_name().as_deref() == Some("entry")
    }

    pub(in crate::ui) fn location_edit_contains(&self, target: &gtk::Widget) -> bool {
        let location_stack = self.state.location_stack.upcast_ref::<gtk::Widget>();
        target == location_stack || target.is_ancestor(location_stack)
    }

    pub fn cancel_location_edit(&self) {
        self.state.cancel_location_edit();
    }

    /// Opens a path or URI typed outside the location bar. A relative path
    /// resolves against the open native folder; errors use the location bar's
    /// dialog and leave the current folder open.
    pub(in crate::ui) fn open_typed_location(&self, input: &str) {
        if let Err(error) = self
            .state
            .open_typed_location(input, self.typed_location_base().as_deref())
        {
            show_error_dialog(
                &self.state.overlay,
                &crate::i18n::tr("Unable to open location"),
                &error.to_string(),
            );
        }
    }

    /// Opens a typed path or URI for a dialog that stays up while it connects.
    /// `finished` runs once, when the location opens or connecting fails, unless
    /// [`Self::cancel_connect`] runs first. Invalid input is returned instead.
    pub(in crate::ui) fn connect_typed_location(
        &self,
        input: &str,
        finished: impl Fn(ConnectOutcome) + 'static,
    ) -> Result<(), LocationValidationError> {
        self.state.connect_typed_location(
            input,
            self.typed_location_base().as_deref(),
            Box::new(finished),
        )
    }

    pub(in crate::ui) fn cancel_connect(&self) {
        self.state.cancel_connect();
    }

    pub(super) fn typed_location_base(&self) -> Option<std::path::PathBuf> {
        self.state
            .browser
            .active_location()
            .and_then(|location| location.native_path().map(std::path::Path::to_path_buf))
    }

    pub fn focus_search_results(&self) -> bool {
        self.state.mode_views.borrow().focus_search_results()
    }

    #[cfg(test)]
    pub fn focus_search_result(&self, path: &std::path::Path) -> bool {
        self.state.mode_views.borrow().focus_search_result(path)
    }

    pub fn keyboard_navigation(&self) {
        self.state
            .input_ownership
            .borrow_mut()
            .keyboard_navigation();
        self.state.overlay.add_css_class("keyboard-navigation");
        if let Some(window) = self.state.overlay.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        cancel_source(&self.state.pending_peek);
        self.state.browser.close_peek();
        // Capture-phase keys run before the pane sees the event that would cancel
        // an in-progress restore, so the command has to cancel it first.
        self.state.mode_views.borrow_mut().cancel_pending_restore();
        self.state.sync_mode_selection();
        self.state.cancel_tab_location_hold();
        self.state.refresh_destination_style();
    }

    pub fn item_view_has_focus(&self) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        self.state.mode_views.borrow().item_view_has_focus()
            || self.state.columns.borrow().iter().any(|column| {
                focused.as_ref().is_some_and(|focused| {
                    focused == column.presentation.stack.upcast_ref::<gtk::Widget>()
                        || focused == column.list.upcast_ref::<gtk::Widget>()
                        || focused.is_ancestor(&column.list)
                })
            })
    }
}

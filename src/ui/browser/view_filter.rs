// SPDX-License-Identifier: MIT

//! The focused pane's filter field and keyboard-opened context menus.

use super::*;

impl BrowserView {
    pub fn show_filter(&self) -> bool {
        self.show_filter_with_optional_query(None)
    }

    pub fn show_filter_with_query(&self, query: &str) -> bool {
        self.show_filter_with_optional_query(Some(query))
    }

    pub(super) fn show_filter_with_optional_query(&self, query: Option<&str>) -> bool {
        if crate::ui::tenxer_mode::chrome_suppressed() {
            return false;
        }
        if self.view_mode() != BrowserMode::Columns {
            return self.state.mode_views.borrow().show_filter_with_query(query);
        }
        let depth = self.state.destination_depth();
        let column = depth.and_then(|depth| self.state.columns.borrow().get(depth).cloned());
        let Some(column) = column else {
            return false;
        };
        column.filter_button.set_active(true);
        focus_filter_entry(&column.filter_entry, query);
        true
    }

    pub fn filter_has_focus(&self) -> bool {
        self.filter_focus() == Some(FilterFocus::Entry)
    }

    pub fn dismiss_empty_focused_filter(&self) -> bool {
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let empty = self.state.mode_views.borrow().empty_filter_has_focus()
            || self.state.columns.borrow().iter().any(|column| {
                column.filter_entry.text().is_empty()
                    && focused.as_ref().is_some_and(|focused| {
                        focused == column.filter_entry.upcast_ref::<gtk::Widget>()
                            || focused.is_ancestor(&column.filter_entry)
                    })
            });
        empty && self.dismiss_focused_filter()
    }

    /// Dismisses the filter whose field or results have focus. An empty revealed
    /// Columns filter has no results, so Escape on its rows clears the selection instead.
    pub fn dismiss_focused_filter(&self) -> bool {
        if self.state.mode_views.borrow().dismiss_focused_filter() {
            return true;
        }
        let focused = self.state.overlay.root().and_then(|root| root.focus());
        let column = focused.and_then(|focused| {
            self.state
                .columns
                .borrow()
                .iter()
                .find(|column| listing_filter::column_filter_focus(column, &focused).is_some())
                .cloned()
        });
        let Some(column) = column else {
            return false;
        };
        column.filter_button.set_active(false);
        // Show the listing now rather than after the debounce, so focus lands on it.
        column.flush_filter_query();
        column.focus_surface();
        // The returning rows arrive unselected; restore the directory's cursor and
        // selection on them.
        self.state.browser.focus_active();
        true
    }

    pub(in crate::ui) fn open_focused_context_menu(&self) -> bool {
        let focused = self.state.browser.focused_item();
        let depth = focused
            .as_ref()
            .map(|(depth, ..)| *depth)
            .or_else(|| self.state.browser.active_depth());
        let Some(depth) = depth else {
            return false;
        };
        let position = focused.map(|(_, position, _)| position).filter(|position| {
            self.state
                .browser
                .selected_positions(depth)
                .contains(position)
        });

        let target = self.context_menu_target(depth, position);

        let Some((trigger, x, y)) = target else {
            return false;
        };
        trigger(x, y);
        true
    }

    fn context_menu_target(
        &self,
        depth: usize,
        position: Option<usize>,
    ) -> Option<ContextMenuTarget> {
        if self.view_mode() == BrowserMode::Columns {
            return self
                .state
                .columns
                .borrow()
                .get(depth)?
                .context_menu_target(position);
        }
        self.state
            .mode_views
            .borrow()
            .context_menu_target(depth, position)
    }

    pub fn dismiss_filter_on_outside_click(&self, root: &gtk::Widget, x: f64, y: f64) {
        if root
            .root()
            .and_downcast::<gtk::Window>()
            .is_some_and(|window| crate::ui::window::visible_modal_layer(&window).is_some())
        {
            return;
        }
        if self.view_mode() != BrowserMode::Columns {
            return;
        }
        let picked = root.pick(x, y, gtk::PickFlags::DEFAULT);
        let filter_button = {
            let columns = self.state.columns.borrow();
            let Some(column) = columns.iter().find(|c| c.filter_button.is_active()) else {
                return;
            };
            let inside = picked.as_ref().is_some_and(|p| {
                p == column.shell.upcast_ref::<gtk::Widget>() || p.is_ancestor(&column.shell)
            });
            if inside {
                return;
            }
            column.filter_button.clone()
        };
        filter_button.set_active(false);
    }
}

// SPDX-License-Identifier: MIT

//! The 10xer cursor, fills and visual ranges, and the moves they share.

use std::rc::Rc;

use crate::{
    app::{Browser, VisualKind},
    ui::keys::Key,
};

use super::super::Dispatcher;

impl Dispatcher {
    /// Grid motion keeps every committed fill, including a singleton and an empty
    /// inverted fill. A load cursor still follows GTK's spatial move.
    pub(super) fn move_icon_cursor(&self, browser: &Rc<Browser>, arrow: Key, search: bool) -> bool {
        if !search && browser.visual_kind().is_some() {
            self.move_icon_range(browser, arrow);
            return true;
        }
        let preserved = (!search).then_some(()).and_then(|_| {
            let depth = browser.active_depth()?;
            if browser.selection_is_load_cursor() {
                return None;
            }
            let positions = browser.selected_positions(depth);
            let cursor = browser
                .focused_item()
                .filter(|(item_depth, _, _)| *item_depth == depth)
                .map(|(_, position, _)| position)?;
            Some((depth, positions, cursor))
        });
        if let Some((depth, _, cursor)) = preserved.clone() {
            browser.install_pane_fill(depth, &[cursor], cursor);
        }
        self.view.keyboard_navigation();
        if search {
            self.view.keep_result_fill(|| {
                self.view.focus_search_results();
                crate::ui::focus_navigation::activate_native_arrow(&self.window, arrow);
            });
            return true;
        }
        crate::ui::focus_navigation::activate_native_arrow(&self.window, arrow);
        if let Some((depth, positions, _)) = preserved {
            let cursor = browser
                .focused_item()
                .filter(|(item_depth, _, _)| *item_depth == depth)
                .map(|(_, position, _)| position)
                .unwrap_or(0);
            browser.install_pane_fill(depth, &positions, cursor);
        }
        true
    }

    /// GTK moves the grid cursor over a single-item selection; the walked range
    /// is then recomputed from the anchor in displayed order.
    pub(super) fn move_icon_range(&self, browser: &Rc<Browser>, arrow: Key) {
        let Some((depth, cursor, _)) = browser.focused_item() else {
            return;
        };
        self.view.keyboard_navigation();
        // GTK's selection echo would otherwise end the range as a pointer change.
        let range = browser.take_visual();
        browser.install_pane_fill(depth, &[cursor], cursor);
        crate::ui::focus_navigation::activate_native_arrow(&self.window, arrow);
        browser.restore_visual(range);
        self.view.refresh_visual();
    }

    /// Inverts the filter's hits or the focused pane; false while search hits
    /// own the selection.
    pub(in crate::ui::window::keyboard) fn invert_selection(&self) -> bool {
        self.view.invert_filter_results()
            || (self.view.selected_search_results().is_none()
                && self.fill_selection(self.view.invert_focused_pane()))
    }

    pub(super) fn fill_selection(&self, filled: bool) -> bool {
        if !filled {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to select"));
        }
        true
    }

    pub(super) fn toggle_visual(&self, kind: VisualKind) {
        if !self.view.toggle_visual(kind) {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to select"));
        }
    }

    pub(super) fn toggle_tenxer_cursor(&self) {
        if !self.view.toggle_cursor_and_advance() {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to select"));
        }
    }

    /// Search hits shown in the listing swallow paging.
    pub(in crate::ui::window::keyboard) fn page_cursor(&self, direction: i32, half: bool) {
        if !self.view.listing_search_active() {
            self.view.page_displayed_cursor(direction, half);
        }
    }

    pub(super) fn jump_displayed(&self, direction: i32) {
        self.view.move_displayed_cursor(direction, usize::MAX);
    }

    pub(super) fn go_parent(&self) {
        self.view.keyboard_navigation();
        self.view.navigate_up();
    }

    pub(super) fn go_back(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        browser.back();
    }

    pub(super) fn go_forward(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        browser.forward();
    }

    /// Opens the focused result while results replace the directory, and
    /// nothing when there is none rather than the hidden directory's cursor.
    pub(super) fn activate_focused(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        if self.chooser_confirm(browser) {
            return;
        }
        if let Some(entry) = self.view.selected_search_result() {
            if entry.is_directory() {
                browser.navigate(entry.location);
            } else {
                browser.open_location(entry.location);
            }
            return;
        }
        if self.view.results_replace_listing() {
            return;
        }
        self.view.activate_focused();
    }

    pub(super) fn selection_keys_blocked(&self) -> bool {
        self.view.selected_search_results().is_some() && !self.view.results_replace_listing()
    }

    /// Opens the directory under the cursor, or the directory hit, in the
    /// next column without moving focus.
    pub(super) fn open_miller_child(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        if self.view.results_replace_listing() {
            if let (Some(depth), Some(entry)) =
                (browser.active_depth(), self.view.selected_search_result())
                && entry.is_directory()
            {
                self.view.open_hit_column(depth, entry.location);
            }
            return;
        }
        let Some((depth, _, entry)) = browser.focused_item() else {
            return;
        };
        if entry.is_directory() {
            browser.show_child(depth, entry.location);
        }
    }
}

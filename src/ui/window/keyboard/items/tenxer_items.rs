// SPDX-License-Identifier: MIT

//! 10xer keys on the focused listing: Icons grid and the List and Columns rows.

use std::rc::Rc;

use crate::{
    app::{Browser, VisualKind},
    ui::{
        browser_modes::{BrowserMode, ThumbnailResize},
        keys::{Binding, Command, KeyPress, bindings::*},
        preview::MediaCommand,
        tenxer_mode::Chord,
    },
};

use super::super::Dispatcher;

/// Paging and jumping keys do nothing while a listing search shows its hits.
const SWALLOWED_ON_HITS: [Command; 4] = [FIRST_ITEM, LAST_ITEM, PAGE_UP, PAGE_DOWN];

const EXTEND_KEYS: [Binding; 6] = [
    EXTEND_UP,
    EXTEND_DOWN,
    EXTEND_LEFT,
    EXTEND_RIGHT,
    EXTEND_PAGE_UP,
    EXTEND_PAGE_DOWN,
];

impl Dispatcher {
    pub(in crate::ui::window::keyboard) fn tenxer_icons(
        &self,
        browser: &Rc<Browser>,
        press: &KeyPress,
    ) -> bool {
        if !self.view.view_mode().is_grid() || !self.view.item_view_has_focus() {
            return false;
        }
        if self.tenxer_open_focused_in_new(browser, press) {
            return true;
        }
        let search = self.view.selected_search_results().is_some();
        if let Some(claimed) = self.icon_media_or_size(press) {
            return claimed;
        }
        if search && !press.modifiers().is_empty() {
            return self.modified_key_on_hits(press);
        }
        self.tenxer_motion(browser, press) || self.icon_arrow_or_command(browser, press, search)
    }

    fn icon_media_or_size(&self, press: &KeyPress) -> Option<bool> {
        if let Some(command) = media_track(press) {
            return Some(self.preview.media_key(command));
        }
        // Before the search branch: the Shift that types + counts as held there.
        ThumbnailResize::for_tenxer_key(press)
            .map(|change| self.view.resize_icons_thumbnails(change))
    }

    fn icon_arrow_or_command(&self, browser: &Rc<Browser>, press: &KeyPress, search: bool) -> bool {
        if let Some(arrow) = crate::ui::focus_navigation::spatial_arrow(press) {
            return self.move_icon_cursor(browser, arrow.key(), search);
        }
        self.swallowed_on_hits(press) || self.tenxer_icon_command(browser, press, search)
    }

    /// Of the modified keys, only these act on hits, whatever keys they are bound to.
    fn modified_key_on_hits(&self, press: &KeyPress) -> bool {
        if press.is(INVERT_SELECTION) && self.view.invert_filter_results() {
            return true;
        }
        if self.view.listing_search_active() && self.tenxer_control_page(press) {
            return true;
        }
        let extends = press.is_any(&[LAST_ITEM, VISUAL_UNSET]) || press.is_any(&EXTEND_KEYS);
        extends && self.tenxer_shifted(press)
    }

    fn swallowed_on_hits(&self, press: &KeyPress) -> bool {
        press.is_any(&SWALLOWED_ON_HITS) && self.view.listing_search_active()
    }

    fn tenxer_icon_command(&self, browser: &Rc<Browser>, press: &KeyPress, search: bool) -> bool {
        self.icon_jump(press, search)
            || self.icon_open(browser, press)
            || self.icon_page(press, search)
            || self.tenxer_mark(press)
            || self.arm_go(press)
    }

    /// Hits replace the grid's order, so these listing moves wait for the search to end.
    fn icon_jump(&self, press: &KeyPress, search: bool) -> bool {
        if search {
            return false;
        }
        if press.is(FIRST_ITEM) {
            self.jump_displayed(-1);
        } else if press.is(LAST_ITEM) {
            self.jump_displayed(1);
        } else if press.is(BACKSPACE) {
            self.go_parent();
        } else {
            return false;
        }
        true
    }

    fn icon_open(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        if press.is(OPEN) || press.is(ENTER) {
            self.activate_focused(browser);
        } else if press.is(PREVIEW) {
            if !self.toggle_file_preview(browser) {
                self.view.toggle_folder_peek();
            }
        } else {
            return false;
        }
        true
    }

    fn icon_page(&self, press: &KeyPress, search: bool) -> bool {
        if search {
            return false;
        }
        if press.is(PAGE_UP) {
            self.view.page_displayed_cursor(-1, false);
        } else if press.is(PAGE_DOWN) {
            self.view.page_displayed_cursor(1, false);
        } else {
            return false;
        }
        true
    }

    fn tenxer_mark(&self, press: &KeyPress) -> bool {
        if press.is(SPACE) && !self.selection_keys_blocked() {
            self.toggle_tenxer_cursor();
        } else if press.is(VISUAL_SELECT) && !self.selection_keys_blocked() {
            self.toggle_visual(VisualKind::Select);
        } else {
            return false;
        }
        true
    }

    fn arm_go(&self, press: &KeyPress) -> bool {
        if !press.is(GO) {
            return false;
        }
        self.shortcuts.arm_chord(Chord::Go);
        true
    }

    pub(in crate::ui::window::keyboard) fn tenxer_listing(
        &self,
        browser: &Rc<Browser>,
        press: &KeyPress,
    ) -> bool {
        if self.view.view_mode().is_grid() || !self.view.item_view_has_focus() {
            return false;
        }
        self.tenxer_open_focused_in_new(browser, press)
            || self.tenxer_motion(browser, press)
            || self.tenxer_plain(browser, press)
    }

    fn tenxer_motion(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        self.tenxer_control_page(press)
            || self.tenxer_first_or_last(press)
            || self.tenxer_selection_command(press)
            || self.tenxer_alt_navigation(browser, press)
            || self.tenxer_shifted(press)
    }

    fn tenxer_selection_command(&self, press: &KeyPress) -> bool {
        if press.is(INVERT_SELECTION) {
            return self.invert_selection();
        }
        press.is(SELECT_ALL)
            && self.view.selected_search_results().is_none()
            && self.fill_selection(self.view.select_focused_pane())
    }

    fn tenxer_first_or_last(&self, press: &KeyPress) -> bool {
        let direction = if press.is(FIRST_ITEM_UP) {
            -1
        } else if press.is(LAST_ITEM_DOWN) {
            1
        } else {
            return false;
        };
        self.jump_displayed(direction);
        true
    }

    /// Plain Page Up and Page Down page in `tenxer_plain`, where search hits
    /// swallow them.
    fn tenxer_control_page(&self, press: &KeyPress) -> bool {
        let (direction, half) = if press.is(HALF_PAGE_UP) {
            (-1, true)
        } else if press.is(HALF_PAGE_DOWN) {
            (1, true)
        } else if press.is(PAGE_UP) && !press.is(PAGE_UP_KEY) {
            (-1, false)
        } else if press.is(PAGE_DOWN) && !press.is(PAGE_DOWN_KEY) {
            (1, false)
        } else {
            return false;
        };
        self.page_cursor(direction, half);
        true
    }

    fn tenxer_alt_navigation(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        if press.is(HISTORY_BACK) {
            self.go_back(browser);
        } else if press.is(HISTORY_FORWARD) {
            self.go_forward(browser);
        } else if press.is(PARENT_FOLDER) {
            self.go_parent();
        } else {
            return false;
        }
        true
    }

    /// End jumps in `tenxer_plain`, where search hits swallow it.
    fn tenxer_shifted(&self, press: &KeyPress) -> bool {
        // Native Shift selection must not interfere with v/V ranges.
        if press.is_any(&EXTEND_KEYS) {
            return true;
        }
        if press.is(VISUAL_UNSET) && !self.selection_keys_blocked() {
            self.toggle_visual(VisualKind::Unset);
        } else if press.is(LAST_ITEM) && !press.is(END_KEY) {
            self.jump_displayed(1);
        } else if press.is(SCROLL_PREVIEW_DOWN) {
            self.scroll_open_preview(1);
        } else if press.is(SCROLL_PREVIEW_UP) {
            self.scroll_open_preview(-1);
        } else {
            return false;
        }
        true
    }

    fn tenxer_plain(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        if self.swallowed_on_hits(press) || self.plain_move_or_open(browser, press) {
            return true;
        }
        if let Some(command) = media_track(press) {
            return self.preview.media_key(command);
        }
        self.plain_preview(browser, press) || self.plain_page(press) || self.tenxer_mark(press)
    }

    fn plain_move_or_open(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        self.plain_cursor(press)
            || self.plain_sideways(browser, press)
            || self.plain_open(browser, press)
            || self.arm_go(press)
    }

    fn plain_open(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        if !press.is(OPEN) && !press.is(ENTER) {
            return false;
        }
        self.activate_focused(browser);
        true
    }

    fn plain_cursor(&self, press: &KeyPress) -> bool {
        if press.is(DOWN.or_alternates()) {
            self.view.move_displayed_cursor(1, 1);
        } else if press.is(UP.or_alternates()) {
            self.view.move_displayed_cursor(-1, 1);
        } else if press.is(FIRST_ITEM) {
            self.jump_displayed(-1);
        } else if press.is(LAST_ITEM) {
            self.jump_displayed(1);
        } else {
            return false;
        }
        true
    }

    /// Left leaves a search before it leaves the folder; Right opens the preview.
    fn plain_sideways(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        let left = press.is(LEFT.or_alternates());
        if left && self.view.listing_search_active() {
            self.view.dismiss_listing_search();
        } else if left || press.is(BACKSPACE) {
            self.go_parent();
        } else if press.is(RIGHT.or_alternates()) {
            self.enter_preview(browser);
        } else {
            return false;
        }
        true
    }

    fn plain_preview(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        if !press.is(PREVIEW) {
            return false;
        }
        if self.toggle_file_preview(browser) {
            return true;
        }
        if self.view.view_mode() == BrowserMode::Columns {
            self.open_miller_child(browser);
        } else {
            self.view.toggle_folder_peek();
        }
        true
    }

    fn plain_page(&self, press: &KeyPress) -> bool {
        if press.is(PAGE_UP) {
            self.view.page_displayed_cursor(-1, false);
        } else if press.is(PAGE_DOWN) {
            self.view.page_displayed_cursor(1, false);
        } else {
            return false;
        }
        true
    }
}

fn media_track(press: &KeyPress) -> Option<MediaCommand> {
    if press.is(TENXER_MEDIA_PREVIOUS) {
        Some(MediaCommand::Previous)
    } else if press.is(TENXER_MEDIA_NEXT) {
        Some(MediaCommand::Next)
    } else {
        None
    }
}

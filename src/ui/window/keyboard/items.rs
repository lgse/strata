// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
    prelude::*,
};

use super::{Dispatcher, KeyEvent, KeyResult};
use crate::{
    app::{Browser, VisualKind},
    model::Location,
    ui::{
        browser_modes::{BrowserMode, ThumbnailResize},
        keys::{Binding, Command, KeyPress, Map, Match, bindings::*},
        preview::{ArchiveMove, MediaCommand, PreviewSurface, preview_target},
        tenxer_mode::Chord,
        window::{
            SinglePaneArrow, home_directory, sidebar_focus_direction, single_pane_arrow_action,
        },
    },
};

const SHIFT: Modifiers = Modifiers::SHIFT_MASK;
const SUPER: Modifiers = Modifiers::SUPER_MASK;

#[derive(Clone, Copy)]
enum OpenInNewTarget {
    Tab,
    Window,
}

impl Dispatcher {
    /// A focused file opens as with plain Enter.
    fn open_focused_in_new(&self, browser: &Rc<Browser>, target: OpenInNewTarget) -> bool {
        // Choosers have no tabs, and Ctrl+Enter there accepts the folder.
        if self.chooser.is_some() {
            return false;
        }
        let entry = self
            .view
            .selected_search_result()
            .or_else(|| browser.focused_entry());
        let Some(entry) = entry else {
            return false;
        };
        if !entry.is_directory() {
            self.activate_focused(browser);
            return true;
        }
        let action = match target {
            OpenInNewTarget::Tab => "win.open-tab-at",
            OpenInNewTarget::Window => "win.open-window-at",
        };
        let uri = crate::adapters::gio_file_for_location(&entry.location)
            .uri()
            .to_variant();
        if let Err(error) = self.window.activate_action(action, Some(&uri)) {
            tracing::warn!(%error, action, "open-in-new location action unavailable");
        }
        true
    }

    fn tenxer_open_focused_in_new(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        let target = if press.is(OPEN_IN_NEW_TAB) {
            OpenInNewTarget::Tab
        } else if press.is(OPEN_IN_NEW_WINDOW) {
            OpenInNewTarget::Window
        } else {
            return false;
        };
        self.open_focused_in_new(browser, target)
    }

    pub(super) fn dismissal(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        const DELETE_MODIFIERS: Modifiers = Modifiers::CONTROL_MASK
            .union(Modifiers::ALT_MASK)
            .union(SUPER);
        let press = &event.press;
        if press.is(BACKSPACE.ignoring(SHIFT.union(SUPER)))
            && self.view.dismiss_empty_focused_filter()
        {
            return Some(Propagation::Stop);
        }
        let permanent = if press.is(MOVE_TO_TRASH
            .in_map(Map::Default)
            .ignoring(DELETE_MODIFIERS))
        {
            Some(false)
        } else if press.is(DELETE_PERMANENTLY
            .in_map(Map::Default)
            .ignoring(DELETE_MODIFIERS))
        {
            Some(true)
        } else {
            None
        };
        if let Some(permanent) = permanent
            && !self.view.filter_has_focus()
            && !event.text_has_focus()
            && self.view.confirm_delete(permanent)
        {
            return Some(Propagation::Stop);
        }
        if press.is(ESCAPE_ANY_SHIFT) && !event.text_has_focus() {
            return self.dismiss_preview_or_selection(browser);
        }
        None
    }

    pub(super) fn dismiss_preview_or_selection(&self, browser: &Browser) -> KeyResult {
        if self.preview.is_enabled() {
            self.preview.close();
            browser.focus_active();
            return Some(Propagation::Stop);
        }
        // Transient surfaces may return focus to pane chrome rather than an item.
        (browser.close_peek() || browser.clear_active_selection()).then_some(Propagation::Stop)
    }

    pub(super) fn preview_navigation(&self, event: &KeyEvent) -> KeyResult {
        let press = &event.press;
        if event.text_has_focus()
            && !event.focused.as_ref().is_some_and(|focused| {
                self.preview.owns_focus(Some(focused))
                    && self.preview.surface(focused) == PreviewSurface::Document
            })
        {
            return None;
        }
        if self.view.item_view_has_focus() {
            if self.view.view_mode() == BrowserMode::Columns
                && (press.is(RIGHT) || press.is(ENTER) || press.is(SPACE))
                && self.view.browser().focused_entry().is_some_and(|entry| {
                    !entry.is_directory()
                        && entry.location.native_path().is_some()
                        && crate::services::archive_preview_format(&entry.native_name).is_some()
                })
            {
                self.enter_preview(&self.view.browser());
                return Some(Propagation::Stop);
            }
            if press.is(RIGHT)
                && matches!(
                    self.view.view_mode(),
                    BrowserMode::List | BrowserMode::Columns
                )
                && self.preview.is_open()
                && self
                    .view
                    .browser()
                    .focused_entry()
                    .is_some_and(|entry| !entry.is_directory())
                && self.preview.take_keyboard()
            {
                return Some(Propagation::Stop);
            }
            return None;
        }
        if !self.preview.owns_focus(event.focused.as_ref()) {
            return None;
        }
        if let Some(focused) = event.focused.as_ref() {
            if self.preview.feedback_action_focused(focused) {
                if press.is(LEFT) {
                    self.return_from_preview(&self.view.browser());
                    return Some(Propagation::Stop);
                }
                // Arrows would otherwise walk GTK focus out of the drawer.
                if press.is_any(&[UP, DOWN, RIGHT]) {
                    return Some(Propagation::Stop);
                }
            } else if press.is_any(&[DOWN, RIGHT])
                && self.preview.surface(focused) == PreviewSurface::Document
                && self.preview.focus_feedback_action()
            {
                return Some(Propagation::Stop);
            }
        }
        if press.is(ENTER)
            && event
                .focused
                .as_ref()
                .is_some_and(|focused| self.preview.surface(focused) == PreviewSurface::Media)
        {
            self.activate_focused(&self.view.browser());
            return Some(Propagation::Stop);
        }
        if press.is(LEFT)
            && self.preview.archive_at_root()
            && event.focused.as_ref().is_some_and(|focused| {
                matches!(
                    self.preview.surface(focused),
                    PreviewSurface::Archive | PreviewSurface::Document
                )
            })
        {
            self.return_from_preview(&self.view.browser());
            return Some(Propagation::Stop);
        }
        if !self.preview.archive_list_has_focus(event.focused.as_ref()) {
            return None;
        }
        if press.is(SPACE) {
            return self.preview.close_archive().then_some(Propagation::Stop);
        }
        let motion = [
            (UP, ArchiveMove::Previous),
            (DOWN, ArchiveMove::Next),
            (LEFT, ArchiveMove::Parent),
            (RIGHT, ArchiveMove::Open),
            (ENTER, ArchiveMove::Open),
        ]
        .into_iter()
        .find(|(key, _)| press.is(*key))?
        .1;
        self.preview
            .archive_key(motion)
            .then_some(Propagation::Stop)
    }

    pub(super) fn item_navigation(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK)
            && !self.view.item_view_has_focus()
            && !event.header_left_boundary
        {
            return Some(Propagation::Proceed);
        }
        let jump = if event.press.is(FIRST_ITEM_UP) {
            Some(-1)
        } else if event.press.is(LAST_ITEM_DOWN) {
            Some(1)
        } else {
            None
        };
        if let Some(direction) = jump
            && self.view.jump_selection(direction)
        {
            return Some(Propagation::Stop);
        }
        self.single_pane_navigation(event)
            .or_else(|| self.item_commands(browser, event))
            .or_else(|| self.selection_navigation(browser, event))
            .or_else(|| self.directory_navigation(browser, event))
    }

    fn single_pane_navigation(&self, event: &KeyEvent) -> KeyResult {
        if !self.view.item_view_has_focus() {
            return None;
        }
        let action = single_pane_arrow_action(
            self.view.view_mode(),
            &event.press,
            self.view.at_left_edge(),
            self.top_bar.sidebar_toggle().is_active(),
        )?;
        let action = match action {
            SinglePaneArrow::Sidebar if self.arrows_scoped_to_content() => SinglePaneArrow::Stay,
            other => other,
        };
        Some(match action {
            SinglePaneArrow::Native => self.native_selection(event),
            SinglePaneArrow::Stay => Propagation::Stop,
            SinglePaneArrow::Sidebar => {
                self.enter_sidebar(event);
                Propagation::Stop
            }
        })
    }

    fn native_selection(&self, event: &KeyEvent) -> Propagation {
        if event.press.is(UP)
            && !self.arrows_scoped_to_content()
            && self.view.focus_header_from_top_item()
        {
            return Propagation::Stop;
        }
        self.view.commit_selection();
        let started_from_empty =
            event.without(Modifiers::CONTROL_MASK) && self.view.resume_native_selection();
        if event.shift() && started_from_empty {
            return Propagation::Stop;
        }
        if let Some(direction) = sidebar_focus_direction(&event.press, Modifiers::empty())
            && self.view.cross_type_group(direction, false)
        {
            Propagation::Stop
        } else if let Some(arrow) = event.vim_arrow() {
            crate::ui::focus_navigation::activate_native_arrow(&self.window, arrow);
            Propagation::Stop
        } else {
            Propagation::Proceed
        }
    }

    fn item_commands(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        let press = &event.press;
        if self.view.item_view_has_focus()
            && (press.is_any(&[FIRST_ITEM, LAST_ITEM].map(|command| command.in_map(Map::Default)))
                || press.is_any(&[EXTEND_TO_FIRST, EXTEND_TO_LAST]))
        {
            self.view.commit_selection();
            let jump = if press.is(FIRST_ITEM.in_map(Map::Default)) {
                Some(-1)
            } else if press.is(LAST_ITEM.in_map(Map::Default)) {
                Some(1)
            } else {
                None
            };
            if let Some(direction) = jump
                && self.view.jump_selection(direction)
            {
                return Some(Propagation::Stop);
            }
        }
        let page: [(&dyn Match, _, _); 4] = [
            (&PAGE_UP.in_map(Map::Default).ignoring(SUPER), -1, false),
            (&PAGE_DOWN.in_map(Map::Default).ignoring(SUPER), 1, false),
            (&EXTEND_PAGE_UP.ignoring(SUPER), -1, true),
            (&EXTEND_PAGE_DOWN.ignoring(SUPER), 1, true),
        ];
        let page = page.into_iter().find(|(key, _, _)| press.is(key));
        if let Some((_, direction, extend)) = page
            && self.view.page_selection(direction, extend)
        {
            return Some(Propagation::Stop);
        }
        if press.is(COPY_PATH) {
            self.view.copy_path();
        } else if press.is(PIN) {
            self.view.pin_focused();
        } else if press.is(PREVIEW.in_map(Map::Default).ignoring(SHIFT.union(SUPER))) {
            self.view.cancel_pending_click_rename();
            let activated_directory =
                press.is(PREVIEW.in_map(Map::Default)) && self.view.activate_directory_on_space();
            if !activated_directory {
                self.preview.toggle(
                    preview_target(browser.focused_entry()),
                    browser.active_depth(),
                );
            }
        } else if press.is(BACKSPACE.ignoring(SHIFT.union(SUPER))) {
            self.view.navigate_up();
        } else {
            return None;
        }
        Some(Propagation::Stop)
    }

    fn selection_navigation(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        // Shift+Up/Down extend the selection even with Ctrl, Alt or Super held.
        const EXTENDS: Modifiers = Modifiers::CONTROL_MASK
            .union(Modifiers::ALT_MASK)
            .union(SUPER);
        let press = &event.press;
        if press.is(EXTEND_UP.ignoring(EXTENDS)) {
            browser.extend_selection(-1);
            return Some(Propagation::Stop);
        }
        if press.is(EXTEND_DOWN.ignoring(EXTENDS)) {
            browser.extend_selection(1);
            return Some(Propagation::Stop);
        }
        if press.is(UP
            .ignoring(Modifiers::CONTROL_MASK.union(SUPER))
            .or_alternates())
            && !self.arrows_scoped_to_content()
            && self.view.focus_header_from_top_item()
        {
            return Some(Propagation::Stop);
        }
        None
    }

    fn directory_navigation(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        let press = &event.press;
        if self.view.item_view_has_focus() && !event.text_has_focus() {
            // Ctrl wins over Shift, so Ctrl+Shift+Enter opens a tab.
            let target = if press.is(OPEN_IN_NEW_TAB.ignoring(SHIFT)) {
                Some(OpenInNewTarget::Tab)
            } else if press.is(OPEN_IN_NEW_WINDOW) {
                Some(OpenInNewTarget::Window)
            } else {
                None
            };
            if let Some(target) = target
                && self.open_focused_in_new(browser, target)
            {
                return Some(Propagation::Stop);
            }
        }
        // Ahead of the Ctrl and Super guard, which a user's binding may hold.
        let jumped = if press.is(HISTORY_BACK.in_map(Map::Default).ignoring(SHIFT)) {
            browser.back();
            true
        } else if press.is(HISTORY_FORWARD.in_map(Map::Default).ignoring(SHIFT)) {
            browser.forward();
            true
        } else if press.is(PARENT_FOLDER.ignoring(SHIFT)) {
            browser.parent();
            true
        } else if press.is(HOME_FOLDER.ignoring(SHIFT)) {
            browser.navigate(Location::local(home_directory()));
            true
        } else {
            false
        };
        if jumped {
            return Some(Propagation::Stop);
        }
        if !event.without(Modifiers::CONTROL_MASK | SUPER) {
            return Some(Propagation::Proceed);
        }
        if press.is(DOWN.ignoring(SHIFT).or_alternates()) {
            browser.move_selection(1);
        } else if press.is(UP.ignoring(SHIFT).or_alternates()) {
            browser.move_selection(-1);
        } else if press.is(LEFT.ignoring(SHIFT).or_alternates()) {
            self.navigate_left(event);
        } else if press.is(RIGHT.ignoring(SHIFT)) && self.view.view_mode() == BrowserMode::Columns {
            browser.enter_focused_directory();
        } else if press.is_any(&[RIGHT.alternates(), ENTER_ANY_SHIFT]) {
            self.view.activate_focused();
        } else if press.is(ESCAPE_ANY_SHIFT) {
            browser.escape();
        } else {
            return None;
        }
        Some(Propagation::Stop)
    }

    fn navigate_left(&self, event: &KeyEvent) {
        if self.view.first_column_has_focus()
            && self.top_bar.sidebar_toggle().is_active()
            && !self.arrows_scoped_to_content()
        {
            self.enter_sidebar(event);
        } else {
            self.view.navigate_left();
        }
    }

    pub(super) fn tenxer_icons(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        if self.view.view_mode() != BrowserMode::Icons || !self.view.item_view_has_focus() {
            return false;
        }
        if self.tenxer_open_focused_in_new(browser, press) {
            return true;
        }
        let search = self.view.selected_search_results().is_some();
        if let Some(command) = media_track(press) {
            return self.preview.media_key(command);
        }
        // Before the search branch: the Shift that types + counts as held there.
        if let Some(change) = ThumbnailResize::for_tenxer_key(press) {
            return self.view.resize_icons_thumbnails(change);
        }
        // Of the modified keys, only these act on hits, whatever keys they are bound to.
        if search && !press.modifiers().is_empty() {
            return (press.is(INVERT_SELECTION) && self.view.invert_filter_results())
                || (self.view.listing_search_active() && self.tenxer_control_page(press))
                || ((press.is_any(&[LAST_ITEM, VISUAL_UNSET]) || press.is_any(&EXTEND_KEYS))
                    && self.tenxer_shifted(press));
        }
        if self.tenxer_control_page(press)
            || self.tenxer_first_or_last(press)
            || self.tenxer_selection_command(press)
            || self.tenxer_alt_navigation(browser, press)
            || self.tenxer_shifted(press)
        {
            return true;
        }
        if let Some(arrow) = crate::ui::focus_navigation::spatial_arrow(press) {
            return self.move_icon_cursor(browser, arrow.key(), search);
        }
        if press.is_any(&SWALLOWED_ON_HITS) && self.view.listing_search_active() {
            return true;
        }
        if press.is(FIRST_ITEM) && !search {
            self.jump_displayed(-1);
        } else if press.is(LAST_ITEM) && !search {
            self.jump_displayed(1);
        } else if press.is(BACKSPACE) && !search {
            self.go_parent();
        } else if press.is(OPEN) || press.is(ENTER) {
            self.activate_focused(browser);
        } else if press.is(PREVIEW) {
            if !self.toggle_file_preview(browser) {
                self.view.toggle_folder_peek();
            }
        } else if press.is(PAGE_UP) && !search {
            self.view.page_displayed_cursor(-1, false);
        } else if press.is(PAGE_DOWN) && !search {
            self.view.page_displayed_cursor(1, false);
        } else if press.is(SPACE) && !self.selection_keys_blocked() {
            self.toggle_tenxer_cursor();
        } else if press.is(VISUAL_SELECT) && !self.selection_keys_blocked() {
            self.toggle_visual(VisualKind::Select);
        } else if press.is(GO) {
            self.shortcuts.arm_chord(Chord::Go);
        } else {
            return false;
        }
        true
    }

    /// Grid motion keeps every committed fill, including a singleton and an empty
    /// inverted fill. A load cursor still follows GTK's spatial move.
    fn move_icon_cursor(&self, browser: &Rc<Browser>, arrow: Key, search: bool) -> bool {
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
    fn move_icon_range(&self, browser: &Rc<Browser>, arrow: Key) {
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

    pub(super) fn tenxer_listing(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        if self.view.view_mode() == BrowserMode::Icons || !self.view.item_view_has_focus() {
            return false;
        }
        self.tenxer_open_focused_in_new(browser, press)
            || self.tenxer_control_page(press)
            || self.tenxer_first_or_last(press)
            || self.tenxer_selection_command(press)
            || self.tenxer_alt_navigation(browser, press)
            || self.tenxer_shifted(press)
            || self.tenxer_plain(browser, press)
    }

    fn tenxer_selection_command(&self, press: &KeyPress) -> bool {
        if press.is(INVERT_SELECTION) {
            return self.invert_selection();
        }
        press.is(SELECT_ALL)
            && self.view.selected_search_results().is_none()
            && self.fill_selection(self.view.select_focused_pane())
    }

    /// Inverts the filter's hits or the focused pane; false while search hits
    /// own the selection.
    pub(super) fn invert_selection(&self) -> bool {
        self.view.invert_filter_results()
            || (self.view.selected_search_results().is_none()
                && self.fill_selection(self.view.invert_focused_pane()))
    }

    fn fill_selection(&self, filled: bool) -> bool {
        if !filled {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to select"));
        }
        true
    }

    fn toggle_visual(&self, kind: VisualKind) {
        if !self.view.toggle_visual(kind) {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to select"));
        }
    }

    fn toggle_tenxer_cursor(&self) {
        if !self.view.toggle_cursor_and_advance() {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to select"));
        }
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

    /// Search hits shown in the listing swallow paging.
    pub(super) fn page_cursor(&self, direction: i32, half: bool) {
        if !self.view.listing_search_active() {
            self.view.page_displayed_cursor(direction, half);
        }
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
        if press.is_any(&SWALLOWED_ON_HITS) && self.view.listing_search_active() {
            return true;
        }
        if press.is(DOWN.or_alternates()) {
            self.view.move_displayed_cursor(1, 1);
        } else if press.is(UP.or_alternates()) {
            self.view.move_displayed_cursor(-1, 1);
        } else if press.is(FIRST_ITEM) {
            self.jump_displayed(-1);
        } else if press.is(LAST_ITEM) {
            self.jump_displayed(1);
        } else if press.is(LEFT.or_alternates()) && self.view.listing_search_active() {
            self.view.dismiss_listing_search();
        } else if press.is(LEFT.or_alternates()) || press.is(BACKSPACE) {
            self.go_parent();
        } else if press.is(RIGHT.or_alternates()) {
            self.enter_preview(browser);
        } else if press.is(OPEN) || press.is(ENTER) {
            self.activate_focused(browser);
        } else if press.is(GO) {
            self.shortcuts.arm_chord(Chord::Go);
        } else if let Some(command) = media_track(press) {
            return self.preview.media_key(command);
        } else if press.is(PREVIEW) {
            if !self.toggle_file_preview(browser) {
                if self.view.view_mode() == BrowserMode::Columns {
                    self.open_miller_child(browser);
                } else {
                    self.view.toggle_folder_peek();
                }
            }
        } else if press.is(PAGE_UP) {
            self.view.page_displayed_cursor(-1, false);
        } else if press.is(PAGE_DOWN) {
            self.view.page_displayed_cursor(1, false);
        } else if press.is(SPACE) && !self.selection_keys_blocked() {
            self.toggle_tenxer_cursor();
        } else if press.is(VISUAL_SELECT) && !self.selection_keys_blocked() {
            self.toggle_visual(VisualKind::Select);
        } else {
            return false;
        }
        true
    }

    fn jump_displayed(&self, direction: i32) {
        self.view.move_displayed_cursor(direction, usize::MAX);
    }

    fn go_parent(&self) {
        self.view.keyboard_navigation();
        self.view.navigate_up();
    }

    fn go_back(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        browser.back();
    }

    fn go_forward(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        browser.forward();
    }

    /// Opens the focused result while results replace the directory, and
    /// nothing when there is none rather than the hidden directory's cursor.
    fn activate_focused(&self, browser: &Rc<Browser>) {
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

    fn selection_keys_blocked(&self) -> bool {
        self.view.selected_search_results().is_some() && !self.view.results_replace_listing()
    }

    /// Opens the directory under the cursor, or the directory hit, in the
    /// next column without moving focus.
    fn open_miller_child(&self, browser: &Rc<Browser>) {
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

fn media_track(press: &KeyPress) -> Option<MediaCommand> {
    if press.is(TENXER_MEDIA_PREVIOUS) {
        Some(MediaCommand::Previous)
    } else if press.is(TENXER_MEDIA_NEXT) {
        Some(MediaCommand::Next)
    } else {
        None
    }
}

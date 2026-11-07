// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
    prelude::*,
};

use super::{Dispatcher, KeyEvent, KeyResult};
use crate::{
    app::Browser,
    model::Location,
    ui::{
        browser_modes::BrowserMode,
        keys::{Binding, Command, KeyPress, Map, Match, bindings::*},
        preview::preview_target,
        window::{
            SinglePaneArrow, home_directory, sidebar_focus_direction, single_pane_arrow_action,
        },
    },
};

mod preview_keys;
mod tenxer_cursor;
mod tenxer_items;

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
        let press = &event.press;
        if press.is(BACKSPACE.ignoring(SHIFT.union(SUPER)))
            && self.view.dismiss_empty_focused_filter()
        {
            return Some(Propagation::Stop);
        }
        if let Some(permanent) = delete_kind(press)
            && self.confirms_delete(event, permanent)
        {
            return Some(Propagation::Stop);
        }
        if press.is(ESCAPE_ANY_SHIFT) && !event.text_has_focus() {
            return self.dismiss_preview_or_selection(browser);
        }
        None
    }

    /// Delete keys act on files, never on text being edited or from inside the
    /// preview.
    fn confirms_delete(&self, event: &KeyEvent, permanent: bool) -> bool {
        if self.view.filter_has_focus() || event.text_has_focus() || self.preview_has_focus(event) {
            return false;
        }
        self.view.confirm_delete(permanent)
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

    pub(super) fn item_navigation(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if self.items_unfocused_for(event) {
            return Some(Propagation::Proceed);
        }
        if let Some(direction) = edge_jump(&event.press)
            && self.view.jump_selection(direction)
        {
            return Some(Propagation::Stop);
        }
        self.single_pane_navigation(event)
            .or_else(|| self.item_commands(browser, event))
            .or_else(|| self.selection_navigation(browser, event))
            .or_else(|| self.directory_navigation(browser, event))
    }

    /// Unmodified keys outside the items, and away from the header's left edge,
    /// are left to GTK.
    fn items_unfocused_for(&self, event: &KeyEvent) -> bool {
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK) {
            return false;
        }
        !self.view.item_view_has_focus() && !event.header_left_boundary
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
        if event.press.is(UP) && self.focus_header_from_top() {
            return Propagation::Stop;
        }
        self.view.commit_selection();
        let started_from_empty =
            event.without(Modifiers::CONTROL_MASK) && self.view.resume_native_selection();
        if event.shift() && started_from_empty {
            return Propagation::Stop;
        }
        self.native_arrow(event)
    }

    /// Up from the top item reaches the header unless arrows stay in the content.
    fn focus_header_from_top(&self) -> bool {
        !self.arrows_scoped_to_content() && self.view.focus_header_from_top_item()
    }

    fn native_arrow(&self, event: &KeyEvent) -> Propagation {
        if let Some(direction) = sidebar_focus_direction(&event.press, Modifiers::empty())
            && self.view.cross_type_group(direction, false)
        {
            return Propagation::Stop;
        }
        let Some((arrow, alias)) = event
            .vim_arrow()
            .map(|arrow| (arrow, true))
            .or_else(|| plain_arrow(&event.press).map(|arrow| (arrow, false)))
        else {
            return Propagation::Proceed;
        };
        // Stopping here hides the key from the browser's own capture handler.
        self.view.cancel_pending_click_rename();
        let moved = self.run_arrow_binding(arrow);
        // An alias passed on would type its letter.
        if moved || alias {
            Propagation::Stop
        } else {
            Propagation::Proceed
        }
    }

    /// Runs GTK's arrow binding. GTK's plain move resets its own Shift range origin; the
    /// model's anchor, which Shift+page keys and Shift+click extend from, follows it
    /// when the move replaced and changed the selection, and not when GTK kept a range at
    /// an edge.
    fn run_arrow_binding(&self, arrow: Key) -> bool {
        let browser = self.view.browser();
        let selection = || {
            browser
                .focused_item()
                .map(|(depth, position, _)| (depth, position, browser.selected_positions(depth)))
        };
        let before = selection();
        let moved = crate::ui::focus_navigation::activate_native_arrow(&self.window, arrow);
        if moved
            && let Some((depth, position, selected)) = selection()
            && replaced_selection(before, depth, position, &selected)
        {
            browser.set_selection_anchor(depth, position);
        }
        moved
    }

    fn item_commands(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        let press = &event.press;
        if self.first_or_last_item(press) {
            return Some(Propagation::Stop);
        }
        if let Some((direction, extend)) = page_key(press)
            && self.view.page_selection(direction, extend)
        {
            return Some(Propagation::Stop);
        }
        self.item_action(browser, press)
            .then_some(Propagation::Stop)
    }

    fn first_or_last_item(&self, press: &KeyPress) -> bool {
        if !self.view.item_view_has_focus() {
            return false;
        }
        let Some((direction, extend)) = edge_key(press) else {
            return false;
        };
        self.view.commit_selection();
        if extend {
            self.view.view_mode() == BrowserMode::Columns
                && self.view.extend_selection_to_edge(direction)
        } else {
            self.view.jump_selection(direction)
        }
    }

    fn item_action(&self, browser: &Browser, press: &KeyPress) -> bool {
        if press.is(COPY_PATH) {
            self.view.copy_path();
        } else if press.is(PIN) {
            self.view.pin_focused();
        } else if press.is(PREVIEW.in_map(Map::Default).ignoring(SHIFT.union(SUPER))) {
            self.toggle_space_preview(browser, press);
        } else if press.is(BACKSPACE.ignoring(SHIFT.union(SUPER))) {
            self.view.navigate_up();
        } else {
            return false;
        }
        true
    }

    fn toggle_space_preview(&self, browser: &Browser, press: &KeyPress) {
        self.view.cancel_pending_click_rename();
        let activated_directory =
            press.is(PREVIEW.in_map(Map::Default)) && self.view.activate_directory_on_space();
        if activated_directory {
            return;
        }
        let focused = browser.focused_entry();
        // A folder, or a column with nothing focused, has nothing to preview: Space
        // only closes a drawer that is already open there.
        if !self.preview.is_open() && focused.as_ref().is_none_or(|entry| entry.is_directory()) {
            return;
        }
        self.preview
            .toggle(preview_target(focused), browser.active_depth());
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
        let up = press.is(UP
            .ignoring(Modifiers::CONTROL_MASK.union(SUPER))
            .or_alternates());
        (up && self.focus_header_from_top()).then_some(Propagation::Stop)
    }

    fn directory_navigation(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        let press = &event.press;
        if self.opens_focused_in_new(browser, event) || self.makes_focused_column_first(event) {
            return Some(Propagation::Stop);
        }
        // Ahead of the Ctrl and Super guard, which a user's binding may hold.
        if history_jump(browser, press) {
            return Some(Propagation::Stop);
        }
        if !event.without(Modifiers::CONTROL_MASK | SUPER) {
            return Some(Propagation::Proceed);
        }
        self.directory_key(browser, event)
            .then_some(Propagation::Stop)
    }

    fn opens_focused_in_new(&self, browser: &Rc<Browser>, event: &KeyEvent) -> bool {
        if !self.view.item_view_has_focus() || event.text_has_focus() {
            return false;
        }
        // Ctrl wins over Shift, so Ctrl+Shift+Enter opens a tab.
        let press = &event.press;
        let target = if press.is(OPEN_IN_NEW_TAB.ignoring(SHIFT)) {
            OpenInNewTarget::Tab
        } else if press.is(OPEN_IN_NEW_WINDOW) {
            OpenInNewTarget::Window
        } else {
            return false;
        };
        self.open_focused_in_new(browser, target)
    }

    fn makes_focused_column_first(&self, event: &KeyEvent) -> bool {
        event.press.is(MAKE_FIRST_COLUMN)
            && self.view.item_view_has_focus()
            && !event.text_has_focus()
            && self.chooser.is_none()
            && self.view.make_focused_column_first()
    }

    fn directory_key(&self, browser: &Rc<Browser>, event: &KeyEvent) -> bool {
        let press = &event.press;
        if press.is(DOWN.ignoring(SHIFT).or_alternates()) {
            browser.move_selection(1);
        } else if press.is(UP.ignoring(SHIFT).or_alternates()) {
            browser.move_selection(-1);
        } else if press.is(LEFT.ignoring(SHIFT).or_alternates()) {
            self.navigate_left(event);
        } else {
            return self.directory_enter_or_escape(browser, press);
        }
        true
    }

    fn directory_enter_or_escape(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        if press.is(RIGHT.ignoring(SHIFT)) && self.view.view_mode() == BrowserMode::Columns {
            browser.enter_focused_directory();
        } else if press.is_any(&[RIGHT.alternates(), ENTER_ANY_SHIFT]) {
            self.view.activate_focused();
        } else if press.is(ESCAPE_ANY_SHIFT) {
            browser.escape();
        } else {
            return false;
        }
        true
    }

    fn navigate_left(&self, event: &KeyEvent) {
        if self.sidebar_left_of_first_column() {
            self.enter_sidebar(event);
        } else {
            self.view.navigate_left();
        }
    }

    fn sidebar_left_of_first_column(&self) -> bool {
        if !self.view.first_column_has_focus() {
            return false;
        }
        self.top_bar.sidebar_toggle().is_active() && !self.arrows_scoped_to_content()
    }
}

fn delete_kind(press: &KeyPress) -> Option<bool> {
    const DELETE_MODIFIERS: Modifiers = Modifiers::CONTROL_MASK
        .union(Modifiers::ALT_MASK)
        .union(SUPER);
    if press.is(MOVE_TO_TRASH
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
    }
}

fn edge_jump(press: &KeyPress) -> Option<i32> {
    if press.is(FIRST_ITEM_UP) {
        Some(-1)
    } else if press.is(LAST_ITEM_DOWN) {
        Some(1)
    } else {
        None
    }
}

/// Home and End, and with Shift whether they extend the selection.
fn edge_key(press: &KeyPress) -> Option<(i32, bool)> {
    let edge: [(&dyn Match, _, _); 4] = [
        (&FIRST_ITEM.in_map(Map::Default), -1, false),
        (&LAST_ITEM.in_map(Map::Default), 1, false),
        (&EXTEND_TO_FIRST, -1, true),
        (&EXTEND_TO_LAST, 1, true),
    ];
    edge.into_iter()
        .find(|(key, _, _)| press.is(key))
        .map(|(_, direction, extend)| (direction, extend))
}

/// The move left only `position` selected where the selection before it differed, rather
/// than GTK keeping a range at an edge. `before` is the cursor's depth, position and
/// selection ahead of the move.
fn replaced_selection(
    before: Option<(usize, usize, Vec<usize>)>,
    depth: usize,
    position: usize,
    selected: &[usize],
) -> bool {
    selected == [position]
        && before.is_none_or(|(before_depth, _, before_selected)| {
            before_depth != depth || before_selected != selected
        })
}

/// An unmodified arrow key itself, not an alias read as one.
fn plain_arrow(press: &KeyPress) -> Option<Key> {
    [UP, DOWN, LEFT, RIGHT]
        .into_iter()
        .find(|arrow| !press.aliased() && press.is(*arrow))
        .and_then(Command::canonical)
        .map(Binding::key)
}

fn page_key(press: &KeyPress) -> Option<(i32, bool)> {
    let page: [(&dyn Match, _, _); 4] = [
        (&PAGE_UP.in_map(Map::Default).ignoring(SUPER), -1, false),
        (&PAGE_DOWN.in_map(Map::Default).ignoring(SUPER), 1, false),
        (&EXTEND_PAGE_UP.ignoring(SUPER), -1, true),
        (&EXTEND_PAGE_DOWN.ignoring(SUPER), 1, true),
    ];
    page.into_iter()
        .find(|(key, _, _)| press.is(key))
        .map(|(_, direction, extend)| (direction, extend))
}

/// History, parent and home jumps, whatever modifiers their bindings hold.
fn history_jump(browser: &Rc<Browser>, press: &KeyPress) -> bool {
    if press.is(HISTORY_BACK.in_map(Map::Default).ignoring(SHIFT)) {
        browser.back();
    } else if press.is(HISTORY_FORWARD.in_map(Map::Default).ignoring(SHIFT)) {
        browser.forward();
    } else if press.is(PARENT_FOLDER.ignoring(SHIFT)) {
        browser.parent();
    } else if press.is(HOME_FOLDER.ignoring(SHIFT)) {
        browser.navigate(Location::local(home_directory()));
    } else {
        return false;
    }
    true
}

// SPDX-License-Identifier: MIT

use gtk::{gdk::ModifierType as Modifiers, glib::Propagation, prelude::*};

use super::{Dispatcher, KeyEvent, KeyResult};
use crate::{
    app::Browser,
    ui::{
        browser_modes::BrowserMode,
        focus_navigation::{
            FOCUS_MODIFIERS, arrow_direction, plain_tab_direction, vim_focus_direction,
        },
        keys::{SHORTCUT_MODIFIERS, bindings::*},
        window::sidebar_focus_direction,
    },
};

impl Dispatcher {
    pub(super) fn focus_navigation(&self, browser: &Browser, event: &mut KeyEvent) -> KeyResult {
        if self.type_to_search.preferences.tenxer_mode() {
            if let Some(result) = self.tenxer_pane_focus(browser, event) {
                return Some(result);
            }
            return self.popover_navigation(event);
        }
        self.popover_navigation(event)
            .or_else(|| self.top_bar_navigation(browser, event))
            .or_else(|| self.header_navigation(event))
            .or_else(|| self.sidebar_navigation(browser, event))
    }

    /// Runs before inline editing so Tab from a rename field in Columns also leaves the strip.
    pub(super) fn default_tab_navigation(&self, event: &KeyEvent) -> KeyResult {
        if self.type_to_search.preferences.tenxer_mode() || self.focus_in_popover() {
            return None;
        }
        let direction = plain_tab_direction(&event.press)?;
        self.view
            .default_tab(direction)
            .then_some(Propagation::Stop)
    }

    fn tenxer_pane_focus(&self, browser: &Browser, event: &mut KeyEvent) -> KeyResult {
        if event.text_has_focus() || self.focus_in_popover() {
            return None;
        }
        let tab = plain_tab_direction(&event.press);
        if self.view.item_view_has_focus() && tab == Some(gtk::DirectionType::TabForward) {
            if self.top_bar.focus_first() {
                self.window.set_focus_visible(true);
            }
            return Some(Propagation::Stop);
        }
        let arrow = arrow_direction(&event.press, SHORTCUT_MODIFIERS);
        let vim = vim_focus_direction(&event.press);
        if tab.is_none() && arrow.is_none() && vim.is_none() {
            return None;
        }
        let panes = self.view.widget();
        let inside = crate::ui::focus_navigation::contains_widget(&panes, event.focused.as_ref());
        if !inside {
            browser.focus_active();
            return Some(Propagation::Stop);
        }
        if tab.is_some() {
            return Some(Propagation::Stop);
        }
        if self.view.header_actions_have_focus() {
            return self.confined_header_navigation(event);
        }
        if self.view.item_view_has_focus() {
            return None;
        }
        if let Some(direction) = arrow.or(vim) {
            self.view.widget().child_focus(direction);
        }
        Some(Propagation::Stop)
    }

    fn confined_header_navigation(&self, event: &KeyEvent) -> KeyResult {
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK) {
            return None;
        }
        let press = &event.press;
        if press.is_any(&[
            LEFT.alternates(),
            DOWN.ignoring(FOCUS_MODIFIERS).or_alternates(),
        ]) {
            self.view.focus_items_from_header();
        } else if press.is(LEFT.ignoring(FOCUS_MODIFIERS)) {
            self.view.move_header_focus(gtk::DirectionType::Left);
        } else if press.is(RIGHT.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            self.view.move_header_focus(gtk::DirectionType::Right);
        } else if press.is(ENTER.ignoring(FOCUS_MODIFIERS))
            || press.is(SPACE.ignoring(FOCUS_MODIFIERS))
        {
            return None;
        }
        Some(Propagation::Stop)
    }

    fn popover_navigation(&self, event: &KeyEvent) -> KeyResult {
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK) {
            return None;
        }
        let popover = event
            .focused
            .as_ref()
            .and_then(|focused| focused.ancestor(gtk::Popover::static_type()))
            .and_downcast::<gtk::Popover>()?;
        if popover.has_css_class("column-popover")
            && let Some(direction) = vim_focus_direction(&event.press)
        {
            popover.child_focus(direction);
            return Some(Propagation::Stop);
        }
        Some(Propagation::Proceed)
    }

    fn top_bar_navigation(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        if !self.top_bar.has_focus() || event.text_has_focus() {
            return None;
        }
        let press = &event.press;
        if press.is(LEFT.ignoring(Modifiers::SUPER_MASK)) {
            self.top_bar.move_focus(gtk::DirectionType::Left);
        } else if press.is(RIGHT.ignoring(Modifiers::SUPER_MASK)) {
            self.top_bar.move_focus(gtk::DirectionType::Right);
        } else if press.is(DOWN.ignoring(Modifiers::SUPER_MASK)) {
            if !self.top_bar.return_to_sidebar() {
                browser.focus_active();
            }
        } else if !press.is(UP.ignoring(Modifiers::SUPER_MASK)) {
            return None;
        }
        Some(Propagation::Stop)
    }

    fn header_navigation(&self, event: &mut KeyEvent) -> KeyResult {
        if !self.view.header_actions_have_focus()
            || !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK)
        {
            return None;
        }
        let press = event.press;
        if press.is(LEFT.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            return self.header_left(event);
        }
        if press.is(RIGHT.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            self.view.move_header_focus(gtk::DirectionType::Right);
        } else if press.is(DOWN.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            self.view.focus_items_from_header();
        } else {
            return None;
        }
        Some(Propagation::Stop)
    }

    fn header_left(&self, event: &mut KeyEvent) -> KeyResult {
        if self.view.move_header_focus(gtk::DirectionType::Left) {
            return Some(Propagation::Stop);
        }
        if self.view.view_mode() == BrowserMode::Columns {
            event.header_left_boundary = true;
            return None;
        }
        if self.top_bar.sidebar_toggle().is_active() {
            self.enter_sidebar(event);
        }
        Some(Propagation::Stop)
    }

    fn sidebar_navigation(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        if !self.sidebar.contains(&event.focused)
            || !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK)
        {
            return None;
        }
        let direction = sidebar_focus_direction(&event.press, FOCUS_MODIFIERS)?;
        if direction == gtk::DirectionType::Right {
            self.sidebar.restore(browser, true);
        } else if direction == gtk::DirectionType::Up && !event.shift() {
            self.top_bar.move_up_from_sidebar();
        } else {
            self.sidebar.widget.child_focus(direction);
        }
        Some(Propagation::Stop)
    }
}

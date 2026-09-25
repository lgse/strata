// SPDX-License-Identifier: MIT

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
    prelude::*,
};

use super::{Dispatcher, KeyEvent, KeyResult};
use crate::{
    app::Browser,
    ui::{
        browser_modes::BrowserMode,
        window::{sidebar_focus_direction, vim_focus_direction},
    },
};

impl Dispatcher {
    pub(super) fn focus_navigation(&self, browser: &Browser, event: &mut KeyEvent) -> KeyResult {
        if self.type_to_search.preferences.omastrata_mode() {
            if let Some(result) = self.omastrata_pane_focus(browser, event) {
                return Some(result);
            }
            return self.popover_navigation(event);
        }
        self.popover_navigation(event)
            .or_else(|| self.top_bar_navigation(browser, event))
            .or_else(|| self.header_navigation(event))
            .or_else(|| self.sidebar_navigation(browser, event))
    }

    /// Tab and arrows stay inside the Columns, List, and Icons panes.
    /// Focus that is already on surrounding chrome returns to the file list.
    fn omastrata_pane_focus(&self, browser: &Browser, event: &mut KeyEvent) -> KeyResult {
        if event.text_has_focus() || self.focus_in_popover() {
            return None;
        }
        let tab =
            crate::ui::focus_navigation::plain_tab_direction(event.key, event.modifiers).is_some();
        let arrow = crate::ui::focus_navigation::arrow_direction(event.key);
        let vim = vim_focus_direction(event.key);
        if !tab && arrow.is_none() && vim.is_none() {
            return None;
        }
        let panes = self.view.widget();
        let inside = crate::ui::focus_navigation::contains_widget(&panes, event.focused.as_ref());
        if !inside {
            browser.focus_active();
            return (tab || arrow.is_none()).then_some(Propagation::Stop);
        }
        if tab {
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
        match event.key {
            Key::h | Key::Left => {
                self.view.move_header_focus(gtk::DirectionType::Left);
            }
            Key::l | Key::Right => {
                self.view.move_header_focus(gtk::DirectionType::Right);
            }
            Key::j | Key::Down => {
                self.view.focus_items_from_header();
            }
            _ => {}
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
            && let Some(direction) = vim_focus_direction(event.key)
        {
            popover.child_focus(direction);
            return Some(Propagation::Stop);
        }
        Some(Propagation::Proceed)
    }

    fn top_bar_navigation(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        if !self.top_bar.has_focus()
            || event.text_has_focus()
            || !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SHIFT_MASK)
        {
            return None;
        }
        match event.key {
            Key::Left => {
                self.top_bar.move_focus(gtk::DirectionType::Left);
            }
            Key::Right => {
                self.top_bar.move_focus(gtk::DirectionType::Right);
            }
            Key::Down => {
                if !self.top_bar.return_to_sidebar() {
                    browser.focus_active();
                }
            }
            Key::Up => {}
            _ => return None,
        }
        Some(Propagation::Stop)
    }

    fn header_navigation(&self, event: &mut KeyEvent) -> KeyResult {
        if !self.view.header_actions_have_focus()
            || !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK)
        {
            return None;
        }
        match event.key {
            Key::h | Key::Left => return self.header_left(event),
            Key::l | Key::Right => {
                self.view.move_header_focus(gtk::DirectionType::Right);
            }
            Key::j | Key::Down => {
                self.view.focus_items_from_header();
            }
            _ => return None,
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
        let direction = sidebar_focus_direction(event.key)?;
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

// SPDX-License-Identifier: GPL-3.0-or-later

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
        self.popover_navigation(event)
            .or_else(|| self.top_bar_navigation(browser, event))
            .or_else(|| self.header_navigation(event))
            .or_else(|| self.sidebar_navigation(browser, event))
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
            self.sidebar.enter(&event.focused);
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

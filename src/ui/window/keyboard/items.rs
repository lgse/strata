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
        preview::preview_target,
        window::{
            SinglePaneArrow, home_directory, jump_direction, page_direction,
            sidebar_focus_direction, single_pane_arrow_action,
        },
    },
};

impl Dispatcher {
    pub(super) fn dismissal(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        if event.key == Key::BackSpace
            && event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK)
            && self.view.dismiss_empty_focused_filter()
        {
            return Some(Propagation::Stop);
        }
        if event.key == Key::Delete
            && !self.view.filter_has_focus()
            && !event.text_has_focus()
            && self.view.confirm_delete(event.shift())
        {
            return Some(Propagation::Stop);
        }
        if event.key == Key::Escape
            && event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            && !event.text_has_focus()
        {
            return self.dismiss_preview_or_selection(browser);
        }
        None
    }

    fn dismiss_preview_or_selection(&self, browser: &Browser) -> KeyResult {
        if self.preview.is_enabled() {
            self.preview.close();
            return Some(Propagation::Stop);
        }
        // Transient surfaces may return focus to pane chrome rather than an item.
        (browser.close_peek() || browser.clear_active_selection()).then_some(Propagation::Stop)
    }

    pub(super) fn item_navigation(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK)
            && !self.view.item_view_has_focus()
            && !event.header_left_boundary
        {
            return Some(Propagation::Proceed);
        }
        if let Some(direction) = jump_direction(event.key, event.modifiers)
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
            event.key,
            event.modifiers,
            self.view.at_left_edge(),
            self.top_bar.sidebar_toggle().is_active(),
        )?;
        Some(match action {
            SinglePaneArrow::Native => self.native_selection(event),
            SinglePaneArrow::Stay => Propagation::Stop,
            SinglePaneArrow::Sidebar => {
                self.sidebar.enter(&event.focused);
                Propagation::Stop
            }
        })
    }

    fn native_selection(&self, event: &KeyEvent) -> Propagation {
        if event.without(Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK)
            && event.key == Key::Up
            && self.view.focus_header_from_top_item()
        {
            return Propagation::Stop;
        }
        self.view.commit_selection();
        let started_from_empty = !event.control() && self.view.resume_native_selection();
        if event.shift() && started_from_empty {
            return Propagation::Stop;
        }
        if event.without(Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK)
            && let Some(direction) = sidebar_focus_direction(event.key)
            && self.view.cross_type_group(direction, false)
        {
            Propagation::Stop
        } else if event.vim_navigation {
            crate::ui::focus_navigation::activate_native_arrow(&self.window, event.key);
            Propagation::Stop
        } else {
            Propagation::Proceed
        }
    }

    fn item_commands(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK) {
            return None;
        }
        if self.view.item_view_has_focus() && matches!(event.key, Key::Home | Key::End) {
            self.view.commit_selection();
        }
        if let Some(direction) = page_direction(event.key)
            && self.view.page_selection(direction)
        {
            return Some(Propagation::Stop);
        }
        match event.key {
            Key::y | Key::Y => {
                self.view.copy_path();
            }
            Key::p | Key::P => self.view.pin_focused(),
            Key::space => self.preview.toggle(preview_target(browser.focused_entry())),
            Key::BackSpace => self.view.navigate_up(),
            _ => return None,
        }
        Some(Propagation::Stop)
    }

    fn selection_navigation(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        if event.shift() {
            match event.key {
                Key::Up => browser.extend_selection(-1),
                Key::Down => browser.extend_selection(1),
                _ => return None,
            }
            return Some(Propagation::Stop);
        }
        if !event.alt()
            && matches!(event.key, Key::k | Key::Up)
            && self.view.focus_header_from_top_item()
        {
            return Some(Propagation::Stop);
        }
        None
    }

    fn directory_navigation(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::SUPER_MASK) {
            return Some(Propagation::Proceed);
        }
        match (event.key, event.alt()) {
            (Key::Left, true) => browser.back(),
            (Key::Right, true) => browser.forward(),
            (Key::Up, true) => browser.parent(),
            (Key::Home, true) => browser.navigate(Location::local(home_directory())),
            (Key::j | Key::Down, false) => browser.move_selection(1),
            (Key::k | Key::Up, false) => browser.move_selection(-1),
            (Key::h | Key::Left, false) => self.navigate_left(event),
            (Key::Right, false) if self.view.view_mode() == BrowserMode::Columns => {
                browser.enter_focused_directory();
            }
            (Key::l | Key::Return | Key::KP_Enter, false) => self.view.activate_focused(),
            (Key::Escape, false) => browser.escape(),
            _ => return None,
        }
        Some(Propagation::Stop)
    }

    fn navigate_left(&self, event: &KeyEvent) {
        if self.view.first_column_has_focus() && self.top_bar.sidebar_toggle().is_active() {
            self.sidebar.enter(&event.focused);
        } else {
            self.view.navigate_left();
        }
    }
}

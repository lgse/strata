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
        if popover.has_css_class("folder-context-popover") {
            return self.context_menu_popover_navigation(event, &popover);
        }
        Some(Propagation::Proceed)
    }

    fn context_menu_popover_navigation(
        &self,
        event: &KeyEvent,
        popover: &gtk::Popover,
    ) -> KeyResult {
        match event.key {
            Key::Up => {
                navigate_menu_items(popover, gtk::DirectionType::Up);
                Some(Propagation::Stop)
            }
            Key::Down => {
                navigate_menu_items(popover, gtk::DirectionType::Down);
                Some(Propagation::Stop)
            }
            Key::Home => {
                focus_first_or_last_menu_item(popover, true);
                Some(Propagation::Stop)
            }
            Key::End => {
                focus_first_or_last_menu_item(popover, false);
                Some(Propagation::Stop)
            }
            _ => None,
        }
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

fn navigate_menu_items(popover: &gtk::Popover, direction: gtk::DirectionType) {
    if popover.child_focus(direction) {
        return;
    }
    // `child_focus` returning false leaves focus on the current (edge) widget;
    // wrap explicitly to the opposite end instead of nudging from there.
    focus_first_or_last_menu_item(popover, direction == gtk::DirectionType::Down);
}

/// Real menu content nests action buttons inside sub-containers (an item
/// menu's `single`/`multiple` groups), so this walks the tree depth-first
/// rather than only the popover content's direct children.
pub fn focus_first_or_last_menu_item(popover: &gtk::Popover, first: bool) {
    let Some(scrolled) = popover.child().and_downcast::<gtk::ScrolledWindow>() else {
        return;
    };
    let Some(content) = scrolled.child() else {
        return;
    };
    focus_first_focusable_descendant(&content, first);
}

fn focus_first_focusable_descendant(container: &gtk::Widget, first: bool) -> bool {
    let mut current = if first {
        container.first_child()
    } else {
        container.last_child()
    };
    while let Some(widget) = current {
        if try_focus_menu_item(&widget, first) {
            return true;
        }
        current = if first {
            widget.next_sibling()
        } else {
            widget.prev_sibling()
        };
    }
    false
}

fn try_focus_menu_item(widget: &gtk::Widget, first: bool) -> bool {
    if !widget.is_visible() {
        return false;
    }
    // A candidate that looks focusable can still fail to take focus; keep
    // scanning rather than stopping on a silent no-op.
    if is_focusable_menu_item(widget) && widget.grab_focus() {
        return true;
    }
    focus_first_focusable_descendant(widget, first)
}

fn is_focusable_menu_item(widget: &gtk::Widget) -> bool {
    widget.type_() != gtk::Separator::static_type() && widget.is_sensitive() && widget.is_visible()
}

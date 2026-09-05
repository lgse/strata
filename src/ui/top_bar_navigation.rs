// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};

#[derive(Clone)]
pub(super) struct TopBarNavigation {
    header: gtk::Box,
    sidebar: gtk::Widget,
    toggle: gtk::ToggleButton,
    sidebar_entry: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>>,
}

impl TopBarNavigation {
    pub fn new(header: &gtk::Box, sidebar: &gtk::Widget, toggle: &gtk::ToggleButton) -> Self {
        Self {
            header: header.clone(),
            sidebar: sidebar.clone(),
            toggle: toggle.clone(),
            sidebar_entry: Rc::new(RefCell::new(None)),
        }
    }

    pub fn sidebar_toggle(&self) -> &gtk::ToggleButton {
        &self.toggle
    }

    pub fn has_focus(&self) -> bool {
        self.header
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| {
                focus == *self.header.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&self.header)
            })
    }

    pub fn move_up_from_sidebar(&self) -> bool {
        let Some(focused) = self.sidebar.root().and_then(|root| root.focus()) else {
            return false;
        };
        if focused != self.sidebar && !focused.is_ancestor(&self.sidebar) {
            return false;
        }
        if self.sidebar.child_focus(gtk::DirectionType::Up) {
            return true;
        }
        if self.toggle.grab_focus() {
            self.sidebar_entry.replace(Some(focused.downgrade()));
            return true;
        }
        false
    }

    pub fn move_focus(&self, direction: gtk::DirectionType) -> bool {
        let direction = match direction {
            gtk::DirectionType::Left => gtk::DirectionType::TabBackward,
            gtk::DirectionType::Right => gtk::DirectionType::TabForward,
            _ => return false,
        };
        self.header.child_focus(direction)
    }

    pub fn return_to_sidebar(&self) -> bool {
        if !self.sidebar.is_mapped() {
            return false;
        }
        if self
            .sidebar_entry
            .borrow_mut()
            .take()
            .and_then(|entry| entry.upgrade())
            .is_some_and(|entry| {
                entry.is_mapped() && entry.is_ancestor(&self.sidebar) && entry.grab_focus()
            })
        {
            return true;
        }
        self.sidebar.child_focus(gtk::DirectionType::TabForward)
    }
}

#[cfg(test)]
mod tests;

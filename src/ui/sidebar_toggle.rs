// SPDX-License-Identifier: MIT

//! The header's sidebar button. A click steps the sidebar through its modes
//! instead of flipping the active state, so one control reaches the full
//! sidebar, the icon rail, and the hidden sidebar.

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*, subclass::prelude::*};

use crate::{
    assets::{self, icons},
    ui::keys::{Keys, bindings::SIDEBAR},
};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SidebarToggle {
        pub(super) step: RefCell<Option<Rc<dyn Fn()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SidebarToggle {
        const NAME: &'static str = "StrataSidebarToggle";
        type Type = super::SidebarToggle;
        type ParentType = gtk::ToggleButton;
    }

    impl ObjectImpl for SidebarToggle {}
    impl WidgetImpl for SidebarToggle {}

    impl ButtonImpl for SidebarToggle {
        fn clicked(&self) {
            let step = self.step.borrow().clone();
            match step {
                Some(step) => step(),
                None => self.parent_clicked(),
            }
        }
    }

    impl ToggleButtonImpl for SidebarToggle {}
}

glib::wrapper! {
    pub struct SidebarToggle(ObjectSubclass<imp::SidebarToggle>)
        @extends gtk::ToggleButton, gtk::Button, gtk::Widget,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

/// What the sidebar shows. The toggle's `active` state is its visibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SidebarMode {
    Expanded,
    Compact,
    Hidden,
}

impl SidebarMode {
    pub(crate) fn of(visible: bool, rail: bool) -> Self {
        match (visible, rail) {
            (false, _) => Self::Hidden,
            (true, true) => Self::Compact,
            (true, false) => Self::Expanded,
        }
    }

    /// The step a click takes: full, then the rail, then hidden, then full.
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Expanded => Self::Compact,
            Self::Compact => Self::Hidden,
            Self::Hidden => Self::Expanded,
        }
    }

    fn step_message(self) -> &'static str {
        match self {
            Self::Expanded => "Show sidebar (%{keys})",
            Self::Compact => "Compact sidebar (%{keys})",
            Self::Hidden => "Hide sidebar (%{keys})",
        }
    }
}

impl SidebarToggle {
    pub(crate) fn new() -> Self {
        let toggle: Self = glib::Object::new();
        toggle.set_active(true);
        toggle.set_child(Some(&assets::primary_icon(icons::PANEL_LEFT, 17)));
        toggle.add_css_class("sidebar-toggle");
        toggle.set_cursor_from_name(Some("pointer"));
        toggle
    }

    /// Runs on click or keyboard activation instead of flipping `active`.
    pub(crate) fn set_step(&self, step: impl Fn() + 'static) {
        self.imp().step.replace(Some(Rc::new(step)));
    }

    /// The tooltip names the step the next click takes from `mode`; the
    /// accessible name stays the control's own so it can be found in any mode.
    pub(crate) fn describe(&self, mode: SidebarMode) {
        let keys = Keys::One(SIDEBAR.shown());
        self.set_tooltip_text(Some(&super::shortcut_reference::with_keys(
            mode.next().step_message(),
            keys,
        )));
        super::accessibility::set_label(
            self,
            &super::shortcut_reference::with_keys("Toggle sidebar (%{keys})", keys),
        );
    }
}

#[cfg(test)]
mod tests;

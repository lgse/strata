// SPDX-License-Identifier: MIT

//! Default key map Tab stops: each listing is one stop, and Tab into it lands on the
//! keyboard cursor. GTK's `ListTabBehavior::Item` makes List and Icons leave in one
//! press; the Columns strip spans several lists, so its Tab moves are routed here.

use std::rc::Rc;

use gtk::{glib, prelude::*};

use super::{BrowserView, ViewState, focus_header_action};
use crate::ui::browser_modes::BrowserMode;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TabCrossing {
    #[default]
    None,
    /// A Tab press from outside the item views is moving focus.
    Pending,
    /// That focus reached the listing; the cursor takes it once GTK finishes.
    Landing,
}

impl BrowserView {
    /// Returns true when it moved focus itself, false to let GTK move it.
    pub fn default_tab(&self, direction: gtk::DirectionType) -> bool {
        if self.view_mode() == BrowserMode::Columns && self.state.columns_tab(direction) {
            return true;
        }
        if !self.item_view_has_focus() {
            self.state.note_tab_crossing();
        }
        false
    }
}

impl ViewState {
    fn note_tab_crossing(self: &Rc<Self>) {
        if self.tab_crossing.replace(TabCrossing::Pending) != TabCrossing::None {
            return;
        }
        // GTK moves focus within the same key event; anything later is not a Tab crossing.
        let state = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(state) = state.upgrade() {
                state.tab_crossing.set(TabCrossing::None);
            }
        });
    }

    /// Called when focus enters an item view or a column. Returns whether a Tab press
    /// carried it there, in which case the keyboard cursor takes focus instead.
    pub(in crate::ui) fn land_tab_crossing(self: &Rc<Self>) -> bool {
        match self.tab_crossing.get() {
            TabCrossing::None => false,
            TabCrossing::Landing => true,
            TabCrossing::Pending => {
                self.tab_crossing.set(TabCrossing::Landing);
                let state = Rc::downgrade(self);
                glib::idle_add_local_once(move || {
                    if let Some(state) = state.upgrade()
                        && crate::ui::focus_navigation::contains_widget(
                            state.overlay.upcast_ref(),
                            state.overlay.root().and_then(|root| root.focus()).as_ref(),
                        )
                    {
                        state.browser.focus_active();
                    }
                });
                true
            }
        }
    }

    /// The Columns strip is one Tab stop. Tab leaves it for the next control; Shift+Tab
    /// goes to the focused column's open filter or header actions, then to the control
    /// before the strip.
    fn columns_tab(&self, direction: gtk::DirectionType) -> bool {
        let Some(focused) = self.overlay.root().and_then(|root| root.focus()) else {
            return false;
        };
        if !focused.is_ancestor(&self.scroller) {
            return false;
        }
        let Some(column) = self
            .columns
            .borrow()
            .iter()
            .find(|column| focused.is_ancestor(&column.shell))
            .cloned()
        else {
            return false;
        };
        let backward = direction == gtk::DirectionType::TabBackward;
        let strip = self.scroller.upcast_ref::<gtk::Widget>();
        let leave = || crate::ui::focus_navigation::focus_beyond(strip, direction);
        let surface = column.presentation.stack.upcast_ref::<gtk::Widget>();
        let moved = if focused.is_ancestor(&column.header_actions) {
            if !backward {
                // GTK continues into this column's list.
                return false;
            }
            column.header_actions.child_focus(direction) || leave()
        } else if focused == *surface || focused.is_ancestor(surface) {
            if backward {
                (column.presentation.retry_has_focus(&focused) && surface.grab_focus())
                    // An open filter sits between the header and the rows, as on Tab.
                    || (column.filter_entry.is_mapped() && column.filter_entry.grab_focus())
                    || (column.header_actions_stack.visible_child_name().as_deref()
                        == Some("actions")
                        && column.header_actions.is_mapped()
                        && focus_header_action(&column.header_actions, gtk::DirectionType::Left))
                    || leave()
            } else {
                (focused == *surface && column.presentation.focus_retry()) || leave()
            }
        } else {
            return false;
        };
        if moved && let Some(window) = self.overlay.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        true
    }
}

// SPDX-License-Identifier: MIT

//! Which surface owns a key before the dispatcher reads it as a command.

use gtk::{glib::Propagation, prelude::*};

use crate::{
    app::Browser,
    ui::{
        keys::{KeyPress, bindings::*},
        modal::is_closing_layer,
        window::{topmost_modal_layer, visible_modal_layer},
    },
};

use super::{Dispatcher, KeyResult};

impl Dispatcher {
    pub(super) fn input_owner(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        if let Some(layer) = topmost_modal_layer(&self.window, &is_closing_layer) {
            let focus_is_inside = gtk::prelude::RootExt::focus(&self.window)
                .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
            self.shortcuts.cancel_chord();
            if !focus_is_inside {
                layer.grab_focus();
                return Some(Propagation::Stop);
            }
            return Some(Propagation::Proceed);
        }
        if visible_modal_layer(&self.window).is_some() {
            // A layer animating out has dropped focus; only window actions, whose
            // guard skips closing layers, may act until focus is restored.
            self.shortcuts.cancel_chord();
            return Some(Propagation::Proceed);
        }
        if self.native_menu_owns_input() {
            return Some(Propagation::Proceed);
        }
        if press.is(ESCAPE_ANY) && self.view.cancel_column_drag() {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        let mut focused = gtk::prelude::RootExt::focus(&self.window);
        while let Some(widget) = focused {
            if widget.has_css_class("file-operation-card") {
                self.shortcuts.cancel_chord();
                return Some(Propagation::Proceed);
            }
            focused = widget.parent();
        }
        if self.shortcuts.prompt_has_focus() {
            if let Some(result) = self.footer_key(press) {
                return Some(result);
            }
            return Some(self.prompt_key(browser, press));
        }
        if self
            .preview
            .handle_find_key(press, self.type_to_search.preferences.tenxer_mode())
        {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        if let Some(result) = self.tenxer_preview_text(browser, press) {
            return Some(result);
        }
        if !self.inline_editing_active()
            && let Some(result) = self.footer_key(press)
        {
            return Some(result);
        }
        if press.is(ESCAPE_ANY) && crate::ui::scrolling::stop_autoscroll() {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        None
    }

    /// Shortcut-reference keys run before the chord consumer. A visible prompt
    /// keeps its armed chord; every other claimed footer key cancels first.
    pub(super) fn footer_key(&self, press: &KeyPress) -> KeyResult {
        let prompted = self.shortcuts.prompt_is_visible();
        let result = self.shortcuts.handle_key(press)?;
        if !prompted {
            self.shortcuts.cancel_chord();
        }
        Some(result)
    }

    pub(super) fn native_menu_owns_input(&self) -> bool {
        if gtk::prelude::RootExt::focus(&self.window).is_some_and(|focused| {
            focused.is::<gtk::PopoverMenu>()
                || focused.ancestor(gtk::PopoverMenu::static_type()).is_some()
                || focused
                    .ancestor(gtk::Popover::static_type())
                    .is_some_and(|popover| popover.has_css_class("folder-context-popover"))
        }) {
            return true;
        }
        visible_popover_menu(self.window.upcast_ref())
    }
}

pub(super) fn visible_popover_menu(widget: &gtk::Widget) -> bool {
    if widget.is_visible() && widget.is::<gtk::PopoverMenu>() {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if visible_popover_menu(&widget) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

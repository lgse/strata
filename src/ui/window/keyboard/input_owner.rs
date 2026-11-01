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
        self.modal_owner()
            .or_else(|| {
                self.native_menu_owns_input()
                    .then_some(Propagation::Proceed)
            })
            .or_else(|| self.chrome_owner(press))
            .or_else(|| self.prompt_owner(browser, press))
            .or_else(|| self.preview_and_footer_owner(browser, press))
    }

    fn modal_owner(&self) -> KeyResult {
        if let Some(layer) = topmost_modal_layer(&self.window, &is_closing_layer) {
            return Some(self.keep_focus_in_layer(&layer));
        }
        visible_modal_layer(&self.window)?;
        // A layer animating out has dropped focus; only window actions, whose
        // guard skips closing layers, may act until focus is restored.
        self.shortcuts.cancel_chord();
        Some(Propagation::Proceed)
    }

    fn keep_focus_in_layer(&self, layer: &gtk::Widget) -> Propagation {
        let focus_is_inside = gtk::prelude::RootExt::focus(&self.window)
            .is_some_and(|focus| focus == *layer || focus.is_ancestor(layer));
        self.shortcuts.cancel_chord();
        if focus_is_inside {
            return Propagation::Proceed;
        }
        layer.grab_focus();
        Propagation::Stop
    }

    fn chrome_owner(&self, press: &KeyPress) -> KeyResult {
        if press.is(ESCAPE_ANY) && self.view.cancel_column_drag() {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        if self.operation_card_focused() {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Proceed);
        }
        None
    }

    fn operation_card_focused(&self) -> bool {
        std::iter::successors(
            gtk::prelude::RootExt::focus(&self.window),
            gtk::Widget::parent,
        )
        .any(|widget| widget.has_css_class("file-operation-card"))
    }

    fn prompt_owner(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        if !self.shortcuts.prompt_has_focus() {
            return None;
        }
        Some(
            self.footer_key(press)
                .unwrap_or_else(|| self.prompt_key(browser, press)),
        )
    }

    fn preview_and_footer_owner(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
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

// SPDX-License-Identifier: MIT

//! The 10xer key map stages.

use std::rc::Rc;

use gtk::gdk::ModifierType as Modifiers;
use gtk::{glib::Propagation, prelude::*};

use crate::{
    app::Browser,
    ui::{
        keys::{KeyPress, Map, bindings::*},
        window::visible_modal_layer,
    },
};

use super::{
    Dispatcher, KeyResult, sidebar,
    sidebar::{SidebarChord, activate_sidebar_focus, move_sidebar_focus, sidebar_chord},
};

impl Dispatcher {
    pub(super) fn tenxer_keys(&self, browser: &Rc<Browser>, press: &KeyPress) -> KeyResult {
        if visible_modal_layer(&self.window).is_some() {
            return None;
        }
        if self.toggles_tenxer_mode(press) {
            return Some(Propagation::Stop);
        }
        if !self.tenxer_reads_keys() {
            return None;
        }
        if let Some(result) = self.tenxer_window_keys(browser, press) {
            return Some(result);
        }
        let focus = gtk::prelude::RootExt::focus(&self.window);
        if let Some(result) = self.tenxer_chrome_keys(browser, press, &focus) {
            return Some(result);
        }
        if press.is(ESCAPE_ANY_SHIFT) && !self.inline_editing_active() {
            return self.tenxer_escape(browser);
        }
        self.route_typing_to_listing(browser, press, &focus);
        self.tenxer_listing_keys(browser, press)
    }

    fn toggles_tenxer_mode(&self, press: &KeyPress) -> bool {
        if !press.is(TENXER_MODE) {
            return false;
        }
        let preferences = &self.type_to_search.preferences;
        preferences.set_tenxer_mode(!preferences.tenxer_mode());
        true
    }

    fn tenxer_reads_keys(&self) -> bool {
        if !self.type_to_search.preferences.tenxer_mode() {
            return false;
        }
        if self.focus_keeps_typing() {
            self.shortcuts.cancel_chord();
            return false;
        }
        true
    }

    /// Text fields keep their typing, except a focused preview document, and so
    /// do popovers.
    fn focus_keeps_typing(&self) -> bool {
        let typing = self.text_focused() && !self.preview_document_focused();
        typing || self.focus_in_popover()
    }

    fn tenxer_window_keys(&self, browser: &Rc<Browser>, press: &KeyPress) -> KeyResult {
        if let Some(result) = self
            .tenxer_chord(browser, press)
            .or_else(|| self.chooser_refusal(press))
            .or_else(|| self.chooser_save(press))
        {
            return Some(result);
        }
        if press.is(CLOSE_WINDOW) {
            self.window.close();
            return Some(Propagation::Stop);
        }
        self.tenxer_preview(browser, press)
    }

    fn tenxer_chrome_keys(
        &self,
        browser: &Rc<Browser>,
        press: &KeyPress,
        focus: &Option<gtk::Widget>,
    ) -> KeyResult {
        if self.sidebar.contains(focus)
            && let Some(result) = self.tenxer_sidebar(browser, press)
        {
            return Some(result);
        }
        if self.tenxer_header_focused(focus) {
            return self.tenxer_header(browser, press);
        }
        None
    }

    /// Unclaimed typing belongs to the listing in 10xer mode, even when
    /// compositor focus restoration selected a header or footer control.
    /// Text, menus, sidebar/header actions, and preview ownership run first.
    fn route_typing_to_listing(
        &self,
        browser: &Rc<Browser>,
        press: &KeyPress,
        focus: &Option<gtk::Widget>,
    ) {
        if crate::ui::window::type_to_search_query(press).is_none()
            || self.view.item_view_has_focus()
        {
            return;
        }
        if !self.preview.owns_focus(focus.as_ref()) {
            browser.focus_active();
        }
    }

    fn tenxer_listing_keys(&self, browser: &Rc<Browser>, press: &KeyPress) -> KeyResult {
        if let Some(result) = self
            .tenxer_prompt_keys(press)
            .or_else(|| self.tenxer_file_keys(press))
        {
            return Some(result);
        }
        let icons = self.view.view_mode().is_grid();
        let claimed = if icons {
            self.tenxer_icons(browser, press)
        } else {
            self.tenxer_listing(browser, press)
        };
        (claimed || self.swallows_unclaimed(press)).then_some(Propagation::Stop)
    }

    fn swallows_unclaimed(&self, press: &KeyPress) -> bool {
        if claims_unbound_command(press) {
            return true;
        }
        if !self.view.item_view_has_focus() {
            return false;
        }
        claims_file_list_typing(press) && !self.runs_directly(press)
    }

    pub(super) fn text_focused(&self) -> bool {
        gtk::prelude::RootExt::focus(&self.window).is_some_and(|focused| {
            focused.is::<gtk::Text>() || focused.is::<gtk::TextView>() || focused.is::<gtk::Entry>()
        })
    }

    pub(super) fn focus_in_popover(&self) -> bool {
        gtk::prelude::RootExt::focus(&self.window).is_some_and(|focused| {
            focused.is::<gtk::Popover>() || focused.ancestor(gtk::Popover::static_type()).is_some()
        })
    }

    pub(super) fn tenxer_sidebar(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        if press.is(CONTEXT_MENU) && sidebar::open_sidebar_context_menu(&self.sidebar.widget) {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        let chord = sidebar_chord(press)?;
        match chord {
            SidebarChord::Move(delta) => {
                move_sidebar_focus(&self.sidebar.widget, delta);
            }
            SidebarChord::Activate => self.activate_sidebar(browser),
            SidebarChord::Leave => self.sidebar.restore(browser, true),
            SidebarChord::Swallow => {}
        }
        Some(Propagation::Stop)
    }

    pub(super) fn activate_sidebar(&self, browser: &Browser) {
        let before = browser.active_location();
        if !activate_sidebar_focus(&self.sidebar.widget) {
            return;
        }
        if browser.active_location() != before {
            self.sidebar.previous.replace(None);
            if !self.view.item_view_has_focus() {
                browser.focus_active();
            }
        }
    }

    pub(super) fn tenxer_header_focused(&self, focus: &Option<gtk::Widget>) -> bool {
        if self.top_bar.has_focus() || self.view.header_actions_have_focus() {
            return true;
        }
        let panes = self.view.widget();
        crate::ui::focus_navigation::contains_widget(&panes, focus.as_ref())
            && !self.view.item_view_has_focus()
    }

    pub(super) fn tenxer_header(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        let held = press.modifiers();
        if held.intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
            return None;
        }
        // The held Shift, including one that typed a symbol, swallows the key.
        if held.contains(Modifiers::SHIFT_MASK) && !press.is(FOCUS_PREVIOUS) {
            return Some(Propagation::Stop);
        }
        if press.is_any(&[LEFT.alternates(), DOWN.alternates()]) {
            self.return_from_header(browser);
        } else if press.is(ENTER) || press.is(SPACE) {
            crate::ui::focus_navigation::activate(self.window.upcast_ref());
        } else if !press.is(MOVE_TO_TRASH.in_map(Map::Default)) {
            return None;
        }
        Some(Propagation::Stop)
    }

    pub(super) fn return_from_header(&self, browser: &Browser) {
        if self.view.header_actions_have_focus() && self.view.focus_items_from_header() {
            return;
        }
        browser.focus_active();
    }
}

/// Default-map keys of window commands that 10xer mode leaves unbound or binds
/// to other keys, so the file list swallows them, and Ctrl+T.
pub(super) fn claims_unbound_command(press: &KeyPress) -> bool {
    [
        DUPLICATE,
        FILTER,
        SIDEBAR,
        RENAME,
        ARROW_SCOPE,
        JUMP_TO_FOLDER,
    ]
    .into_iter()
    .any(|command| press.is(command.in_map(Map::Default)) && !press.is(command.in_map(Map::Tenxer)))
        || press.is(TAB_NEW)
}

pub(super) fn claims_file_list_typing(press: &KeyPress) -> bool {
    press.is(SPACE.ignoring(Modifiers::SHIFT_MASK))
        || crate::ui::window::type_to_search_query(press).is_some()
}

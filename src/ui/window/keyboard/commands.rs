// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::{self, Propagation},
    prelude::*,
};

use super::{Dispatcher, KeyEvent, KeyResult};
use crate::{
    app::Browser,
    ui::{
        browser::BrowserView,
        window::{
            apply_browser_mode, browser_mode_for_digit, is_browser_navigation_key,
            is_open_terminal_shortcut, is_refresh_shortcut, is_rename_shortcut,
            is_sidebar_focus_shortcut, is_toggle_hidden_shortcut, is_undo_shortcut,
            type_to_search_query,
        },
    },
};

impl Dispatcher {
    pub(super) fn window_commands(&self, event: &KeyEvent) -> KeyResult {
        if event.control()
            && event.without(Modifiers::SHIFT_MASK | Modifiers::ALT_MASK)
            && let Some(mode) = browser_mode_for_digit(event.key)
        {
            apply_browser_mode(&self.view, &crate::ui::theme::ThemeManager::shared(), mode);
            return Some(Propagation::Stop);
        }
        if event.control() && matches!(event.key, Key::k | Key::K) {
            if let Err(error) =
                gtk::prelude::WidgetExt::activate_action(&self.window, "win.search", None)
            {
                tracing::warn!(%error, "unable to activate global search shortcut");
            }
            return Some(Propagation::Stop);
        }
        None
    }

    pub(super) fn inline_editing(&self, event: &KeyEvent) -> KeyResult {
        if is_rename_shortcut(event.key, event.modifiers)
            && !event
                .focused
                .as_ref()
                .is_some_and(crate::ui::focus_navigation::editable)
            && self.view.begin_rename()
        {
            return Some(Propagation::Stop);
        }
        if event.key == Key::Escape && (self.view.cancel_new_entry() || self.view.cancel_rename()) {
            return Some(Propagation::Stop);
        }
        self.inline_editing_active().then_some(Propagation::Proceed)
    }

    pub(super) fn filter_and_location_commands(&self, event: &KeyEvent) -> KeyResult {
        if event.key == Key::space
            && event.without(
                Modifiers::CONTROL_MASK
                    | Modifiers::ALT_MASK
                    | Modifiers::SUPER_MASK
                    | Modifiers::SHIFT_MASK,
            )
            && let Some(entry) = self.view.selected_search_result()
        {
            self.preview
                .toggle(crate::ui::preview::preview_target(Some(entry)));
            return Some(Propagation::Stop);
        }
        if event.key == Key::Escape && self.view.dismiss_focused_filter() {
            return Some(Propagation::Stop);
        }
        if event.control()
            && event.without(Modifiers::SHIFT_MASK | Modifiers::ALT_MASK)
            && matches!(event.key, Key::f | Key::F)
            && self.view.show_filter()
        {
            return Some(Propagation::Stop);
        }
        if event.control() && event.key == Key::l {
            self.view.begin_location_edit();
            return Some(Propagation::Stop);
        }
        None
    }

    pub(super) fn video_controls(&self, event: &KeyEvent) -> KeyResult {
        if !matches!(
            event.key,
            Key::space | Key::Up | Key::Down | Key::Left | Key::Right | Key::m | Key::M
        ) {
            return None;
        }
        if self.preview.has_video()
            && !self.sidebar.contains(&event.focused)
            && !self.top_bar.has_focus()
            && !event.text_has_focus()
            && event.without(Modifiers::ALT_MASK | Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK)
        {
            self.preview.handle_video_key(event.key);
            return Some(Propagation::Stop);
        }
        None
    }

    pub(super) fn sidebar_commands(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        let toggle = self.top_bar.sidebar_toggle();
        if is_sidebar_focus_shortcut(event.key, event.modifiers) {
            self.view.keyboard_navigation();
            if self.sidebar.contains(&event.focused) {
                self.sidebar.restore(browser, false);
            } else {
                self.sidebar.previous.replace(event.focused.clone());
                if !toggle.is_active() {
                    toggle.set_active(true);
                }
                let sidebar = self.sidebar.state.clone();
                glib::idle_add_local_once(move || {
                    sidebar.focus_active_place();
                });
            }
            return Some(Propagation::Stop);
        }
        if event.control() && !event.shift() && matches!(event.key, Key::b | Key::B) {
            toggle.set_active(!toggle.is_active());
            return Some(Propagation::Stop);
        }
        None
    }

    pub(super) fn text_input(&self, event: &KeyEvent) -> KeyResult {
        if self.view.location_has_focus() {
            if event.key == Key::Escape {
                self.view.cancel_location_edit();
                return Some(Propagation::Stop);
            }
            return Some(Propagation::Proceed);
        }
        if !event.text_has_focus()
            && is_undo_shortcut(event.key, event.modifiers)
            && self.view.undo_last_operation()
        {
            return Some(Propagation::Stop);
        }
        if self.view.item_view_has_focus()
            && let Some(query) = type_to_search_query(event.key, event.modifiers)
            && self.type_to_search.show(query)
        {
            return Some(Propagation::Stop);
        }
        if !event.text_has_focus() && is_browser_navigation_key(event.key, event.modifiers) {
            self.view.keyboard_navigation();
        }
        None
    }

    pub(super) fn file_commands(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if event.alt()
            && event.without(Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK)
            && matches!(event.key, Key::Return | Key::KP_Enter)
            && self.view.show_focused_properties()
        {
            return Some(Propagation::Stop);
        }
        if event.control() && event.shift() && matches!(event.key, Key::n | Key::N) {
            self.view.create_new_folder();
            return Some(Propagation::Stop);
        }
        self.clipboard_command(event)
            .or_else(|| self.browser_commands(browser, event))
    }

    fn clipboard_command(&self, event: &KeyEvent) -> KeyResult {
        if !event.control() || event.shift() {
            return None;
        }
        let action: fn(&BrowserView) -> bool = match event.key {
            Key::v => |view| {
                view.paste();
                true
            },
            Key::c => BrowserView::copy_selection,
            Key::d | Key::D => BrowserView::duplicate_selection,
            Key::x => BrowserView::cut_selection,
            Key::a => |view| {
                view.select_all();
                true
            },
            _ => return None,
        };
        if self.view.filter_has_focus() || event.text_has_focus() {
            return Some(Propagation::Proceed);
        }
        action(&self.view).then_some(Propagation::Stop)
    }

    fn browser_commands(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if is_toggle_hidden_shortcut(event.key, event.modifiers) {
            browser.toggle_hidden();
        } else if is_open_terminal_shortcut(event.key, event.modifiers) {
            self.view.open_terminal();
        } else if is_refresh_shortcut(event.key) {
            self.view.refresh();
        } else {
            return None;
        }
        Some(Propagation::Stop)
    }
}

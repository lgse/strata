// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{
    glib::{self, Propagation},
    prelude::*,
};

use super::{Dispatcher, KeyEvent, KeyResult};
use crate::{
    app::Browser,
    ui::{
        browser::BrowserView,
        keys::{Map, SHORTCUT_MODIFIERS, bindings::*},
        window::{
            apply_browser_mode, browser_mode_for_digit, is_browser_navigation_key,
            type_to_search_query,
        },
    },
};

fn entry_for(widget: &gtk::Widget) -> Option<gtk::Entry> {
    widget.clone().downcast().ok().or_else(|| {
        widget
            .ancestor(gtk::Entry::static_type())
            .and_then(|entry| entry.downcast().ok())
    })
}

impl Dispatcher {
    pub(super) fn window_commands(&self, event: &KeyEvent) -> KeyResult {
        if (!event.text_has_focus() || self.view.filter_has_focus())
            && let Some(mode) = browser_mode_for_digit(&event.press)
        {
            apply_browser_mode(
                &self.view,
                &crate::ui::preferences::PreferenceManager::shared(),
                mode,
            );
            return Some(Propagation::Stop);
        }
        if event.text_has_focus() {
            return None;
        }
        let (action, label) = if event.press.is(SEARCH) {
            ("win.search", "global search")
        } else if event.press.is(JUMP_TO_FOLDER.in_map(Map::Default)) {
            ("win.jump-folder", "folder jump")
        } else {
            return None;
        };
        if let Err(error) = gtk::prelude::WidgetExt::activate_action(&self.window, action, None) {
            tracing::warn!(%error, "unable to activate {label} shortcut");
        }
        Some(Propagation::Stop)
    }

    pub(super) fn inline_editing(&self, event: &KeyEvent) -> KeyResult {
        if event.press.is(RENAME.in_map(Map::Default))
            && (self.view.filter_has_focus()
                || !event
                    .focused
                    .as_ref()
                    .is_some_and(crate::ui::focus_navigation::editable))
            && self.view.begin_rename()
        {
            return Some(Propagation::Stop);
        }
        if event.press.is(ESCAPE_ANY) && (self.view.cancel_new_entry() || self.view.cancel_rename())
        {
            return Some(Propagation::Stop);
        }
        if !self.inline_editing_active() {
            return None;
        }
        // Stop Ctrl+A before the collection view also applies its select-all binding.
        if event.press.is(SELECT_ALL)
            && let Some(field) = self.view.active_rename_field()
        {
            field.select_region(0, -1);
            return Some(Propagation::Stop);
        }
        Some(Propagation::Proceed)
    }

    pub(super) fn filter_and_location_commands(&self, event: &KeyEvent) -> KeyResult {
        if event.press.is(PREVIEW.in_map(Map::Default))
            && let Some(entry) = self.view.search_result_for_space()
        {
            if self.view.activate_directory_on_space() {
                return Some(Propagation::Stop);
            }
            self.preview.toggle(
                crate::ui::preview::preview_target(Some(entry)),
                self.view.browser().active_depth(),
            );
            return Some(Propagation::Stop);
        }
        if event.press.is(ESCAPE_ANY)
            && (self.view.dismiss_focused_filter() || self.dismiss_direct_search(event))
        {
            return Some(Propagation::Stop);
        }
        if event.press.is(FILTER.in_map(Map::Default)) && self.view.show_filter() {
            return Some(Propagation::Stop);
        }
        if event.press.is(LOCATION) {
            self.view.begin_location_edit();
            return Some(Propagation::Stop);
        }
        None
    }

    pub(super) fn video_controls(&self, event: &KeyEvent) -> KeyResult {
        if !self.sidebar.contains(&event.focused)
            && !self.top_bar.has_focus()
            && !event.text_has_focus()
            && self.preview.handle_video_key(&event.press)
        {
            return Some(Propagation::Stop);
        }
        None
    }

    pub(super) fn sidebar_commands(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        let toggle = self.top_bar.sidebar_toggle();
        let tenxer = self.type_to_search.preferences.tenxer_mode();
        if event.press.is(SIDEBAR_FOCUS) {
            if tenxer {
                if !toggle.is_active() {
                    return Some(Propagation::Stop);
                }
                if self.sidebar.contains(&event.focused) {
                    self.sidebar.restore(browser, true);
                } else {
                    self.sidebar.previous.replace(event.focused.clone());
                    self.sidebar.state.focus_active_place();
                }
                return Some(Propagation::Stop);
            }
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
        if event.press.is(SIDEBAR) {
            toggle.emit_clicked();
            return Some(Propagation::Stop);
        }
        None
    }

    pub(super) fn text_input(&self, event: &KeyEvent) -> KeyResult {
        if self.view.location_has_focus() {
            if event.press.is(ESCAPE_ANY) {
                self.view.cancel_location_edit();
                return Some(Propagation::Stop);
            }
            if event.press.is(SELECT_ALL)
                && let Some(entry) = event.focused.as_ref().and_then(entry_for)
            {
                entry.select_region(0, -1);
                return Some(Propagation::Stop);
            }
            return Some(Propagation::Proceed);
        }
        if !event.text_has_focus() && event.press.is(UNDO) && self.view.undo_last_operation() {
            return Some(Propagation::Stop);
        }
        if !event.text_has_focus() && event.press.is(REDO) && self.view.redo_last_operation() {
            return Some(Propagation::Stop);
        }
        if self.view.item_view_has_focus()
            && let Some(query) = type_to_search_query(&event.press)
            && self.type_to_search.show(query)
        {
            return Some(Propagation::Stop);
        }
        if !event.text_has_focus() && is_browser_navigation_key(&event.press) {
            self.view.keyboard_navigation();
        }
        None
    }

    pub(super) fn properties_command(&self, event: &KeyEvent) -> KeyResult {
        if !event.text_has_focus()
            && event
                .press
                .is(PROPERTIES.ignoring(gtk::gdk::ModifierType::SUPER_MASK))
            && self.view.show_focused_properties()
        {
            return Some(Propagation::Stop);
        }
        None
    }

    pub(super) fn file_commands(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if event.press.is(NEW_FOLDER) {
            self.view.create_new_folder();
            return Some(Propagation::Stop);
        }
        if event.press.is(NEW_FOLDER_WITH_SELECTION) {
            if !self.view.create_new_folder_for_selection() {
                self.view.create_new_folder();
            }
            return Some(Propagation::Stop);
        }
        self.clipboard_command(event)
            .or_else(|| self.browser_commands(browser, event))
    }

    pub(super) fn clipboard_command(&self, event: &KeyEvent) -> KeyResult {
        let press = &event.press;
        let action: fn(&BrowserView) -> bool = if press.is(PASTE) {
            |view| {
                view.paste();
                true
            }
        } else if press.is(COPY) {
            BrowserView::copy_selection
        } else if press.is(DUPLICATE.in_map(Map::Default)) {
            BrowserView::duplicate_selection
        } else if press.is(CUT) {
            BrowserView::cut_selection
        } else if press.is(SELECT_ALL) {
            |view| {
                view.select_all();
                true
            }
        } else {
            return None;
        };
        if self.view.filter_has_focus()
            || event.text_has_focus()
            || (self.preview_has_focus(event) && press.is_any(&[SELECT_ALL, COPY, PASTE, CUT]))
        {
            return Some(Propagation::Proceed);
        }
        action(&self.view).then_some(Propagation::Stop)
    }

    fn preview_has_focus(&self, event: &KeyEvent) -> bool {
        let preview = self.preview.widget();
        event
            .focused
            .as_ref()
            .is_some_and(|focused| focused == &preview || focused.is_ancestor(&preview))
    }

    pub(super) fn context_menu_command(&self, event: &KeyEvent) -> KeyResult {
        if event.press.is(CONTEXT_MENU)
            && !event.text_has_focus()
            && (super::sidebar::open_sidebar_context_menu(&self.sidebar.widget)
                || self.view.open_focused_context_menu())
        {
            return Some(Propagation::Stop);
        }
        None
    }

    fn browser_commands(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if event.press.is(HIDDEN_FILES.in_map(Map::Default)) {
            browser.toggle_hidden();
        } else if event.press.is(TERMINAL) {
            self.view.open_terminal();
        } else if event.press.is(REFRESH.ignoring(SHORTCUT_MODIFIERS)) {
            self.view.refresh();
        } else {
            return None;
        }
        Some(Propagation::Stop)
    }
}

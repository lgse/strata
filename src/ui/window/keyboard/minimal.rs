// SPDX-License-Identifier: MIT

//! Minimal browsing map used when `PreferenceManager::minimal_mode` is on.

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::{self, Propagation},
    prelude::*,
};

use super::{Dispatcher, KeyEvent, KeyResult};
use crate::{
    app::Browser,
    model::{FileEntry, Location, SortDirection, SortKey},
    services::{ActionHandle, InvocationSource},
    ui::{
        browser_modes::BrowserMode,
        minimal_mode::{
            MinimalChord, MinimalPrompt, MinimalState, MinimalVisual, cycle_goto_path,
            resolve_goto_input,
        },
        preview::preview_target,
        window::{
            apply_browser_mode, browser_mode_for_digit, home_directory, is_context_menu_shortcut,
            is_sidebar_focus_shortcut, is_undo_shortcut, vim_focus_direction,
        },
    },
};

impl Dispatcher<'_> {
    pub(super) fn minimal_commands(
        &self,
        browser: &Rc<Browser>,
        event: &mut KeyEvent,
    ) -> KeyResult {
        // An already-started row editor keeps native behavior; F2 / Ctrl+R
        // must not start one from here.
        if self.inline_editing_active() {
            if event.key == Key::Escape
                && (self.view.cancel_new_entry() || self.view.cancel_rename())
            {
                return Some(Propagation::Stop);
            }
            if event.control()
                && event.without(Modifiers::SHIFT_MASK | Modifiers::ALT_MASK)
                && event.key == Key::a
                && let Some(field) = self.view.active_rename_field()
            {
                field.select_region(0, -1);
                return Some(Propagation::Stop);
            }
            return Some(Propagation::Proceed);
        }
        // The footer prompt owns Enter/Escape/Up/Down/Tab; anything else edits
        // natively. This must precede the generic text-focus early-out.
        if self.minimal.borrow().prompt().is_some() {
            return Some(self.minimal_prompt_key(browser, event));
        }
        // Preview ownership is a dispatcher flag, not GTK text focus. Skip the
        // native-edit early-out so a source view cannot swallow `j`/`k`.
        if event.text_has_focus()
            && !self.shortcuts.is_prompt_entry(&event.focused)
            && !self.preview_owns_keys()
        {
            if browser.is_chooser_mode()
                && event.key == Key::Escape
                && event
                    .without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            {
                if self.view.location_has_focus() {
                    self.view.cancel_location_edit();
                    return Some(Propagation::Stop);
                }
                // The filename field is a gtk::Entry. Chooser Esc still dismisses
                // one browsing step before cancelling. Inline editors return above.
                return Some(self.minimal_escape(browser));
            }
            return Some(Propagation::Proceed);
        }
        // Preference teardown can clear prompt state before GTK moves entry focus.
        if self.shortcuts.is_prompt_entry(&event.focused)
            && self.minimal.borrow().prompt().is_none()
        {
            self.shortcuts.hide_prompt();
            self.view.set_find_prompt_active(false);
            self.view.restore_file_view_focus();
        }
        // Overlay rows (`f` matches or recursive `s` hits) own selection.
        // `selected_search_results().is_some()` is true for both; only
        // `force_recursive_search` marks recursive `s` (h dismiss, no invert).
        let overlay = self.view.selected_search_results().is_some();
        if event.key == Key::comma
            && event.control()
            && event.without(Modifiers::SHIFT_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            self.cancel_chord_ui();
            (self.open_settings)();
            return Some(Propagation::Stop);
        }
        // An armed chord owns the next key: search, sidebar, chrome, and
        // video must not steal a sort/go/copy/action completion.
        let chord = self.minimal.borrow().chord();
        if let Some(kind) = chord {
            return Some(self.finish_chord(browser, event, overlay, kind));
        }
        if let Some(result) = self.video_controls(event) {
            return Some(result);
        }
        if let Some(result) = self.minimal_sidebar_commands(browser, event) {
            return Some(result);
        }
        if let Some(result) = self.minimal_chrome_commands(browser, event) {
            return Some(result);
        }
        if let Some(result) = self.minimal_preview_keys(event) {
            return Some(result);
        }
        if overlay {
            if let Some(result) = self.minimal_search_commands(browser, event) {
                return Some(result);
            }
        } else if !self.preview_owns_keys() {
            // Claim keyboard ownership so paste lands in the keyboard
            // destination, never under a parked pointer. Search results own
            // their own selection, so this stays out of their way. Preview
            // ownership must not be stolen back by the listing.
            self.view.keyboard_navigation();
        }
        Some(self.minimal_motion(browser, event, overlay))
    }

    /// Search rows own navigation and selection; commands must not act on the hidden listing.
    fn minimal_search_commands(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        let plain =
            event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK);
        if plain {
            match event.key {
                Key::j | Key::Down => {
                    self.minimal_search_step(1);
                    return Some(Propagation::Stop);
                }
                Key::k | Key::Up => {
                    self.minimal_search_step(-1);
                    return Some(Propagation::Stop);
                }
                Key::h | Key::Left | Key::BackSpace => {
                    if self.view.view_mode() == BrowserMode::Icons && event.key != Key::BackSpace {
                        return Some(Propagation::Stop);
                    }
                    if !self.view.force_recursive_search() {
                        return None;
                    }
                    self.dismiss_recursive_search();
                    return Some(Propagation::Stop);
                }
                Key::o | Key::Return | Key::KP_Enter => {
                    return Some(self.activate_search_result(browser));
                }
                Key::O => {
                    if !browser.is_chooser_mode() {
                        self.select_search_cursor_if_fill_empty();
                        self.view.show_open_with();
                    }
                    return Some(Propagation::Stop);
                }
                Key::i => {
                    return Some(self.preview_search_result(browser));
                }
                Key::l | Key::Right | Key::KP_Right => {
                    if self.view.view_mode() == BrowserMode::Icons {
                        return Some(Propagation::Stop);
                    }
                    return Some(self.open_search_directory_or_preview(browser));
                }
                Key::space => {
                    self.minimal_search_space();
                    return Some(Propagation::Stop);
                }
                Key::G => {
                    self.minimal_search_jump(1);
                    return Some(Propagation::Stop);
                }
                Key::v => {
                    self.toggle_search_visual(MinimalVisual::Select);
                    return Some(Propagation::Stop);
                }
                Key::V => {
                    self.toggle_search_visual(MinimalVisual::Unset);
                    return Some(Propagation::Stop);
                }
                Key::Home
                | Key::End
                | Key::Page_Up
                | Key::Page_Down
                | Key::KP_Page_Up
                | Key::KP_Page_Down => {
                    // Paging still acts on the hidden directory cursor.
                    return Some(Propagation::Stop);
                }
                _ if self.view.view_mode() == BrowserMode::Icons
                    && icons_spatial_arrow(event.key).is_some() =>
                {
                    return Some(Propagation::Stop);
                }
                _ => {}
            }
        }
        // Page chords move the hidden directory cursor, so they stay swallowed
        // on result rows. Invert is listing-filter only: recursive `s` keeps
        // it swallowed; an `f` overlay falls through to invert among matches.
        if event.control()
            && event.without(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            && matches!(
                event.key,
                Key::u | Key::U | Key::d | Key::D | Key::b | Key::B | Key::f | Key::F
            )
        {
            return Some(Propagation::Stop);
        }
        if self.view.force_recursive_search()
            && event.control()
            && event.without(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            && matches!(event.key, Key::r | Key::R)
        {
            return Some(Propagation::Stop);
        }
        None
    }

    fn activate_search_result(&self, browser: &Rc<Browser>) -> Propagation {
        if let Some(entry) = self.focused_search_result() {
            if entry.is_directory() {
                browser.navigate(entry.location);
            } else {
                browser.open_location(entry.location);
            }
        }
        Propagation::Stop
    }

    fn preview_search_result(&self, browser: &Rc<Browser>) -> Propagation {
        if let Some(entry) = self.focused_search_result() {
            self.preview
                .toggle(preview_target(Some(entry)), browser.active_depth());
            if !self.preview.is_enabled() {
                self.leave_preview_keys();
            }
        }
        Propagation::Stop
    }

    fn open_search_directory_or_preview(&self, browser: &Rc<Browser>) -> Propagation {
        match self.focused_search_result() {
            Some(entry) if entry.is_directory() => self.activate_search_result(browser),
            Some(entry) => {
                self.preview_file_if_possible(browser, entry);
                Propagation::Stop
            }
            None => Propagation::Stop,
        }
    }

    /// Displayed search-result row under the independent search cursor.
    /// Selection-based helpers stay on the leftover fill after Space.
    fn focused_search_result(&self) -> Option<FileEntry> {
        let listing = self.view.search_result_listing()?;
        if listing.is_empty() {
            return None;
        }
        let index = self
            .current_search_cursor()
            .map(|index| index as usize)
            .unwrap_or(0)
            .min(listing.len() - 1);
        listing.get(index).cloned()
    }

    fn rename_target_entry(&self, browser: &Rc<Browser>) -> Option<FileEntry> {
        // An empty overlay is still the active result list. Falling through to
        // the directory cursor would rename a file the search is hiding.
        if self.view.selected_search_results().is_some() {
            return self.focused_search_result();
        }
        browser.focused_entry()
    }

    /// An empty `f` or `s` fill still has an independent search cursor. Select
    /// that hit so yank, cut, delete, copy, and open-with use the same entry
    /// as `o`, `i`, and `l`. A non-empty fill is left unchanged.
    fn select_search_cursor_if_fill_empty(&self) {
        let Some(entries) = self.view.selected_search_results() else {
            return;
        };
        if !entries.is_empty() {
            return;
        }
        let count = self.view.search_hit_count();
        if count == 0 {
            return;
        }
        let index = self.current_search_cursor().unwrap_or(0).min(count - 1);
        self.view.focus_search_hit(index, true);
        self.remember_search_cursor(index);
    }

    fn open_directory_or_preview_file(&self, browser: &Rc<Browser>) {
        match browser.focused_entry() {
            Some(entry) if entry.is_directory() => {
                self.leave_directory();
                self.view.activate_focused();
                self.view.restore_file_view_focus();
            }
            Some(entry) => self.preview_file_if_possible(browser, entry),
            None => self.toggle_listing_preview(browser),
        }
    }

    fn preview_file_if_possible(&self, browser: &Rc<Browser>, entry: FileEntry) {
        let Some(target) = preview_target(Some(entry)) else {
            self.shortcuts.flash("Nothing to preview");
            return;
        };
        self.preview.show(target, browser.active_depth());
        self.enter_preview_keys();
    }

    fn toggle_listing_preview(&self, browser: &Rc<Browser>) {
        if !self.preview.is_open() && browser.focused_entry().is_none() {
            self.shortcuts.flash("Nothing to preview");
        } else {
            self.preview.toggle(
                preview_target(browser.focused_entry()),
                browser.active_depth(),
            );
            if !self.preview.is_enabled() {
                self.leave_preview_keys();
            }
        }
    }

    fn preview_owns_keys(&self) -> bool {
        self.minimal.borrow().preview_owns_keys() && self.preview.is_enabled()
    }

    fn enter_preview_keys(&self) {
        self.minimal.borrow_mut().set_preview_owns_keys(true);
        self.preview.set_owns_keys_chrome(true);
        self.view.set_column_header_focus(false);
        let _ = self.preview.grab_pane_focus();
        let preview = self.preview.downgrade();
        let minimal = Rc::downgrade(self.minimal);
        glib::idle_add_local_once(move || {
            if let Some(minimal) = minimal.upgrade()
                && minimal.borrow().preview_owns_keys()
                && let Some(preview) = preview.upgrade()
                && preview.is_enabled()
            {
                let _ = preview.grab_pane_focus();
            }
        });
    }

    fn leave_preview_for_parent(&self) {
        self.leave_directory();
        self.leave_preview_keys();
        self.view.navigate_up();
    }

    pub(super) fn leave_preview_keys(&self) {
        release_preview_keys(self.minimal, self.preview, self.view);
    }

    /// While the preview owns keys in List or Columns, folder motion scrolls
    /// the mapped document. Left/`h` return to the listing without
    /// `navigate_up`. Backspace and Alt+Up go to the parent on that same press.
    /// Right/`l` stay in the pane. Icons motion never uses this map.
    fn minimal_preview_keys(&self, event: &KeyEvent) -> KeyResult {
        if self.minimal.borrow().preview_owns_keys() && !self.preview.is_enabled() {
            self.leave_preview_keys();
            return None;
        }
        if !self.preview_owns_keys() {
            return None;
        }
        if self.view.view_mode() == BrowserMode::Icons {
            self.leave_preview_keys();
            return None;
        }
        if event.control()
            && event.without(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            && let Some(result) = self.preview_control_scroll(event)
        {
            return Some(result);
        }
        if event.alt()
            && event.without(Modifiers::CONTROL_MASK | Modifiers::SUPER_MASK)
            && event.key == Key::Up
        {
            self.leave_preview_for_parent();
            return Some(Propagation::Stop);
        }
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
            return None;
        }
        match event.key {
            Key::BackSpace => {
                self.leave_preview_for_parent();
                Some(Propagation::Stop)
            }
            Key::h | Key::Left => {
                self.leave_preview_keys();
                Some(Propagation::Stop)
            }
            Key::l | Key::Right | Key::KP_Right => Some(Propagation::Stop),
            Key::j | Key::Down => {
                self.preview.scroll_step(1);
                Some(Propagation::Stop)
            }
            Key::k | Key::Up => {
                self.preview.scroll_step(-1);
                Some(Propagation::Stop)
            }
            Key::G | Key::End => {
                self.preview.scroll_to_edge(1);
                Some(Propagation::Stop)
            }
            Key::Home => {
                self.preview.scroll_to_edge(-1);
                Some(Propagation::Stop)
            }
            Key::Page_Up | Key::KP_Page_Up => {
                self.preview.scroll_page(-1);
                Some(Propagation::Stop)
            }
            Key::Page_Down | Key::KP_Page_Down => {
                self.preview.scroll_page(1);
                Some(Propagation::Stop)
            }
            _ => None,
        }
    }

    fn preview_control_scroll(&self, event: &KeyEvent) -> KeyResult {
        match event.key {
            Key::u | Key::U => {
                self.preview.scroll_by(-1);
                Some(Propagation::Stop)
            }
            Key::d | Key::D => {
                self.preview.scroll_by(1);
                Some(Propagation::Stop)
            }
            Key::b | Key::B if !event.shift() => {
                self.preview.scroll_page(-1);
                Some(Propagation::Stop)
            }
            Key::f | Key::F => {
                self.preview.scroll_page(1);
                Some(Propagation::Stop)
            }
            _ => None,
        }
    }

    fn dismiss_recursive_search(&self) {
        self.minimal.borrow_mut().leave_visual();
        self.leave_preview_keys();
        self.restore_applied_filter();
        self.view.restore_file_view_focus();
    }

    fn current_search_cursor(&self) -> Option<u32> {
        self.minimal
            .borrow()
            .search_cursor()
            .or_else(|| self.view.search_hit_index())
    }

    fn remember_search_cursor(&self, index: u32) {
        self.minimal.borrow_mut().set_search_cursor(Some(index));
    }

    fn next_search_index(&self, direction: i32) -> Option<u32> {
        crate::ui::browser::search_result_navigation_position(
            self.current_search_cursor(),
            self.view.search_hit_count(),
            direction,
        )
    }

    fn jump_search_index(&self, direction: i32) -> Option<u32> {
        let count = self.view.search_hit_count();
        crate::ui::browser::search_result_navigation_position(None, count, direction)
    }

    fn minimal_search_step(&self, direction: i32) {
        let Some(next) = self.next_search_index(direction) else {
            if direction > 0 {
                if !self.view.move_inline_search_results(1) {
                    crate::ui::focus_navigation::activate_native_arrow(self.window, Key::Down);
                }
            } else if !self.view.move_inline_search_results(-1) {
                crate::ui::focus_navigation::activate_native_arrow(self.window, Key::Up);
            }
            if let Some(index) = self.view.search_hit_index() {
                self.remember_search_cursor(index);
            }
            return;
        };
        self.apply_search_motion(next);
    }

    fn minimal_search_jump(&self, direction: i32) {
        if self.view.search_hit_count() == 0 {
            if !self.view.jump_inline_search_results(direction)
                && self.view.view_mode() == BrowserMode::Columns
            {
                self.view.jump_selection(direction);
            }
            return;
        }
        let Some(next) = self.jump_search_index(direction) else {
            return;
        };
        self.apply_search_motion(next);
    }

    fn apply_search_motion(&self, next: u32) {
        let visual = self.minimal.borrow().visual();
        match visual {
            Some(MinimalVisual::Select) => {
                let anchor = self
                    .minimal
                    .borrow()
                    .search_anchor()
                    .or_else(|| self.current_search_cursor())
                    .unwrap_or(next);
                self.view.select_search_hit_range(anchor, next);
                self.view.focus_search_hit(next, false);
            }
            Some(MinimalVisual::Unset) => {
                let previous = self.current_search_cursor().unwrap_or(next);
                self.view.unselect_search_hit_range(previous, next);
                self.view.focus_search_hit(next, false);
            }
            None => {
                let keep_fill = self.search_should_keep_fill();
                self.view.focus_search_hit(next, !keep_fill);
            }
        }
        self.remember_search_cursor(next);
    }

    fn search_should_keep_fill(&self) -> bool {
        if self.minimal.borrow().explicit_fill() {
            return true;
        }
        let count = self.view.search_hit_selected_count();
        if count != 1 {
            return count > 1;
        }
        self.current_search_cursor()
            .is_none_or(|index| !self.view.search_hit_is_selected(index))
    }

    fn minimal_search_space(&self) {
        let Some(index) = self.current_search_cursor().or_else(|| {
            crate::ui::browser::search_result_navigation_position(
                None,
                self.view.search_hit_count(),
                1,
            )
        }) else {
            self.shortcuts.flash("Nothing to select");
            return;
        };
        let visual = self.minimal.borrow().visual();
        if visual.is_some() {
            self.view.toggle_search_hit(index);
            self.view.focus_search_hit(index, false);
            self.remember_search_cursor(index);
            self.minimal.borrow_mut().set_explicit_fill(true);
            return;
        }
        let cursor_only =
            self.view.search_hit_selected_count() == 1 && self.view.search_hit_is_selected(index);
        if !cursor_only || self.minimal.borrow().explicit_fill() {
            self.view.toggle_search_hit(index);
        }
        self.minimal.borrow_mut().set_explicit_fill(true);
        self.remember_search_cursor(index);
        self.minimal_search_step(1);
    }

    fn toggle_search_visual(&self, kind: MinimalVisual) {
        if self.minimal.borrow().visual() == Some(kind) {
            self.minimal.borrow_mut().leave_visual();
            self.minimal.borrow_mut().set_search_anchor(None);
            return;
        }
        let index = self.current_search_cursor().or_else(|| {
            crate::ui::browser::search_result_navigation_position(
                None,
                self.view.search_hit_count(),
                1,
            )
        });
        self.minimal.borrow_mut().set_search_anchor(index);
        if let Some(index) = index {
            self.remember_search_cursor(index);
        }
        self.minimal.borrow_mut().enter_visual(kind);
    }

    fn minimal_sidebar_commands(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if !self.sidebar.contains(&event.focused)
            || !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            return None;
        }
        match event.key {
            Key::j | Key::Down => {
                self.sidebar.widget.child_focus(gtk::DirectionType::Down);
            }
            Key::k | Key::Up => {
                self.sidebar.widget.child_focus(gtk::DirectionType::Up);
            }
            Key::h | Key::Left | Key::BackSpace => {
                self.sidebar.restore(browser, false);
            }
            Key::l | Key::Right | Key::Return | Key::KP_Enter | Key::space => {
                crate::ui::focus_navigation::activate(&self.sidebar.widget);
            }
            _ => return None,
        }
        Some(Propagation::Stop)
    }

    fn minimal_chrome_commands(&self, browser: &Rc<Browser>, event: &mut KeyEvent) -> KeyResult {
        if event.text_has_focus()
            || self.sidebar.contains(&event.focused)
            || (!self.top_bar.has_focus() && !self.view.header_actions_have_focus())
            || !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            return None;
        }
        if matches!(event.key, Key::Return | Key::KP_Enter | Key::space) {
            crate::ui::focus_navigation::activate(self.window.upcast_ref());
            return Some(Propagation::Stop);
        }
        if let Some(direction) = vim_focus_direction(event.key) {
            event.key = match direction {
                gtk::DirectionType::Left => Key::Left,
                gtk::DirectionType::Right => Key::Right,
                gtk::DirectionType::Up => Key::Up,
                gtk::DirectionType::Down => Key::Down,
                _ => event.key,
            };
        }
        if let Some(result) = self.focus_navigation(browser, event) {
            return Some(result);
        }
        if matches!(
            event.key,
            Key::Left | Key::Right | Key::Up | Key::Down | Key::BackSpace
        ) {
            browser.focus_active();
            return Some(Propagation::Stop);
        }
        None
    }

    fn minimal_motion(
        &self,
        browser: &Rc<Browser>,
        event: &KeyEvent,
        searching: bool,
    ) -> Propagation {
        if is_context_menu_shortcut(event.key, event.modifiers) {
            self.view.open_focused_context_menu();
            return Propagation::Stop;
        }
        if is_sidebar_focus_shortcut(event.key, event.modifiers) {
            // The header toggle still shows a hidden sidebar; this only moves
            // focus when the sidebar is already visible.
            self.view.keyboard_navigation();
            if self.sidebar.contains(&event.focused) {
                self.sidebar.restore(browser, false);
            } else if self.top_bar.sidebar_toggle().is_active() {
                self.sidebar.previous.replace(event.focused.clone());
                let sidebar = self.sidebar.state.clone();
                glib::idle_add_local_once(move || {
                    sidebar.focus_active_place();
                });
            }
            return Propagation::Stop;
        }
        if is_undo_shortcut(event.key, event.modifiers) && self.view.undo_last_operation() {
            return Propagation::Stop;
        }
        if event.control()
            && event.without(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            && let Some(result) = self.minimal_control_chords(browser, event, searching)
        {
            return result;
        }
        if event.alt() && event.without(Modifiers::CONTROL_MASK | Modifiers::SUPER_MASK) {
            match event.key {
                Key::Left => browser.back(),
                Key::Right => browser.forward(),
                Key::Up => self.view.navigate_up(),
                Key::Return | Key::KP_Enter if !event.shift() => {
                    self.view.show_focused_properties();
                    return Propagation::Stop;
                }
                _ => return self.minimal_fallback(event),
            }
            self.leave_directory();
            return Propagation::Stop;
        }
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
            return self.minimal_fallback(event);
        }
        let visual = self.minimal.borrow().visual();
        if !searching
            && self.view.view_mode() == BrowserMode::Icons
            && let Some(arrow) = icons_spatial_arrow(event.key)
        {
            self.icons_spatial_step(browser, visual, arrow);
            return Propagation::Stop;
        }
        match event.key {
            Key::h | Key::Left | Key::BackSpace => {
                self.leave_directory();
                self.view.navigate_up();
            }
            Key::l | Key::Right | Key::KP_Right => self.open_directory_or_preview_file(browser),
            Key::o | Key::Return | Key::KP_Enter => {
                if browser
                    .focused_entry()
                    .is_some_and(|entry| entry.is_directory())
                {
                    self.leave_directory();
                }
                self.view.activate_focused();
            }
            Key::O => {
                if !browser.is_chooser_mode() {
                    self.select_search_cursor_if_fill_empty();
                    self.view.show_open_with();
                }
            }
            Key::j | Key::Down => self.minimal_step(browser, visual, 1, searching),
            Key::k | Key::Up => self.minimal_step(browser, visual, -1, searching),
            Key::g => self.arm_chord(MinimalChord::Go),
            Key::c => self.arm_chord(MinimalChord::Copy),
            Key::comma => self.arm_chord(MinimalChord::Sort),
            Key::semicolon => self.arm_chord(MinimalChord::Action),
            Key::period => browser.toggle_hidden(),
            Key::F2 => self.open_rename_prompt(browser),
            Key::F5 => self.view.refresh(),
            Key::Delete | Key::KP_Delete => {
                self.leave_visual_keep_fill();
                self.select_search_cursor_if_fill_empty();
                self.view.select_focused_if_empty();
                if event.shift() {
                    self.view.confirm_delete_preferring_cancel();
                } else {
                    self.view.confirm_trash();
                }
            }
            Key::G | Key::End => self.minimal_jump(browser, visual, 1, searching),
            Key::Home => self.minimal_jump(browser, visual, -1, searching),
            Key::Page_Up | Key::KP_Page_Up => {
                self.minimal_full_page(browser, visual, -1, searching)
            }
            Key::Page_Down | Key::KP_Page_Down => {
                self.minimal_full_page(browser, visual, 1, searching)
            }
            Key::H => {
                self.leave_directory();
                browser.back();
            }
            Key::L => {
                self.leave_directory();
                browser.forward();
            }
            Key::i => self.toggle_listing_preview(browser),
            Key::J => {
                self.preview.scroll_by(1);
            }
            Key::K => {
                self.preview.scroll_by(-1);
            }
            Key::v => self.toggle_visual(browser, MinimalVisual::Select),
            Key::V => self.toggle_visual(browser, MinimalVisual::Unset),
            Key::space => self.minimal_space(browser, visual, searching),
            Key::y => {
                self.leave_visual_keep_fill();
                self.select_search_cursor_if_fill_empty();
                self.view.select_focused_if_empty();
                if !self.view.copy_selection() {
                    self.shortcuts.flash("Nothing to yank");
                }
            }
            Key::x => {
                self.leave_visual_keep_fill();
                self.select_search_cursor_if_fill_empty();
                self.view.select_focused_if_empty();
                if !self.view.cut_selection() {
                    self.shortcuts.flash("Nothing to cut");
                }
            }
            Key::p => self.minimal_paste(false),
            Key::P => self.minimal_paste(true),
            Key::Y | Key::X => {
                self.view.clear_yank_marks(&self.window.clipboard());
            }
            Key::d => {
                self.leave_visual_keep_fill();
                self.select_search_cursor_if_fill_empty();
                self.view.select_focused_if_empty();
                if !self.view.confirm_trash() {
                    self.shortcuts.flash("Nothing to delete");
                }
            }
            Key::D => {
                self.leave_visual_keep_fill();
                self.select_search_cursor_if_fill_empty();
                self.view.select_focused_if_empty();
                self.view.confirm_delete_preferring_cancel();
            }
            Key::slash => {
                self.open_prompt(browser, MinimalPrompt::FindNext, "");
            }
            Key::question => {
                self.open_prompt(browser, MinimalPrompt::FindPrev, "");
            }
            Key::f => {
                self.open_prompt(
                    browser,
                    MinimalPrompt::Filter,
                    &self.view.hidden_filter_query(),
                );
            }
            Key::s => {
                self.open_prompt(browser, MinimalPrompt::Search, "");
            }
            Key::a => {
                self.open_prompt(browser, MinimalPrompt::NewEntry, "");
            }
            Key::r => {
                self.open_rename_prompt(browser);
            }
            Key::z => {
                self.open_prompt(browser, MinimalPrompt::HistoryFuzzy, "");
            }
            Key::Z => {
                self.open_prompt(browser, MinimalPrompt::HistoryRecent, "");
            }
            Key::n => {
                self.repeat_find(browser, 0);
            }
            Key::N => {
                self.repeat_find(browser, 1);
            }
            Key::q => {
                self.leave_preview_keys();
                self.type_to_search.preferences.set_minimal_mode(false);
                self.shortcuts
                    .flash("Left minimal mode — Ctrl+Shift+M returns");
            }
            Key::Q => {
                self.window.close();
            }
            Key::asciitilde => {
                self.shortcuts.toggle_reference();
            }
            Key::Escape => return self.minimal_escape(browser),
            _ => return self.minimal_fallback(event),
        }
        Propagation::Stop
    }

    /// Ctrl chords that stay bound in minimal mode. Conflicting Yazi chords
    /// (`Ctrl+D` half-page, `Ctrl+R` invert, `Ctrl+F`/`Ctrl+B` paging) keep
    /// their Yazi meaning; non-conflicting GUI conventions stay bound.
    fn minimal_control_chords(
        &self,
        browser: &Rc<Browser>,
        event: &KeyEvent,
        searching: bool,
    ) -> KeyResult {
        match event.key {
            Key::a | Key::A if !event.shift() => {
                if self.view.filter_has_focus() {
                    return Some(Propagation::Proceed);
                }
                self.view.select_all();
                self.minimal.borrow_mut().set_explicit_fill(true);
            }
            Key::c if !event.shift() => {
                if self.view.filter_has_focus() || event.text_has_focus() {
                    return Some(Propagation::Proceed);
                }
                self.leave_visual_keep_fill();
                self.select_search_cursor_if_fill_empty();
                self.view.select_focused_if_empty();
                self.view.copy_selection();
            }
            Key::x if !event.shift() => {
                if self.view.filter_has_focus() || event.text_has_focus() {
                    return Some(Propagation::Proceed);
                }
                self.leave_visual_keep_fill();
                self.select_search_cursor_if_fill_empty();
                self.view.select_focused_if_empty();
                self.view.cut_selection();
            }
            Key::v if !event.shift() => {
                if self.view.filter_has_focus() || event.text_has_focus() {
                    return Some(Propagation::Proceed);
                }
                self.minimal_paste(true);
            }
            Key::k | Key::K if !event.shift() => {
                if !browser.is_chooser_mode()
                    && let Err(error) =
                        gtk::prelude::WidgetExt::activate_action(self.window, "win.search", None)
                {
                    tracing::warn!(%error, "unable to activate global search shortcut");
                }
            }
            Key::l | Key::L if !event.shift() => {
                self.view.begin_location_edit();
            }
            Key::h | Key::H | Key::period if !event.shift() => {
                browser.toggle_hidden();
            }
            Key::n | Key::N if event.shift() => {
                self.view.create_new_folder();
            }
            Key::r | Key::R => {
                if self.view.filter_has_focus() {
                    return Some(Propagation::Proceed);
                }
                self.view.invert_selection();
                self.minimal.borrow_mut().set_explicit_fill(true);
            }
            Key::u | Key::U => {
                self.minimal_half_page(browser, -1, searching);
            }
            Key::d | Key::D => {
                self.minimal_half_page(browser, 1, searching);
            }
            Key::b | Key::B if !event.shift() => {
                self.minimal_full_page_control(browser, -1, searching);
            }
            Key::f | Key::F => {
                self.minimal_full_page_control(browser, 1, searching);
            }
            _ if !event.shift() && event.without(Modifiers::ALT_MASK | Modifiers::SUPER_MASK) => {
                let mode = browser_mode_for_digit(event.key)?;
                apply_browser_mode(
                    self.view,
                    &crate::ui::preferences::PreferenceManager::shared(),
                    mode,
                );
                if mode == BrowserMode::Icons {
                    self.leave_preview_keys();
                }
                self.sync_filter_mark();
            }
            _ => return None,
        }
        Some(Propagation::Stop)
    }

    fn minimal_half_page(&self, browser: &Rc<Browser>, direction: i32, searching: bool) {
        if searching {
            return;
        }
        let items = self
            .view
            .focused_page_items()
            .map(|items| (items / 2).max(1))
            .unwrap_or(1);
        self.minimal_page(browser, direction, items);
    }

    fn minimal_full_page_control(&self, browser: &Rc<Browser>, direction: i32, searching: bool) {
        if searching {
            return;
        }
        if self.minimal.borrow().visual().is_none() {
            if self.should_keep_fill(browser) {
                let items = self.view.focused_page_items().unwrap_or(1).max(1);
                self.view.move_cursor(direction, items);
                return;
            }
            self.view.page_selection(direction);
            return;
        }
        let items = self.view.focused_page_items().unwrap_or(1).max(1);
        self.minimal_page(browser, direction, items);
    }

    /// Page-sized motion shared by half, full, and viewport pages. Browse
    /// moves the cursor; visual extends or subtracts a range so paging keeps
    /// the fill instead of replacing it.
    fn minimal_page(&self, browser: &Rc<Browser>, direction: i32, items: usize) {
        if self.minimal.borrow().visual().is_none() {
            if self.should_keep_fill(browser) {
                self.view.move_cursor(direction, items);
                return;
            }
            self.view.page_by(direction, items);
            return;
        }
        let visual = self.minimal.borrow().visual();
        let order = self.view.active_visual_order();
        let Some(target) = browser.next_visible_index(direction, items, order.as_deref()) else {
            return;
        };
        self.visual_go(browser, visual, target, order.as_deref());
    }

    /// Icons `hjkl`/arrows stay on the grid: native spatial motion, no miller
    /// parent/enter, and no preview key ownership.
    fn icons_spatial_step(&self, browser: &Rc<Browser>, visual: Option<MinimalVisual>, arrow: Key) {
        self.view.keyboard_navigation();
        let Some((depth, before, _)) = browser.focused_item() else {
            crate::ui::focus_navigation::activate_native_arrow(self.window, arrow);
            return;
        };
        let fill = browser.selected_positions(depth);
        let keep = visual.is_none() && self.should_keep_fill(browser);
        if visual.is_none() && !keep {
            let _ = self.view.resume_native_selection();
        }
        if !crate::ui::focus_navigation::activate_native_arrow(self.window, arrow) {
            self.view.restore_file_view_focus();
            crate::ui::focus_navigation::activate_native_arrow(self.window, arrow);
        }
        self.view.synchronize_native_selection(false);
        let Some((_, after, _)) = browser.focused_item() else {
            return;
        };
        if after == before {
            return;
        }
        match visual {
            Some(MinimalVisual::Select) => {
                let order = self.view.active_visual_order();
                browser.extend_selection_to(depth, after, order.as_deref());
            }
            Some(MinimalVisual::Unset) => {
                let order = self.view.active_visual_order();
                browser.subtract_visual_selection(depth, before, after, order.as_deref());
            }
            None if keep => {
                browser.set_selection(depth, &fill, Some(after));
                browser.focus_keeping_fill(after);
            }
            None => {}
        }
    }

    fn minimal_step(
        &self,
        browser: &Rc<Browser>,
        visual: Option<MinimalVisual>,
        direction: i32,
        searching: bool,
    ) {
        if searching {
            self.minimal_search_step(direction);
            return;
        }
        if visual.is_none() {
            if self.should_keep_fill(browser) {
                self.view.move_cursor(direction, 1);
                return;
            }
            if self.view.view_mode() == BrowserMode::Columns {
                browser.move_selection(direction);
            } else {
                self.view.step_selection(direction);
            }
            return;
        }
        let order = self.view.active_visual_order();
        let Some(target) = browser.next_visible_index(direction, 1, order.as_deref()) else {
            return;
        };
        self.visual_go(browser, visual, target, order.as_deref());
    }

    fn minimal_jump(
        &self,
        browser: &Rc<Browser>,
        visual: Option<MinimalVisual>,
        direction: i32,
        searching: bool,
    ) {
        if searching {
            self.minimal_search_jump(direction);
            return;
        }
        if visual.is_none() {
            if self.should_keep_fill(browser) {
                self.view.move_cursor(direction, usize::MAX);
                return;
            }
            self.view.jump_selection(direction);
            return;
        }
        let order = self.view.active_visual_order();
        let Some(target) = browser.next_visible_index(direction, usize::MAX, order.as_deref())
        else {
            return;
        };
        self.visual_go(browser, visual, target, order.as_deref());
    }

    fn minimal_full_page(
        &self,
        browser: &Rc<Browser>,
        visual: Option<MinimalVisual>,
        direction: i32,
        searching: bool,
    ) {
        if searching {
            return;
        }
        if visual.is_none() {
            if self.should_keep_fill(browser) {
                let items = self.view.focused_page_items().unwrap_or(1).max(1);
                self.view.move_cursor(direction, items);
                return;
            }
            self.view.page_selection(direction);
            return;
        }
        let items = self.view.focused_page_items().unwrap_or(1).max(1);
        let order = self.view.active_visual_order();
        let Some(target) = browser.next_visible_index(direction, items, order.as_deref()) else {
            return;
        };
        self.visual_go(browser, visual, target, order.as_deref());
    }

    fn minimal_space(&self, browser: &Rc<Browser>, visual: Option<MinimalVisual>, searching: bool) {
        if searching {
            self.minimal_search_space();
            return;
        }
        if browser.focused_item().is_none() {
            self.shortcuts.flash("Nothing to select");
            return;
        }
        if visual.is_some() {
            self.view.toggle_focused_selection();
            return;
        }
        if !self.cursor_only_fill(browser) || self.minimal.borrow().explicit_fill() {
            self.view.toggle_focused_selection();
        }
        self.minimal.borrow_mut().set_explicit_fill(true);
        self.minimal_step(browser, None, 1, searching);
    }

    fn should_keep_fill(&self, browser: &Browser) -> bool {
        self.minimal.borrow().explicit_fill() || !self.cursor_only_fill(browser)
    }

    fn cursor_only_fill(&self, browser: &Browser) -> bool {
        let Some((depth, position, _)) = browser.focused_item() else {
            return true;
        };
        let selected = browser.selected_positions(depth);
        selected.len() == 1 && selected[0] == position
    }

    fn visual_go(
        &self,
        browser: &Rc<Browser>,
        visual: Option<MinimalVisual>,
        target: usize,
        order: Option<&[usize]>,
    ) {
        let Some((depth, cursor, _)) = browser.focused_item() else {
            return;
        };
        match visual {
            Some(MinimalVisual::Select) => {
                browser.extend_selection_to(depth, target, order);
            }
            Some(MinimalVisual::Unset) => {
                browser.subtract_visual_selection(depth, cursor, target, order);
            }
            None => {}
        }
        browser.focus_active();
    }

    fn toggle_visual(&self, browser: &Rc<Browser>, kind: MinimalVisual) {
        if self.view.selected_search_results().is_some() {
            self.toggle_search_visual(kind);
            return;
        }
        if self.minimal.borrow().visual() == Some(kind) {
            self.minimal.borrow_mut().leave_visual();
            return;
        }
        if let Some((depth, position, _)) = browser.focused_item() {
            browser.set_selection_anchor(depth, position);
        }
        self.minimal.borrow_mut().enter_visual(kind);
    }

    fn leave_visual_keep_fill(&self) {
        self.minimal.borrow_mut().leave_visual();
    }

    fn clipboard_offers_paste(&self) -> bool {
        let formats = self.window.clipboard().formats();
        formats.contains_type(gtk::gdk::FileList::static_type())
            || formats.contain_mime_type("text/uri-list")
            || formats.contains_type(gtk::gdk::Texture::static_type())
            || formats.contain_mime_type("image/png")
    }

    fn minimal_paste(&self, prefer_replace: bool) {
        self.leave_visual_keep_fill();
        if !self.clipboard_offers_paste() {
            self.shortcuts.flash("Nothing to paste");
            return;
        }
        if self.minimal.borrow().explicit_fill() {
            if prefer_replace {
                self.view.paste_preferring_replace();
            } else {
                self.view.paste_preferring_keep_both();
            }
        } else {
            self.view.paste_into_listing(prefer_replace);
        }
    }

    fn leave_directory(&self) {
        let mut minimal = self.minimal.borrow_mut();
        minimal.leave_visual();
        minimal.set_explicit_fill(false);
    }

    fn minimal_escape(&self, browser: &Rc<Browser>) -> Propagation {
        if self.view.force_recursive_search() && self.view.selected_search_results().is_some() {
            if self.minimal.borrow().visual().is_some() {
                self.minimal.borrow_mut().leave_visual();
                self.minimal.borrow_mut().set_search_anchor(None);
                return Propagation::Stop;
            }
            if self.preview.is_enabled() {
                self.preview.close();
                self.leave_preview_keys();
                return Propagation::Stop;
            }
            self.dismiss_recursive_search();
            return Propagation::Stop;
        }
        if self.view.hidden_filter_active() {
            let keep = kept_filter_fill(self.view, browser);
            self.minimal.borrow_mut().clear_applied_filter();
            self.view.dismiss_hidden_filter();
            self.sync_filter_mark();
            self.view.restore_file_view_focus();
            restore_fill_after_hidden_filter(
                self.view.clone(),
                browser.clone(),
                self.minimal.clone(),
                keep,
                true,
            );
            return Propagation::Stop;
        }
        if self.view.find_highlights_active() {
            self.view.set_find_highlight("");
            return Propagation::Stop;
        }
        if self.minimal.borrow().visual().is_some() {
            self.minimal.borrow_mut().leave_visual();
            return Propagation::Stop;
        }
        if self.preview.is_enabled() {
            self.preview.close();
            self.leave_preview_keys();
            return Propagation::Stop;
        }
        if browser.is_chooser_mode() {
            self.window.close();
            return Propagation::Stop;
        }
        {
            let mut minimal = self.minimal.borrow_mut();
            minimal.cancel_fill_restore();
            minimal.set_explicit_fill(false);
        }
        browser.clear_active_selection();
        Propagation::Stop
    }

    // --- Footer prompts ---

    fn open_prompt(&self, browser: &Rc<Browser>, kind: MinimalPrompt, initial: &str) {
        // Prompts replace visual (keeping the fill) and never stack on a chord.
        self.cancel_chord_ui();
        self.minimal.borrow_mut().leave_visual();
        self.minimal.borrow_mut().enter_prompt(kind);
        self.view.keyboard_navigation();
        match kind {
            MinimalPrompt::FindNext | MinimalPrompt::FindPrev => {
                let direction = if kind == MinimalPrompt::FindNext {
                    1
                } else {
                    -1
                };
                let view = self.view.downgrade();
                let footer = self.shortcuts.downgrade();
                let lock = self.prompt_focus_lock.clone();
                self.view.set_find_prompt_active(true);
                self.shortcuts.show_prompt(
                    kind.prefix(),
                    kind.placeholder(),
                    initial,
                    Some(Rc::new(move |text| {
                        let (Some(view), Some(footer)) = (view.upgrade(), footer.upgrade()) else {
                            return;
                        };
                        lock.set(true);
                        view.set_find_highlight(&text);
                        view.find_in_listing(&text, direction);
                        // select() emits FocusChanged, which grabs the list.
                        footer.prompt_entry_widget().grab_focus_without_selecting();
                        lock.set(false);
                    })),
                );
            }
            MinimalPrompt::Filter => {
                let view = self.view.downgrade();
                let footer = self.shortcuts.downgrade();
                let lock = self.prompt_focus_lock.clone();
                // Seeding `f` from a live recursive `s` must not rewrite that
                // feed as a non-recursive filter before Enter.
                let preserve_search = self.recursive_hits_showing();
                self.shortcuts.show_prompt(
                    kind.prefix(),
                    kind.placeholder(),
                    initial,
                    Some(Rc::new(move |text| {
                        let (Some(view), Some(footer)) = (view.upgrade(), footer.upgrade()) else {
                            return;
                        };
                        lock.set(true);
                        view.set_filter_query_without_revealer(&text);
                        footer.set_filter_mark(&view.hidden_filter_query());
                        footer.prompt_entry_widget().grab_focus_without_selecting();
                        lock.set(false);
                    })),
                );
                if !initial.is_empty() && !preserve_search {
                    self.view.set_filter_query_without_revealer(initial);
                    self.sync_filter_mark();
                }
            }
            MinimalPrompt::Search => {
                let view = self.view.downgrade();
                let footer = self.shortcuts.downgrade();
                let lock = self.prompt_focus_lock.clone();
                self.view.set_find_prompt_active(true);
                self.shortcuts.show_prompt(
                    kind.prefix(),
                    kind.placeholder(),
                    initial,
                    Some(Rc::new(move |text| {
                        let (Some(view), Some(footer)) = (view.upgrade(), footer.upgrade()) else {
                            return;
                        };
                        lock.set(true);
                        view.set_recursive_search_query(&text);
                        footer.set_query_mark(
                            &view.hidden_filter_query(),
                            view.force_recursive_search(),
                        );
                        footer.prompt_entry_widget().grab_focus_without_selecting();
                        lock.set(false);
                    })),
                );
                if !initial.is_empty() {
                    self.view.set_recursive_search_query(initial);
                }
            }
            MinimalPrompt::Goto => {
                let minimal = Rc::downgrade(self.minimal);
                let lock = self.prompt_focus_lock.clone();
                self.shortcuts.show_prompt(
                    kind.prefix(),
                    kind.placeholder(),
                    initial,
                    Some(Rc::new(move |_| {
                        if !lock.get()
                            && let Some(minimal) = minimal.upgrade()
                        {
                            minimal.borrow_mut().clear_goto_completion();
                        }
                    })),
                );
            }
            MinimalPrompt::HistoryFuzzy | MinimalPrompt::HistoryRecent => {
                let footer = self.shortcuts.downgrade();
                self.shortcuts.show_prompt(
                    kind.prefix(),
                    kind.placeholder(),
                    initial,
                    Some(Rc::new(move |text| {
                        if let Some(footer) = footer.upgrade() {
                            footer.show_history_candidates(history_candidates(kind, &text), &text);
                        }
                    })),
                );
                self.shortcuts
                    .show_history_candidates(history_candidates(kind, initial), initial);
            }
            _ => {
                self.shortcuts
                    .show_prompt(kind.prefix(), kind.placeholder(), initial, None);
            }
        }
        let _ = browser;
    }

    fn open_rename_prompt(&self, browser: &Rc<Browser>) {
        let Some(entry) = self.rename_target_entry(browser) else {
            self.shortcuts.flash("Nothing to rename");
            return;
        };
        self.cancel_chord_ui();
        self.minimal.borrow_mut().leave_visual();
        self.minimal
            .borrow_mut()
            .enter_prompt(MinimalPrompt::Rename);
        self.minimal
            .borrow_mut()
            .set_rename_target(Some(entry.clone()));
        self.view.keyboard_navigation();
        self.shortcuts
            .show_rename_prompt(&entry.display_name, entry.is_directory());
    }

    fn repeat_find(&self, browser: &Rc<Browser>, flip: i32) {
        let Some((query, direction)) = self.minimal.borrow().last_find() else {
            return;
        };
        let direction = if flip == 0 { direction } else { -direction };
        self.view.keyboard_navigation();
        self.view.find_in_listing(&query, direction);
        let _ = browser;
    }

    /// Prompt-focused keys: Enter submits, Escape cancels (clearing secrets),
    /// Up/Down moves the listing or search results without leaving the prompt,
    /// and Tab cycles folders in the go prompt.
    fn minimal_prompt_key(&self, browser: &Rc<Browser>, event: &mut KeyEvent) -> Propagation {
        let Some(kind) = self.minimal.borrow().prompt() else {
            return Propagation::Stop;
        };
        let in_entry = self.shortcuts.is_prompt_entry(&event.focused);
        // Native editing continues in the entry; only these keys are ours.
        if in_entry {
            match event.key {
                Key::Return | Key::KP_Enter
                    if event.without(
                        Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK,
                    ) =>
                {
                    let text = self.shortcuts.prompt_text();
                    return self.submit_prompt(browser, kind, &text);
                }
                Key::Escape
                    if event.without(
                        Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK,
                    ) =>
                {
                    self.cancel_prompt(browser);
                    return Propagation::Stop;
                }
                Key::Up | Key::Down
                    if event.without(
                        Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK,
                    ) =>
                {
                    let direction = if event.key == Key::Down { 1 } else { -1 };
                    self.prompt_focus_lock.set(true);
                    if matches!(
                        kind,
                        MinimalPrompt::HistoryFuzzy | MinimalPrompt::HistoryRecent
                    ) {
                        self.shortcuts.move_history_selection(direction);
                    } else if self.view.selected_search_results().is_some() {
                        self.minimal_search_step(direction);
                    } else {
                        self.minimal_step(browser, None, direction, false);
                    }
                    // Listing motion can steal focus; unlock after re-grab so
                    // a later pointer click still dismisses.
                    self.shortcuts.prompt_entry_widget().grab_focus();
                    self.prompt_focus_lock.set(false);
                    return Propagation::Stop;
                }
                Key::Tab | Key::ISO_Left_Tab
                    if kind == MinimalPrompt::Goto
                        && event.without(
                            Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK,
                        ) =>
                {
                    let reverse = event.shift() || event.key == Key::ISO_Left_Tab;
                    self.complete_goto_prompt(browser, reverse);
                    return Propagation::Stop;
                }
                _ => return Propagation::Proceed,
            }
        }
        // Pointer focus-loss already closed the prompt. If a key arrives
        // while Prompt is still armed without entry focus, Escape/Enter
        // still finish it; other keys cancel first then behave as Browse.
        match event.key {
            Key::Escape
                if event.without(
                    Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK,
                ) =>
            {
                self.cancel_prompt(browser);
                Propagation::Stop
            }
            Key::Return | Key::KP_Enter
                if event.without(
                    Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK,
                ) =>
            {
                let text = self.shortcuts.prompt_text();
                self.submit_prompt(browser, kind, &text)
            }
            _ => {
                self.cancel_prompt(browser);
                self.minimal_commands(browser, event)
                    .unwrap_or(Propagation::Proceed)
            }
        }
    }

    fn complete_goto_prompt(&self, browser: &Rc<Browser>, reverse: bool) {
        self.shortcuts.set_prompt_hint("");
        let text = self.shortcuts.prompt_text();
        let current = browser
            .active_location()
            .and_then(|location| location.native_path().map(std::path::Path::to_path_buf));
        let listing = if !text.contains('/') && !text.starts_with('~') {
            goto_listing_folders(browser)
        } else {
            Vec::new()
        };
        let mut cycle = self.minimal.borrow_mut().take_goto_cycle();
        let minimal = Rc::downgrade(self.minimal);
        let footer = self.shortcuts.downgrade();
        let window = self.window.downgrade();
        let browser_weak = Rc::downgrade(browser);
        let navigation = browser.navigation_generation();
        let lock = self.prompt_focus_lock.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let completed = cycle_goto_path(
                &text,
                current.as_deref(),
                &glib::home_dir(),
                &listing,
                reverse,
                &mut cycle,
            )
            .await;
            let (Some(minimal), Some(footer), Some(window), Some(browser)) = (
                minimal.upgrade(),
                footer.upgrade(),
                window.upgrade(),
                browser_weak.upgrade(),
            ) else {
                return;
            };
            if !window.is_mapped()
                || !crate::ui::preferences::PreferenceManager::shared().minimal_mode()
                || minimal.borrow().prompt() != Some(MinimalPrompt::Goto)
                || browser.navigation_generation() != navigation
                || footer.prompt_text() != text
            {
                return;
            }
            match completed {
                Ok(completed) => {
                    minimal.borrow_mut().set_goto_cycle(cycle);
                    if let Some(completed) = completed {
                        lock.set(true);
                        let entry = footer.prompt_entry_widget();
                        entry.set_text(&completed);
                        entry.set_position(-1);
                        entry.grab_focus_without_selecting();
                        lock.set(false);
                    }
                }
                Err(_) => {
                    minimal.borrow_mut().set_goto_cycle(None);
                    footer.set_prompt_hint("Check or refine the folder path");
                }
            }
        });
        self.minimal.borrow_mut().set_goto_request(task);
    }

    fn submit_prompt(&self, browser: &Rc<Browser>, kind: MinimalPrompt, text: &str) -> Propagation {
        // Selection and navigation emit focus-leave synchronously while view models are borrowed.
        let previous = self.prompt_focus_lock.replace(true);
        let result = self.apply_prompt(browser, kind, text);
        self.prompt_focus_lock.set(previous);
        result
    }

    fn apply_prompt(&self, browser: &Rc<Browser>, kind: MinimalPrompt, text: &str) -> Propagation {
        match kind {
            MinimalPrompt::FindNext | MinimalPrompt::FindPrev => {
                let direction = if kind == MinimalPrompt::FindNext {
                    1
                } else {
                    -1
                };
                let query = text.trim().to_owned();
                if !query.is_empty() {
                    self.minimal
                        .borrow_mut()
                        .record_find(query.clone(), direction);
                    self.view.set_find_highlight(&query);
                    self.view.find_in_listing(&query, direction);
                    tracing::debug!("minimal find submitted");
                }
                self.close_prompt(browser);
            }
            MinimalPrompt::Filter => {
                if text.trim().is_empty() {
                    self.minimal.borrow_mut().clear_applied_filter();
                    self.view.dismiss_hidden_filter();
                } else {
                    self.minimal
                        .borrow_mut()
                        .remember_applied_filter(text.to_owned());
                    self.view.set_filter_query_without_revealer(text);
                }
                self.sync_filter_mark();
                tracing::debug!("minimal filter submitted");
                self.close_prompt_to_results(browser);
            }
            MinimalPrompt::Search => {
                self.view.set_recursive_search_query(text);
                self.sync_filter_mark();
                tracing::debug!("minimal search submitted");
                self.close_prompt_to_results(browser);
                return Propagation::Stop;
            }
            MinimalPrompt::NewEntry => {
                if !self.submit_new_entry(browser, text) {
                    // Invalid names stay in the prompt so they can be fixed.
                    return Propagation::Stop;
                }
                self.close_prompt(browser);
            }
            MinimalPrompt::Rename => {
                if !self.submit_rename(browser, text) {
                    return Propagation::Stop;
                }
                self.close_prompt(browser);
            }
            MinimalPrompt::Goto => {
                let input = text.to_owned();
                // Clear secrets before navigating.
                self.minimal.borrow_mut().leave_prompt();
                self.shortcuts.hide_prompt();
                self.view.set_find_prompt_active(false);
                if input.trim().is_empty() {
                    self.view.restore_file_view_focus();
                } else {
                    let current = browser.active_location().and_then(|location| {
                        location.native_path().map(std::path::Path::to_path_buf)
                    });
                    let resolved = resolve_goto_input(input.trim(), current.as_deref());
                    self.view.submit_footer_location(&resolved);
                }
                tracing::debug!("minimal goto submitted");
                return Propagation::Stop;
            }
            MinimalPrompt::HistoryFuzzy | MinimalPrompt::HistoryRecent => {
                self.submit_history(browser);
                self.close_prompt(browser);
            }
        }
        Propagation::Stop
    }

    fn submit_new_entry(&self, browser: &Rc<Browser>, text: &str) -> bool {
        let is_dir = text.ends_with('/');
        let mut name = text.to_owned();
        if is_dir {
            name.pop();
        }
        if name.is_empty() {
            self.shortcuts.flash("Enter a name");
            return false;
        }
        if let Err(message) = crate::services::validate_basename(&name) {
            self.shortcuts.flash(message);
            return false;
        }
        let Some(parent) = browser.active_location() else {
            self.shortcuts.flash("No destination folder");
            return false;
        };
        if is_dir {
            browser.create_directory_named(parent, name);
        } else {
            browser.create_file_named(parent, name);
        }
        tracing::debug!("minimal new entry submitted");
        true
    }

    fn submit_rename(&self, browser: &Rc<Browser>, text: &str) -> bool {
        let captured = self.minimal.borrow().rename_target().cloned();
        let Some(entry) = captured.or_else(|| self.rename_target_entry(browser)) else {
            self.shortcuts.flash("No focused item");
            return false;
        };
        if text == entry.display_name {
            return true;
        }
        if let Err(message) = crate::services::validate_basename(text) {
            self.shortcuts.flash(message);
            return false;
        }
        crate::ui::browser::queue_rename(browser, entry, text.to_owned());
        tracing::debug!("minimal rename submitted");
        true
    }

    fn submit_history(&self, browser: &Rc<Browser>) {
        if let Some(path) = self.shortcuts.selected_history_path() {
            browser.navigate(Location::local(path));
            tracing::debug!("minimal history jump submitted");
        } else {
            self.shortcuts.flash("No matching folders");
        }
    }

    fn cancel_prompt(&self, browser: &Rc<Browser>) {
        // Escape in a filter prompt discards the query. Search with hits
        // keeps the result list; empty search cancels. Secrets always clear.
        let kind = self.minimal.borrow().prompt();
        let keep_search = kind == Some(MinimalPrompt::Search)
            && self.view.force_recursive_search()
            && self.view.selected_search_results().is_some();
        self.minimal.borrow_mut().leave_prompt();
        self.shortcuts.hide_prompt();
        self.view.set_find_prompt_active(false);
        if matches!(
            kind,
            Some(MinimalPrompt::FindNext | MinimalPrompt::FindPrev)
        ) {
            self.view.set_find_highlight("");
        }
        if matches!(kind, Some(MinimalPrompt::Filter)) {
            // Opening `f` over a live recursive `s` only displays that search.
            // Esc must leave the hits and the search mark; it is not a filter cancel.
            if live_recursive_search(self.view) {
                self.sync_filter_mark();
                self.view.restore_file_view_focus();
            } else {
                let keep = kept_filter_fill(self.view, browser);
                {
                    let mut minimal = self.minimal.borrow_mut();
                    minimal.clear_applied_filter();
                    // The walked hit cursor and explicit fill belong to this
                    // prompt. Leaving them makes the next `s` advance an empty
                    // selection instead of selecting the first hit.
                    minimal.clear_search_nav();
                    minimal.set_explicit_fill(false);
                }
                self.view.dismiss_hidden_filter();
                self.sync_filter_mark();
                self.view.restore_file_view_focus();
                restore_fill_after_hidden_filter(
                    self.view.clone(),
                    browser.clone(),
                    self.minimal.clone(),
                    keep,
                    false,
                );
            }
        } else if keep_search {
            self.focus_kept_search_results();
            self.sync_filter_mark();
        } else if matches!(kind, Some(MinimalPrompt::Search)) {
            if self.view.force_recursive_search() || self.view.selected_search_results().is_some() {
                self.restore_applied_filter();
            }
            self.view.restore_file_view_focus();
        } else {
            self.view.restore_file_view_focus();
        }
    }

    fn close_prompt(&self, _browser: &Rc<Browser>) {
        self.minimal.borrow_mut().leave_prompt();
        self.shortcuts.hide_prompt();
        self.view.set_find_prompt_active(false);
        self.view.restore_file_view_focus();
    }

    fn close_prompt_to_results(&self, _browser: &Rc<Browser>) {
        self.minimal.borrow_mut().leave_prompt();
        self.shortcuts.hide_prompt();
        self.view.set_find_prompt_active(false);
        self.focus_kept_search_results();
    }

    fn focus_kept_search_results(&self) {
        if self.view.focus_first_search_result() {
            if let Some(index) = self.view.search_hit_index() {
                self.remember_search_cursor(index);
            } else {
                self.remember_search_cursor(0);
            }
        } else {
            self.view.restore_file_view_focus();
        }
    }

    fn sync_filter_mark(&self) {
        self.shortcuts.set_query_mark(
            &self.view.hidden_filter_query(),
            self.view.force_recursive_search(),
        );
    }

    fn recursive_hits_showing(&self) -> bool {
        live_recursive_search(self.view)
    }

    fn restore_applied_filter(&self) {
        self.minimal.borrow_mut().clear_search_nav();
        super::restore_applied_hidden_filter(self.view, self.shortcuts, self.minimal);
    }

    fn cancel_chord_ui(&self) {
        self.minimal.borrow_mut().cancel_chord();
        self.sidebar.state.clear_minimal_chord_hints();
        self.shortcuts.clear_chord_mark();
    }

    fn arm_chord(&self, kind: MinimalChord) {
        let slots = if kind == MinimalChord::Action {
            self.matching_action_slots()
        } else {
            Vec::new()
        };
        {
            let mut state = self.minimal.borrow_mut();
            state.enter_chord(kind);
            state.set_action_slots(slots);
        }
        if kind == MinimalChord::Go {
            self.sidebar.state.show_minimal_chord_hints();
        }
        if kind == MinimalChord::Action {
            let hints = self.minimal.borrow().action_slot_hints();
            self.shortcuts.set_chord_mark_named(kind, &hints);
        } else {
            self.shortcuts.set_chord_mark(kind);
        }
    }

    fn end_chord_ui(&self) {
        self.sidebar.state.clear_minimal_chord_hints();
        self.shortcuts.clear_chord_mark();
    }

    fn finish_chord(
        &self,
        browser: &Rc<Browser>,
        event: &KeyEvent,
        searching: bool,
        kind: MinimalChord,
    ) -> Propagation {
        // Holding Shift to reverse a sort (or any other modifier key) is not a
        // second chord key; cancelling here would flash `Unknown chord` before
        // the letter arrives.
        if modifier_key(event.key) {
            return Propagation::Stop;
        }
        match kind {
            MinimalChord::Go => self.finish_go_chord(browser, event, searching),
            MinimalChord::Copy => self.finish_copy_chord(event),
            MinimalChord::Sort => self.finish_sort_chord(browser, event),
            MinimalChord::Action => self.finish_action_chord(browser, event),
        }
    }

    fn finish_copy_chord(&self, event: &KeyEvent) -> Propagation {
        if !self.plain_chord_key(event) {
            return Propagation::Stop;
        }
        let origin = self.minimal.borrow().chord_from_visual();
        match event.key {
            Key::c => {
                self.minimal.borrow_mut().finish_chord(origin);
                self.end_chord_ui();
                self.select_search_cursor_if_fill_empty();
                self.view.copy_path();
            }
            Key::n => {
                self.minimal.borrow_mut().finish_chord(origin);
                self.end_chord_ui();
                self.select_search_cursor_if_fill_empty();
                self.view.copy_name();
            }
            _ => self.cancel_unknown_chord(),
        }
        Propagation::Stop
    }

    fn finish_sort_chord(&self, browser: &Rc<Browser>, event: &KeyEvent) -> Propagation {
        if !self.plain_chord_key(event) {
            return Propagation::Stop;
        }
        let origin = self.minimal.borrow().chord_from_visual();
        let Some((key, direction)) = sort_chord(event.key, event.shift()) else {
            self.cancel_unknown_chord();
            return Propagation::Stop;
        };
        self.minimal.borrow_mut().finish_chord(origin);
        self.end_chord_ui();
        if let Some(depth) = browser.active_depth() {
            browser.set_sort(depth, key, direction);
        }
        Propagation::Stop
    }

    fn finish_action_chord(&self, browser: &Rc<Browser>, event: &KeyEvent) -> Propagation {
        if !self.plain_chord_key(event) {
            return Propagation::Stop;
        }
        let origin = self.minimal.borrow().chord_from_visual();
        let Some(key) = action_chord_key(event.key) else {
            self.cancel_unknown_chord();
            return Propagation::Stop;
        };
        let Some(handle) = self.minimal.borrow().action_slot(key) else {
            self.minimal.borrow_mut().cancel_chord();
            self.end_chord_ui();
            self.shortcuts.flash(&format!("No action {key}"));
            tracing::debug!(%key, "minimal action chord has no slot");
            return Propagation::Stop;
        };
        self.minimal.borrow_mut().finish_chord(origin);
        self.end_chord_ui();
        self.run_listing_action(browser, handle);
        Propagation::Stop
    }

    fn matching_action_slots(&self) -> Vec<(char, Rc<ActionHandle>)> {
        let Some(inputs) = crate::ui::actions::inputs_for_entries(&self.action_chord_target())
        else {
            return Vec::new();
        };
        crate::ui::actions::chord_slots(&crate::ui::actions::shared().catalog().matches(&inputs))
    }

    fn action_chord_target(&self) -> Vec<FileEntry> {
        if let Some(entries) = self.view.selected_search_results() {
            if !entries.is_empty() {
                return entries;
            }
            return self.focused_search_result().into_iter().collect();
        }
        let browser = self.view.browser();
        let selected = browser.selected_entries();
        if !selected.is_empty() {
            return selected;
        }
        browser.focused_entry().into_iter().collect()
    }

    fn run_listing_action(&self, browser: &Rc<Browser>, handle: Rc<ActionHandle>) {
        let entries = self.action_chord_target();
        let Some(paths) = crate::ui::actions::native_paths(&entries) else {
            return;
        };
        let Some(parent) = browser
            .active_location()
            .and_then(|location| location.native_path().map(PathBuf::from))
        else {
            return;
        };
        crate::ui::actions::run_action(
            &self.view.overlay(),
            handle,
            paths,
            parent,
            InvocationSource::Selection,
        );
    }

    fn plain_chord_key(&self, event: &KeyEvent) -> bool {
        if event.key == Key::Escape
            && event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            self.minimal.borrow_mut().cancel_chord();
            self.end_chord_ui();
            return false;
        }
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
            self.minimal.borrow_mut().cancel_chord();
            self.end_chord_ui();
            tracing::debug!("minimal chord cancelled by modified key");
            return false;
        }
        true
    }

    fn cancel_unknown_chord(&self) {
        self.minimal.borrow_mut().cancel_chord();
        self.end_chord_ui();
        self.shortcuts.flash("Unknown chord");
        tracing::debug!("minimal chord cancelled by unmatched key");
    }

    fn finish_go_chord(
        &self,
        browser: &Rc<Browser>,
        event: &KeyEvent,
        searching: bool,
    ) -> Propagation {
        if event.key == Key::Escape
            && event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            self.minimal.borrow_mut().cancel_chord();
            self.end_chord_ui();
            return Propagation::Stop;
        }
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
            self.minimal.borrow_mut().cancel_chord();
            self.end_chord_ui();
            tracing::debug!("minimal chord cancelled by modified key");
            return Propagation::Stop;
        }
        let origin = self.minimal.borrow().chord_from_visual();
        match event.key {
            Key::g => {
                self.minimal.borrow_mut().finish_chord(origin);
                self.end_chord_ui();
                if self.preview_owns_keys() {
                    self.preview.scroll_to_edge(-1);
                } else {
                    self.minimal_jump(browser, origin, -1, searching);
                }
            }
            Key::Home => {
                if origin.is_none() {
                    self.minimal.borrow_mut().cancel_chord();
                    self.end_chord_ui();
                    self.shortcuts.flash("Unknown chord");
                    return Propagation::Stop;
                }
                self.minimal.borrow_mut().finish_chord(origin);
                self.end_chord_ui();
                self.minimal_jump(browser, origin, -1, searching);
            }
            Key::End | Key::G => {
                if origin.is_none() {
                    self.minimal.borrow_mut().cancel_chord();
                    self.end_chord_ui();
                    self.shortcuts.flash("Unknown chord");
                    return Propagation::Stop;
                }
                self.minimal.borrow_mut().finish_chord(origin);
                self.end_chord_ui();
                self.minimal_jump(browser, origin, 1, searching);
            }
            Key::h => {
                self.go_place(browser, Location::local(home_directory()));
            }
            Key::d => {
                self.go_special_dir(browser, glib::UserDirectory::Downloads, "Downloads");
            }
            Key::c => {
                self.go_place(browser, Location::local(glib::user_config_dir()));
            }
            Key::t => {
                self.go_place(browser, Location::uri("trash:///"));
            }
            Key::n => {
                self.go_place(browser, Location::uri("network:///"));
            }
            Key::r => {
                self.go_place(browser, Location::uri("recent:///"));
            }
            Key::k => {
                self.go_special_dir(browser, glib::UserDirectory::Documents, "Documents");
            }
            Key::p => {
                self.go_special_dir(browser, glib::UserDirectory::Pictures, "Pictures");
            }
            Key::v => {
                self.go_special_dir(browser, glib::UserDirectory::Videos, "Videos");
            }
            Key::space => {
                self.minimal.borrow_mut().finish_chord(None);
                self.end_chord_ui();
                self.leave_directory();
                self.open_prompt(browser, MinimalPrompt::Goto, "");
            }
            _ if chord_digit(event.key).is_some() => {
                let digit = chord_digit(event.key).expect("chord digit");
                self.minimal.borrow_mut().finish_chord(None);
                self.end_chord_ui();
                self.leave_directory();
                self.navigate_pin(browser, digit);
            }
            _ => {
                self.minimal.borrow_mut().cancel_chord();
                self.end_chord_ui();
                self.shortcuts.flash("Unknown chord");
                tracing::debug!("minimal chord cancelled by unmatched key");
            }
        }
        Propagation::Stop
    }

    fn go_place(&self, browser: &Rc<Browser>, location: Location) {
        self.minimal.borrow_mut().finish_chord(None);
        self.end_chord_ui();
        self.leave_directory();
        browser.navigate(location);
    }

    fn go_special_dir(&self, browser: &Rc<Browser>, directory: glib::UserDirectory, name: &str) {
        self.minimal.borrow_mut().finish_chord(None);
        self.end_chord_ui();
        self.leave_directory();
        if let Some(path) = glib::user_special_dir(directory) {
            browser.navigate(Location::local(path));
        } else {
            self.shortcuts.flash(&format!("No {name} folder"));
            tracing::debug!(name, "minimal chord has no special folder");
        }
    }

    fn navigate_pin(&self, browser: &Rc<Browser>, digit: usize) {
        // Numbered 1-based in visible PINNED order, after the same filters as
        // the sidebar itself: standard-place locations never take a digit,
        // and remotes are skipped for local-only sidebars.
        let visible = self.sidebar.state.visible_pinned_locations();
        if let Some(location) = visible.get(digit.saturating_sub(1)).cloned() {
            browser.navigate(location);
        } else {
            self.shortcuts.flash(&format!("No pin {digit}"));
            tracing::debug!(digit, "minimal chord has no visible pin");
        }
    }

    /// Unmatched keys: swallow printables and function keys that would
    /// otherwise type-to-search or trigger default bindings; let unmatched
    /// Ctrl/Alt/Super chords, focus traversal, and modifier keys through.
    fn minimal_fallback(&self, event: &KeyEvent) -> Propagation {
        if event
            .modifiers
            .intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            return Propagation::Proceed;
        }
        match event.key {
            Key::Tab | Key::ISO_Left_Tab => Propagation::Proceed,
            key if modifier_key(key) => Propagation::Proceed,
            _ => Propagation::Stop,
        }
    }
}

fn goto_listing_folders(browser: &Browser) -> Vec<String> {
    let Some(depth) = browser.active_depth() else {
        return Vec::new();
    };
    let Some(count) = browser.column_snapshot(depth).map(|column| column.count) else {
        return Vec::new();
    };
    browser
        .with_entries(depth, 0..count, |entries| {
            entries
                .iter()
                .filter(|entry| entry.is_directory())
                .map(|entry| entry.display_name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn modifier_key(key: Key) -> bool {
    matches!(
        key,
        Key::Shift_L
            | Key::Shift_R
            | Key::Control_L
            | Key::Control_R
            | Key::Alt_L
            | Key::Alt_R
            | Key::Super_L
            | Key::Super_R
            | Key::Meta_L
            | Key::Meta_R
            | Key::Caps_Lock
            | Key::Num_Lock
            | Key::Scroll_Lock
    )
}

fn sort_chord(key: Key, shift: bool) -> Option<(SortKey, SortDirection)> {
    let sort_key = match key {
        Key::a | Key::A => SortKey::Name,
        Key::m | Key::M => SortKey::Modified,
        Key::s | Key::S => SortKey::Size,
        Key::e | Key::E => SortKey::Type,
        _ => return None,
    };
    let descending = shift || matches!(key, Key::A | Key::M | Key::S | Key::E);
    Some((
        sort_key,
        if descending {
            SortDirection::Descending
        } else {
            SortDirection::Ascending
        },
    ))
}

fn history_candidates(kind: MinimalPrompt, query: &str) -> Vec<crate::services::SearchItem> {
    use crate::services::{NavigationHistory, fold_for_search};
    let history = NavigationHistory::shared();
    match kind {
        MinimalPrompt::HistoryFuzzy => history.search(query),
        MinimalPrompt::HistoryRecent => {
            let folded = fold_for_search(query.trim());
            history
                .recent()
                .into_iter()
                .filter(|item| {
                    folded.is_empty()
                        || fold_for_search(&item.path.to_string_lossy()).contains(&folded)
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

pub(super) fn kept_filter_fill(
    view: &crate::ui::browser::BrowserView,
    browser: &Browser,
) -> Vec<Location> {
    view.selected_search_results()
        .filter(|entries| !entries.is_empty())
        .unwrap_or_else(|| browser.selected_entries())
        .into_iter()
        .map(|entry| entry.location)
        .collect()
}

fn apply_kept_listing_fill(
    view: &crate::ui::browser::BrowserView,
    browser: &Browser,
    minimal: &Rc<RefCell<MinimalState>>,
    keep: &[Location],
    mark_explicit: bool,
) {
    if keep.is_empty() || view.selected_search_results().is_some() {
        return;
    }
    browser.apply_filled_locations(keep);
    if mark_explicit && !browser.selected_entries().is_empty() {
        minimal.borrow_mut().set_explicit_fill(true);
    }
}

pub(super) fn restore_fill_after_hidden_filter(
    view: crate::ui::browser::BrowserView,
    browser: Rc<Browser>,
    minimal: Rc<RefCell<MinimalState>>,
    keep: Vec<Location>,
    mark_explicit: bool,
) {
    if keep.is_empty() {
        return;
    }
    let generation = minimal.borrow_mut().start_fill_restore();
    if view.selected_search_results().is_none() {
        apply_kept_listing_fill(&view, &browser, &minimal, &keep, mark_explicit);
        return;
    }
    glib::timeout_add_local_once(crate::ui::browser::FILTER_DEBOUNCE_DELAY, move || {
        if !minimal.borrow().fill_restore_is(generation) {
            return;
        }
        glib::idle_add_local_once(move || {
            if !minimal.borrow().fill_restore_is(generation) {
                return;
            }
            apply_kept_listing_fill(&view, &browser, &minimal, &keep, mark_explicit);
        });
    });
}

/// Drops List/Columns preview-key ownership. No-op when the listing already owns the keys.
pub(super) fn release_preview_keys(
    minimal: &RefCell<MinimalState>,
    preview: &crate::ui::preview::PreviewDrawer,
    view: &crate::ui::browser::BrowserView,
) {
    if !minimal.borrow().preview_owns_keys() {
        return;
    }
    minimal.borrow_mut().set_preview_owns_keys(false);
    preview.set_owns_keys_chrome(false);
    view.set_column_header_focus(true);
    if view.selected_search_results().is_some() {
        let index = minimal
            .borrow()
            .search_cursor()
            .or_else(|| view.search_hit_index());
        if let Some(index) = index {
            view.focus_search_hit(index, false);
        } else {
            let _ = view.focus_first_search_result();
        }
        return;
    }
    view.browser().focus_active();
    view.restore_file_view_focus();
}

/// True while `f` is only displaying a live recursive `s`, not a pane filter.
pub(super) fn live_recursive_search(view: &crate::ui::browser::BrowserView) -> bool {
    view.force_recursive_search() && view.selected_search_results().is_some()
}

fn icons_spatial_arrow(key: Key) -> Option<Key> {
    match key {
        Key::h | Key::Left | Key::KP_Left => Some(Key::Left),
        Key::l | Key::Right | Key::KP_Right => Some(Key::Right),
        Key::k | Key::Up | Key::KP_Up => Some(Key::Up),
        Key::j | Key::Down | Key::KP_Down => Some(Key::Down),
        _ => None,
    }
}

fn chord_digit(key: Key) -> Option<usize> {
    match key {
        Key::_1 | Key::KP_1 => Some(1),
        Key::_2 | Key::KP_2 => Some(2),
        Key::_3 | Key::KP_3 => Some(3),
        Key::_4 | Key::KP_4 => Some(4),
        Key::_5 | Key::KP_5 => Some(5),
        Key::_6 | Key::KP_6 => Some(6),
        Key::_7 | Key::KP_7 => Some(7),
        Key::_8 | Key::KP_8 => Some(8),
        Key::_9 | Key::KP_9 => Some(9),
        _ => None,
    }
}

fn action_chord_key(key: Key) -> Option<char> {
    match key {
        Key::_1 | Key::KP_1 => Some('1'),
        Key::_2 | Key::KP_2 => Some('2'),
        Key::_3 | Key::KP_3 => Some('3'),
        Key::_4 | Key::KP_4 => Some('4'),
        Key::_5 | Key::KP_5 => Some('5'),
        Key::_6 | Key::KP_6 => Some('6'),
        Key::_7 | Key::KP_7 => Some('7'),
        Key::_8 | Key::KP_8 => Some('8'),
        Key::_9 | Key::KP_9 => Some('9'),
        Key::_0 | Key::KP_0 => Some('0'),
        _ => None,
    }
}

// SPDX-License-Identifier: MIT

//! Preview motion must not move or fill the listing behind the drawer.

use std::rc::Rc;

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
};

use super::{Dispatcher, KeyResult, command_modifiers};
use crate::{
    app::Browser,
    ui::{
        preview::{DocumentScroll, PreviewSurface, preview_target},
        tenxer_mode::Chord,
    },
};

impl Dispatcher {
    fn preview_focus(&self) -> Option<gtk::Widget> {
        gtk::prelude::RootExt::focus(&self.window)
            .filter(|focused| self.preview.owns_focus(Some(focused)))
    }

    /// Read-only source text inside the drawer takes preview keys, not typing.
    pub(super) fn preview_document_focused(&self) -> bool {
        self.preview_focus()
            .is_some_and(|focused| self.preview.surface(&focused) != PreviewSurface::Text)
    }

    /// Text fields inside the drawer keep typed text and editing shortcuts,
    /// including characters the footer would otherwise claim.
    pub(super) fn tenxer_preview_text(
        &self,
        browser: &Browser,
        key: Key,
        modifiers: Modifiers,
    ) -> KeyResult {
        if !self.type_to_search.preferences.tenxer_mode()
            || key == Key::F1
            || crate::ui::tenxer_mode::is_toggle_shortcut(key, modifiers)
        {
            return None;
        }
        let focused = self.preview_focus()?;
        if self.preview.surface(&focused) != PreviewSurface::Text {
            return None;
        }
        Some(
            self.leave_or_close_preview(browser, key, modifiers)
                .unwrap_or(Propagation::Proceed),
        )
    }

    pub(super) fn tenxer_preview(
        &self,
        browser: &Browser,
        key: Key,
        modifiers: Modifiers,
    ) -> KeyResult {
        let focused = self.preview_focus()?;
        if let Some(result) = self.leave_or_close_preview(browser, key, modifiers) {
            return Some(result);
        }
        let mods = command_modifiers(modifiers);
        // `i` toggled the preview open from the listing; from inside it, it closes.
        if key == Key::i && mods.is_empty() {
            self.close_preview(browser);
            return Some(Propagation::Stop);
        }
        let surface = self.preview.surface(&focused);
        if !mods.intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
            if matches!(key, Key::J | Key::K) {
                if surface != PreviewSurface::Media {
                    self.scroll_open_preview(key);
                }
                return Some(Propagation::Stop);
            }
            if matches!(key, Key::Tab) && mods.is_empty() {
                return Some(Propagation::Proceed);
            }
        }
        if key == Key::g
            && mods.is_empty()
            && matches!(surface, PreviewSurface::Document | PreviewSurface::Archive)
        {
            self.shortcuts.arm_chord(Chord::PreviewTop);
            return Some(Propagation::Stop);
        }
        let handled = match surface {
            PreviewSurface::Document => self.preview_document_key(key, mods),
            PreviewSurface::Archive => self.preview_archive_key(browser, key, mods),
            PreviewSurface::Media => self.preview_media_key(browser, key, mods),
            PreviewSurface::Control | PreviewSurface::Text => {
                // A focused button or slider keeps GTK's Enter.
                let listing = surface == PreviewSurface::Control
                    && !matches!(key, Key::Return | Key::KP_Enter)
                    && reaches_listing(key, mods);
                if !listing && !mods.intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK) {
                    return Some(Propagation::Proceed);
                }
                false
            }
        };
        if handled {
            return Some(Propagation::Stop);
        }
        if passes_through_preview(key, mods) {
            return None;
        }
        if mods == Modifiers::CONTROL_MASK
            && matches!(key, Key::c | Key::a)
            && surface != PreviewSurface::Media
        {
            // The focused document widget selects all or copies its own text.
            return Some(Propagation::Proceed);
        }
        if reaches_listing(key, mods) {
            self.return_from_preview(browser);
            return None;
        }
        Some(Propagation::Stop)
    }

    /// **Esc** closes the drawer; **Shift+Tab** returns keys to the listing
    /// with the drawer still open. Both work on every preview surface.
    fn leave_or_close_preview(
        &self,
        browser: &Browser,
        key: Key,
        modifiers: Modifiers,
    ) -> KeyResult {
        let mods = command_modifiers(modifiers);
        if key == Key::Escape && mods.is_empty() {
            self.close_preview(browser);
            return Some(Propagation::Stop);
        }
        if key == Key::ISO_Left_Tab || (key == Key::Tab && mods == Modifiers::SHIFT_MASK) {
            self.return_from_preview(browser);
            return Some(Propagation::Stop);
        }
        None
    }

    pub(super) fn close_preview(&self, browser: &Browser) {
        self.preview.close();
        self.return_from_preview(browser);
    }

    /// Results replacing the listing take the keys back on their own cursor.
    pub(super) fn return_from_preview(&self, browser: &Browser) {
        if self.view.focus_results_cursor() {
            return;
        }
        self.view.keyboard_navigation();
        browser.focus_active();
    }

    fn preview_document_key(&self, key: Key, mods: Modifiers) -> bool {
        let motion = if mods == Modifiers::CONTROL_MASK {
            match key {
                Key::u | Key::U => DocumentScroll::HalfPage(-1),
                Key::d | Key::D => DocumentScroll::HalfPage(1),
                Key::b | Key::B | Key::Page_Up | Key::KP_Page_Up => DocumentScroll::Page(-1),
                Key::f | Key::F | Key::Page_Down | Key::KP_Page_Down => DocumentScroll::Page(1),
                Key::Up | Key::KP_Up => DocumentScroll::Start,
                Key::Down | Key::KP_Down => DocumentScroll::End,
                _ => return false,
            }
        } else if mods.is_empty() || mods == Modifiers::SHIFT_MASK {
            match key {
                Key::h | Key::Left | Key::KP_Left if mods.is_empty() => {
                    self.return_from_preview(&self.view.browser());
                    return true;
                }
                Key::j | Key::Down | Key::KP_Down => DocumentScroll::Line(1),
                Key::k | Key::Up | Key::KP_Up => DocumentScroll::Line(-1),
                Key::Page_Up | Key::KP_Page_Up => DocumentScroll::Page(-1),
                Key::Page_Down | Key::KP_Page_Down => DocumentScroll::Page(1),
                Key::Home | Key::KP_Home => DocumentScroll::Start,
                Key::End | Key::KP_End | Key::G => DocumentScroll::End,
                _ => return false,
            }
        } else {
            return false;
        };
        self.preview.scroll_document(motion);
        true
    }

    fn preview_archive_key(&self, browser: &Browser, key: Key, mods: Modifiers) -> bool {
        if !mods.is_empty() && !(mods == Modifiers::SHIFT_MASK && key == Key::G) {
            return false;
        }
        match key {
            Key::j | Key::Down | Key::KP_Down => {
                self.preview.archive_key(Key::Down);
            }
            Key::k | Key::Up | Key::KP_Up => {
                self.preview.archive_key(Key::Up);
            }
            Key::l | Key::Right | Key::KP_Right | Key::Return | Key::KP_Enter => {
                self.preview.archive_key(Key::Right);
            }
            Key::h | Key::Left | Key::KP_Left => {
                if self.preview.archive_at_root() {
                    self.return_from_preview(browser);
                } else {
                    self.preview.archive_key(Key::Left);
                }
            }
            Key::Home | Key::KP_Home => {
                self.preview.archive_edge(false);
            }
            Key::End | Key::KP_End | Key::G => {
                self.preview.archive_edge(true);
            }
            _ => return false,
        }
        true
    }

    fn preview_media_key(&self, browser: &Browser, key: Key, mods: Modifiers) -> bool {
        // `<` and `>` need Shift on most layouts.
        let track_key = matches!(key, Key::less | Key::greater);
        if !(mods.is_empty() || track_key && mods == Modifiers::SHIFT_MASK) {
            return false;
        }
        let media_key = match key {
            Key::h => {
                self.return_from_preview(browser);
                return true;
            }
            Key::KP_Left => Key::Left,
            Key::KP_Right => Key::Right,
            Key::KP_Up => Key::Up,
            Key::KP_Down => Key::Down,
            other => other,
        };
        self.preview.media_key(media_key);
        matches!(
            media_key,
            Key::space
                | Key::Left
                | Key::Right
                | Key::Up
                | Key::Down
                | Key::m
                | Key::less
                | Key::greater
        )
    }

    /// **g g** in an owning document or archive preview.
    pub(super) fn preview_to_top(&self) -> bool {
        let Some(focused) = self.preview_focus() else {
            return false;
        };
        match self.preview.surface(&focused) {
            PreviewSurface::Document => {
                self.preview.scroll_document(DocumentScroll::Start);
            }
            PreviewSurface::Archive => {
                self.preview.archive_edge(false);
            }
            _ => return false,
        }
        true
    }

    /// **J** / **K** scroll whatever the drawer shows without moving focus.
    pub(super) fn scroll_open_preview(&self, key: Key) {
        if !self.preview.is_open() {
            return;
        }
        self.preview
            .scroll_document(DocumentScroll::Line(if key == Key::J { 1 } else { -1 }));
    }

    /// **i** on a file opens or closes the preview and leaves focus in the
    /// listing. Directories keep their Miller column or folder peek.
    pub(super) fn toggle_file_preview(&self, browser: &Rc<Browser>) -> bool {
        let entry = if self.view.selected_search_results().is_some() {
            self.view.selected_search_result()
        } else {
            browser.focused_entry()
        };
        if entry.as_ref().is_none_or(|entry| entry.is_directory()) {
            return false;
        }
        self.view.keyboard_navigation();
        if self.preview.is_enabled() {
            self.preview.close();
        } else if let Some(target) = preview_target(entry) {
            self.preview.show(target, browser.active_depth());
        } else {
            self.shortcuts.show_feedback("Nothing to preview");
        }
        true
    }

    /// List and Columns **l** / **→** on a file: open the drawer if needed and
    /// hand it the keys. Never launches the file.
    pub(super) fn enter_preview(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        let search = self.view.selected_search_results().is_some();
        let entry = if search {
            self.view.selected_search_result()
        } else {
            browser.focused_entry()
        };
        if let Some(directory) = entry.as_ref().filter(|entry| entry.is_directory()) {
            if search {
                browser.navigate(directory.location.clone());
            } else {
                self.view.activate_focused();
            }
            return;
        }
        let Some(target) = preview_target(entry) else {
            self.shortcuts.show_feedback("Nothing to preview");
            return;
        };
        self.preview.show(target, browser.active_depth());
        self.preview.take_keyboard();
    }
}

/// Window-level commands that neither read nor change the listing.
fn passes_through_preview(key: Key, mods: Modifiers) -> bool {
    let plain = mods.is_empty();
    let control = mods == Modifiers::CONTROL_MASK;
    let control_shift = mods == Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK;
    match key {
        Key::F1 | Key::F5 if plain => true,
        Key::k | Key::K | Key::l | Key::comma | Key::h | Key::H | Key::period | Key::n | Key::N
            if control =>
        {
            true
        }
        Key::_1 | Key::_2 | Key::_3 if control => true,
        Key::plus | Key::minus | Key::equal | Key::_0 | Key::KP_Add | Key::KP_Subtract
            if control =>
        {
            true
        }
        Key::b | Key::B | Key::m | Key::M if control_shift => true,
        Key::z | Key::Z => control || control_shift,
        Key::y | Key::Y if control => true,
        // Media controls reach the drawer's player from the window handler.
        Key::space | Key::Left | Key::Right | Key::Up | Key::Down | Key::m | Key::M => {
            mods == Modifiers::CONTROL_MASK | Modifiers::ALT_MASK
        }
        _ => false,
    }
}

fn reaches_listing(key: Key, mods: Modifiers) -> bool {
    let plain = mods.is_empty();
    let shift = mods == Modifiers::SHIFT_MASK;
    let control = mods == Modifiers::CONTROL_MASK;
    let alt = mods == Modifiers::ALT_MASK;
    match key {
        Key::g
        | Key::z
        | Key::f
        | Key::s
        | Key::n
        | Key::slash
        | Key::KP_Divide
        | Key::BackSpace
            if plain =>
        {
            true
        }
        Key::H | Key::L | Key::Z | Key::N | Key::question if shift => true,
        Key::Left | Key::KP_Left | Key::Right | Key::KP_Right | Key::Up | Key::KP_Up if alt => true,
        Key::o
        | Key::y
        | Key::x
        | Key::p
        | Key::d
        | Key::a
        | Key::c
        | Key::r
        | Key::F2
        | Key::Menu
            if plain =>
        {
            true
        }
        Key::Y | Key::X | Key::P | Key::D | Key::O | Key::M | Key::C | Key::R | Key::F10
            if shift =>
        {
            true
        }
        Key::comma | Key::semicolon | Key::period | Key::Delete | Key::KP_Delete => plain || shift,
        Key::Return | Key::KP_Enter => plain || alt,
        Key::c | Key::x | Key::v if control => true,
        Key::n | Key::N => mods == Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK,
        _ => false,
    }
}

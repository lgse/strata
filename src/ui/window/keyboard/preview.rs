// SPDX-License-Identifier: MIT

//! Preview motion must not move or fill the listing behind the drawer.

use std::rc::Rc;

use gtk::{gdk::ModifierType as Modifiers, glib::Propagation};

use super::{Dispatcher, KeyResult};
use crate::{
    app::Browser,
    ui::{
        keys::{Binding, KeyPress, SHORTCUT_MODIFIERS, bindings::*},
        preview::{ArchiveMove, DocumentScroll, PreviewSurface, media_key_command, preview_target},
        tenxer_mode::Chord,
    },
};

const CONTROL: Modifiers = Modifiers::CONTROL_MASK;
const SHIFT: Modifiers = Modifiers::SHIFT_MASK;

/// GTK's own text selection keys, which a focused document keeps.
const DOCUMENT_SELECTION: [Binding; 8] = [
    EXTEND_UP.ignoring(CONTROL),
    EXTEND_DOWN.ignoring(CONTROL),
    EXTEND_LEFT.ignoring(CONTROL),
    EXTEND_RIGHT.ignoring(CONTROL),
    EXTEND_TO_FIRST.ignoring(CONTROL),
    EXTEND_TO_LAST.ignoring(CONTROL),
    DOCUMENT_START,
    DOCUMENT_END,
];

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
    pub(super) fn tenxer_preview_text(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        if !self.type_to_search.preferences.tenxer_mode()
            || press.is_any(&[REFERENCE.ignoring(SHORTCUT_MODIFIERS), TENXER_MODE])
        {
            return None;
        }
        let focused = self.preview_focus()?;
        if self.preview.surface(&focused) != PreviewSurface::Text {
            return None;
        }
        Some(
            self.leave_or_close_preview(browser, press)
                .unwrap_or(Propagation::Proceed),
        )
    }

    pub(super) fn tenxer_preview(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        let focused = self.preview_focus()?;
        if let Some(result) = self.leave_or_close_preview(browser, press) {
            return Some(result);
        }
        // `i` toggled the preview open from the listing; from inside it, it closes.
        if press.is(PEEK) {
            self.close_preview(browser);
            return Some(Propagation::Stop);
        }
        let surface = self.preview.surface(&focused);
        if surface == PreviewSurface::Document && press.is_any(&DOCUMENT_SELECTION) {
            return Some(Propagation::Proceed);
        }
        if press.is_any(&[SCROLL_PREVIEW_DOWN, SCROLL_PREVIEW_UP]) {
            if surface != PreviewSurface::Media {
                self.scroll_open_preview(if press.is(SCROLL_PREVIEW_DOWN) { 1 } else { -1 });
            }
            return Some(Propagation::Stop);
        }
        if press.is(FOCUS_NEXT) {
            return Some(Propagation::Proceed);
        }
        if press.is(GO) && matches!(surface, PreviewSurface::Document | PreviewSurface::Archive) {
            self.shortcuts.arm_chord(Chord::PreviewTop);
            return Some(Propagation::Stop);
        }
        let handled = match surface {
            PreviewSurface::Document => self.preview_document_key(press),
            PreviewSurface::Archive => self.preview_archive_key(browser, press),
            PreviewSurface::Media => self.preview_media_key(browser, press),
            PreviewSurface::Control | PreviewSurface::Text => {
                // A focused button or slider keeps GTK's Enter.
                let listing = surface == PreviewSurface::Control
                    && !press.is(ENTER_ANY)
                    && reaches_listing(press);
                if !listing && !press.modifiers().intersects(CONTROL | Modifiers::ALT_MASK) {
                    return Some(Propagation::Proceed);
                }
                false
            }
        };
        if handled {
            return Some(Propagation::Stop);
        }
        if passes_through_preview(press) {
            return None;
        }
        if press.is_any(&[COPY, SELECT_ALL]) && surface != PreviewSurface::Media {
            // The focused document widget selects all or copies its own text.
            return Some(Propagation::Proceed);
        }
        if reaches_listing(press) {
            self.return_from_preview(browser);
            return None;
        }
        Some(Propagation::Stop)
    }

    /// **Esc** closes the drawer; **Shift+Tab** returns keys to the listing
    /// with the drawer still open. Both work on every preview surface.
    fn leave_or_close_preview(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        if press.is(ESCAPE) {
            self.close_preview(browser);
            return Some(Propagation::Stop);
        }
        let held_too = CONTROL
            .union(Modifiers::ALT_MASK)
            .union(Modifiers::SUPER_MASK);
        if press.is(FOCUS_PREVIOUS.ignoring(held_too)) {
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

    fn preview_document_key(&self, press: &KeyPress) -> bool {
        if press.is_any(&[VIM_LEFT, LEFT]) {
            self.return_from_preview(&self.view.browser());
            return true;
        }
        let Some((_, motion)) = [
            (HALF_PAGE_UP, DocumentScroll::HalfPage(-1)),
            (HALF_PAGE_DOWN, DocumentScroll::HalfPage(1)),
            (TENXER_PAGE_UP, DocumentScroll::Page(-1)),
            (TENXER_PAGE_UP_PAGE_UP, DocumentScroll::Page(-1)),
            (TENXER_PAGE_DOWN, DocumentScroll::Page(1)),
            (TENXER_PAGE_DOWN_PAGE_DOWN, DocumentScroll::Page(1)),
            (FIRST_ITEM_UP, DocumentScroll::Start),
            (LAST_ITEM_DOWN, DocumentScroll::End),
            (VIM_DOWN, DocumentScroll::Line(1)),
            (DOWN, DocumentScroll::Line(1)),
            (VIM_UP, DocumentScroll::Line(-1)),
            (UP, DocumentScroll::Line(-1)),
            (PAGE_UP.ignoring(SHIFT), DocumentScroll::Page(-1)),
            (PAGE_DOWN.ignoring(SHIFT), DocumentScroll::Page(1)),
            (FIRST_ITEM, DocumentScroll::Start),
            (LAST_ITEM, DocumentScroll::End),
            (TENXER_LAST_ITEM, DocumentScroll::End),
        ]
        .into_iter()
        .find(|(key, _)| press.is(*key)) else {
            return false;
        };
        self.preview.scroll_document(motion);
        true
    }

    fn preview_archive_key(&self, browser: &Browser, press: &KeyPress) -> bool {
        if press.is_any(&[VIM_DOWN, DOWN]) {
            self.preview.archive_key(ArchiveMove::Next);
        } else if press.is_any(&[VIM_UP, UP]) {
            self.preview.archive_key(ArchiveMove::Previous);
        } else if press.is_any(&[VIM_RIGHT, RIGHT, ENTER]) {
            self.preview.archive_key(ArchiveMove::Open);
        } else if press.is_any(&[VIM_LEFT, LEFT]) {
            if self.preview.archive_at_root() {
                self.return_from_preview(browser);
            } else {
                self.preview.archive_key(ArchiveMove::Parent);
            }
        } else if press.is(FIRST_ITEM) {
            self.preview.archive_edge(false);
        } else if press.is_any(&[LAST_ITEM, TENXER_LAST_ITEM]) {
            self.preview.archive_edge(true);
        } else {
            return false;
        }
        true
    }

    fn preview_media_key(&self, browser: &Browser, press: &KeyPress) -> bool {
        if press.is(VIM_LEFT) {
            self.return_from_preview(browser);
            return true;
        }
        let Some(command) = media_key_command(press) else {
            return false;
        };
        self.preview.media_key(command);
        true
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
    pub(super) fn scroll_open_preview(&self, lines: i32) {
        if !self.preview.is_open() {
            return;
        }
        self.preview.scroll_document(DocumentScroll::Line(lines));
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
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to preview"));
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
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to preview"));
            return;
        };
        self.preview.show(target, browser.active_depth());
        self.preview.take_keyboard();
    }
}

/// Window-level commands that neither read nor change the listing.
fn passes_through_preview(press: &KeyPress) -> bool {
    press.is_any(&[
        REFERENCE,
        REFRESH,
        SEARCH,
        LOCATION,
        SETTINGS,
        HIDDEN_FILES,
        HIDDEN_FILES_PERIOD,
        TENXER_SIDEBAR,
        VIEW_COLUMNS,
        VIEW_ICONS,
        VIEW_LIST,
        TEXT_LARGER,
        TEXT_LARGER_EQUAL,
        TEXT_SMALLER,
        TEXT_RESET,
        SIDEBAR_FOCUS,
        TENXER_MODE,
        UNDO,
        REDO,
        REDO_Y,
        // Media controls reach the drawer's player from the window handler.
        MEDIA_PLAY_PAUSE,
        MEDIA_SEEK_BACK,
        MEDIA_SEEK_FORWARD,
        MEDIA_VOLUME_UP,
        MEDIA_VOLUME_DOWN,
        MEDIA_MUTE,
    ])
}

fn reaches_listing(press: &KeyPress) -> bool {
    press.is_any(&[
        BOOKMARK_SET,
        BOOKMARK_GO,
        GO,
        JUMP_VISITED,
        FILTER_LISTING,
        SEARCH_SUBFOLDERS,
        REPEAT_FIND,
        FIND,
        BACKSPACE,
        TENXER_HISTORY_BACK,
        TENXER_HISTORY_FORWARD,
        JUMP_RECENT,
        REPEAT_FIND_REVERSED,
        FIND_BACKWARD,
        HISTORY_BACK,
        HISTORY_FORWARD,
        PARENT_FOLDER,
        OPEN,
        YANK,
        YANK_CUT,
        PASTE_KEEP_BOTH,
        TENXER_DELETE,
        CREATE,
        COPY_AS,
        TENXER_RENAME,
        RENAME,
        CONTEXT_MENU,
        UNYANK,
        UNYANK_X,
        PASTE_REPLACE,
        TENXER_DELETE_PERMANENTLY,
        OPEN_WITH,
        MOVE_TO,
        COPY_TO,
        RESTORE,
        CONTEXT_MENU_F10,
        SORT_BY,
        ACTIONS,
        TENXER_HIDDEN_FILES,
        DELETE,
        DELETE_PERMANENTLY,
        ENTER,
        PROPERTIES,
        COPY,
        CUT,
        PASTE,
        NEW_FOLDER,
    ])
}

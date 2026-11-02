// SPDX-License-Identifier: MIT

//! Keys that move between the file list and the preview drawer, and inside it.

use gtk::glib::Propagation;

use crate::{
    model::FileEntry,
    ui::{
        browser_modes::BrowserMode,
        keys::{KeyPress, bindings::*},
        preview::{ArchiveMove, PreviewSurface},
    },
};

use super::super::{Dispatcher, KeyEvent, KeyResult};

impl Dispatcher {
    pub(in crate::ui::window::keyboard) fn preview_navigation(
        &self,
        event: &KeyEvent,
    ) -> KeyResult {
        if event.text_has_focus() && !self.preview_document_has_focus(event) {
            return None;
        }
        if self.view.item_view_has_focus() {
            return self.preview_from_items(&event.press);
        }
        if !self.preview.owns_focus(event.focused.as_ref()) {
            return None;
        }
        if let Some(focused) = event.focused.as_ref()
            && let Some(result) = self.preview_feedback_keys(&event.press, focused)
        {
            return Some(result);
        }
        self.preview_surface_keys(event)
            .or_else(|| self.archive_list_keys(event))
    }

    fn preview_document_has_focus(&self, event: &KeyEvent) -> bool {
        event.focused.as_ref().is_some_and(|focused| {
            self.preview.owns_focus(Some(focused))
                && self.preview.surface(focused) == PreviewSurface::Document
        })
    }

    fn preview_from_items(&self, press: &KeyPress) -> KeyResult {
        if self.opens_archive_preview(press) {
            self.enter_preview(&self.view.browser());
            return Some(Propagation::Stop);
        }
        (press.is(RIGHT) && self.preview_takes_keyboard()).then_some(Propagation::Stop)
    }

    /// Columns opens a focused archive in the preview rather than as a folder.
    fn opens_archive_preview(&self, press: &KeyPress) -> bool {
        if self.view.view_mode() != BrowserMode::Columns || !opens_entry(press) {
            return false;
        }
        self.view
            .browser()
            .focused_entry()
            .is_some_and(|entry| is_previewable_archive(&entry))
    }

    fn preview_takes_keyboard(&self) -> bool {
        if !matches!(
            self.view.view_mode(),
            BrowserMode::List | BrowserMode::Columns
        ) || !self.preview.is_open()
        {
            return false;
        }
        let file_focused = self
            .view
            .browser()
            .focused_entry()
            .is_some_and(|entry| !entry.is_directory());
        file_focused && self.preview.take_keyboard()
    }

    fn preview_feedback_keys(&self, press: &KeyPress, focused: &gtk::Widget) -> KeyResult {
        if self.preview.feedback_action_focused(focused) {
            return self.feedback_action_keys(press);
        }
        let leaves_document = press.is_any(&[DOWN, RIGHT])
            && self.preview.surface(focused) == PreviewSurface::Document;
        (leaves_document && self.preview.focus_feedback_action()).then_some(Propagation::Stop)
    }

    fn feedback_action_keys(&self, press: &KeyPress) -> KeyResult {
        if press.is(LEFT) {
            self.return_from_preview(&self.view.browser());
            return Some(Propagation::Stop);
        }
        // Arrows would otherwise walk GTK focus out of the drawer.
        press
            .is_any(&[UP, DOWN, RIGHT])
            .then_some(Propagation::Stop)
    }

    fn preview_surface_keys(&self, event: &KeyEvent) -> KeyResult {
        let press = &event.press;
        if press.is(ENTER)
            && self.focused_surface_is(event, |surface| surface == PreviewSurface::Media)
        {
            self.activate_focused(&self.view.browser());
            return Some(Propagation::Stop);
        }
        if press.is(LEFT) && self.leaves_archive_root(event) {
            self.return_from_preview(&self.view.browser());
            return Some(Propagation::Stop);
        }
        None
    }

    fn leaves_archive_root(&self, event: &KeyEvent) -> bool {
        self.preview.archive_at_root()
            && self.focused_surface_is(event, |surface| {
                matches!(surface, PreviewSurface::Archive | PreviewSurface::Document)
            })
    }

    fn focused_surface_is(
        &self,
        event: &KeyEvent,
        matches: impl Fn(PreviewSurface) -> bool,
    ) -> bool {
        event
            .focused
            .as_ref()
            .is_some_and(|focused| matches(self.preview.surface(focused)))
    }

    fn archive_list_keys(&self, event: &KeyEvent) -> KeyResult {
        if !self.preview.archive_list_has_focus(event.focused.as_ref()) {
            return None;
        }
        let press = &event.press;
        if press.is(SPACE) {
            return self.preview.close_archive().then_some(Propagation::Stop);
        }
        let motion = [
            (UP, ArchiveMove::Previous),
            (DOWN, ArchiveMove::Next),
            (LEFT, ArchiveMove::Parent),
            (RIGHT, ArchiveMove::Open),
            (ENTER, ArchiveMove::Open),
        ]
        .into_iter()
        .find(|(key, _)| press.is(*key))?
        .1;
        self.preview
            .archive_key(motion)
            .then_some(Propagation::Stop)
    }
}

fn opens_entry(press: &KeyPress) -> bool {
    press.is(RIGHT) || press.is(ENTER) || press.is(SPACE)
}

fn is_previewable_archive(entry: &FileEntry) -> bool {
    !entry.is_directory()
        && entry.location.native_path().is_some()
        && crate::services::archive_preview_format(&entry.native_name).is_some()
}

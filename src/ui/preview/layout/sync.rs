// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use super::super::PreviewState;
use super::geometry::Geometry;

impl PreviewState {
    pub(in crate::ui::preview) fn sync_split(self: &Rc<Self>, split: &gtk::Paned) {
        let mut geometry = self.geometry(split);
        let reserved = self.reserves_column_space();
        if self.preview_present() {
            geometry = self.sync_sidebar_rail(split, geometry);
        }
        // Keep the slot even below the content threshold to prevent scroll clamping.
        let bare = reserved
            && (self.slot_is_empty() || !split.is_mapped() || !geometry.can_show_preview());
        if bare {
            self.sync_bare_slot(split, geometry);
            return;
        }
        self.slot.set_width_request(0);
        let showing = self.current.borrow().is_some();
        if !showing && !self.show_empty_preview(geometry) {
            return;
        }
        if !split.is_mapped() || !geometry.can_show_preview() {
            self.suspend_panel();
            return;
        }
        if self.animating.get() || self.sizing.resizing.get() {
            return;
        }
        let restored = self.dock_preview(split, geometry);
        self.resume_after_dock(restored);
    }

    /// Columns keeps its reserved slot without a preview inside it.
    fn sync_bare_slot(&self, split: &gtk::Paned, geometry: Geometry) {
        if self.sizing.resizing.get() {
            return;
        }
        if self.current.borrow().is_some() {
            self.suspend_panel();
        } else if self.revealer.reveals_child() {
            self.hide_panel();
        }
        let manual = self.sizing.manual_width.get();
        self.slot.set_visible(true);
        let position = geometry.empty_slot_position(manual);
        let slot_fills_free_space = position == geometry.occupied.saturating_add(geometry.trailing);
        // A stale minimum would over-allocate the slot for one shrinking frame.
        self.slot.set_width_request(if slot_fills_free_space {
            0
        } else {
            geometry.empty_slot_minimum(manual.is_some())
        });
        split.set_resize_start_child(!slot_fills_free_space);
        split.set_resize_end_child(slot_fills_free_space);
        split.set_position(position);
    }

    /// Shows the empty placeholder when one is reserved and fits; `false` stops the sync.
    fn show_empty_preview(&self, geometry: Geometry) -> bool {
        let reserves_empty_preview = self.reserves_empty_preview();
        if reserves_empty_preview && geometry.can_show_preview() {
            self.show_placeholder();
            return true;
        }
        if self.revealer.reveals_child() {
            self.hide_panel();
        }
        if !reserves_empty_preview {
            self.sizing.suspended.set(false);
        }
        false
    }

    /// Places the divider for the docked preview, reporting whether this ended a
    /// suspension.
    fn dock_preview(&self, split: &gtk::Paned, geometry: Geometry) -> bool {
        // GTK allocates after this tick; free-space previews must absorb window
        // growth instead of correcting the browser's divider on the next frame.
        let manual = self.sizing.manual_width.get();
        let position = geometry.position(manual);
        let preview_fills_free_space = geometry.columns && position == geometry.occupied;
        split.set_resize_start_child(!preview_fills_free_space);
        split.set_resize_end_child(preview_fills_free_space);
        let restored = self.sizing.suspended.replace(false);
        if restored || !self.revealer.reveals_child() {
            self.show_panel();
        }
        let minimum = if preview_fills_free_space {
            0
        } else {
            geometry.minimum_width(manual.is_some())
        };
        if self.pane.width_request() != minimum || split.position() != position {
            self.pane.set_width_request(minimum);
            split.set_position(position);
        }
        if let Some(browser) = self.sizing.browser() {
            browser.clear_preview_scroll_space();
        }
        restored
    }

    fn resume_after_dock(self: &Rc<Self>, restored: bool) {
        if self.sizing.reload_on_resume.replace(false) {
            self.reload_current();
        } else if restored && self.sizing.resume_media.replace(false) {
            self.resume_media();
        }
        if restored {
            self.resume_keyboard_claim();
        }
    }

    fn reload_current(self: &Rc<Self>) {
        let entry = self.current.borrow().clone();
        if let Some(entry) = entry {
            self.load(entry, 0);
        }
    }

    fn resume_media(&self) {
        let media = self.media.borrow().clone();
        if let Some(media) = media {
            media.play();
        }
    }
}

// SPDX-License-Identifier: MIT

use super::*;

impl PreviewState {
    pub(super) fn cancel_pending_show(&self) {
        if let Some(source) = self.pending_show.borrow_mut().take() {
            source.remove();
        }
    }

    pub(super) fn show_after_focus_change(self: &Rc<Self>, entry: FileEntry, depth: Option<usize>) {
        self.cancel_pending_show();
        self.retain_pending_playback(&entry);
        if self.current.borrow().as_ref() == Some(&entry) && self.current_request.get().is_some() {
            return;
        }
        // A suspended l stays pending until this file changes or the drawer closes.
        self.claim_on_resume.set(false);
        if !self.revealer.reveals_child() {
            self.show(entry, depth);
            return;
        }
        self.focus_archive_on_ready.set(false);
        self.current_request.set(None);
        self.load.borrow_mut().take();
        self.pdf_loads.borrow_mut().clear();
        self.raw_metadata_load.borrow_mut().take();
        self.raw_details.reset();
        // Keep the displayed target during debounce: split synchronization must
        // not mistake a replacement request for an empty, closed drawer.

        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(FOCUS_PREVIEW_DELAY, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.pending_show.borrow_mut().take();
            if state.is_enabled() {
                state.show(entry, depth);
            }
        });
        self.pending_show.replace(Some(source));
    }

    // Mirroring also emits focus for the child beyond the active depth. A folder that a
    // click is opening becomes the focused column, so lending to it would only flicker.
    pub(super) fn lend_slot_to_child(
        &self,
        browser: &Browser,
        depth: usize,
        entry: Option<&FileEntry>,
    ) {
        let opening = entry.is_some_and(|entry| {
            self.sizing
                .browser()
                .is_some_and(|view| view.pointer_opens(depth, &entry.location))
        });
        let child_pane = self.browsing_columns()
            && !opening
            && (entry.is_some_and(FileEntry::is_directory)
                || browser.active_depth().is_some_and(|active| depth > active));
        self.child_pane.set(child_pane);
    }

    pub(super) fn show(self: &Rc<Self>, entry: FileEntry, depth: Option<usize>) {
        let other_file = self
            .current
            .borrow()
            .as_ref()
            .is_none_or(|current| current.location != entry.location);
        if other_file {
            self.reset_find();
            self.model_view.set(Default::default());
            self.model.borrow_mut().take();
        }
        self.cancel_pending_show();
        self.retain_pending_playback(&entry);
        if self.handed_off.borrow().as_ref() != Some(&entry.location) {
            self.handed_off.take();
        }
        self.current_depth.set(depth);
        self.reserve_columns.set(true);
        self.dismissed.set(false);
        self.child_pane.set(false);
        self.set_enabled(true);
        let reopening = self.closing.replace(false);
        let was_open = (self.revealer.reveals_child() && !reopening) || self.sizing.is_suspended();
        let already_showing =
            self.current.borrow().as_ref() == Some(&entry) && self.current_request.get().is_some();
        let reload = !was_open || !already_showing;
        let split = self.split.borrow().clone();
        if let Some(split) = split.as_ref()
            && (!self.can_show_in(split) || self.sizing.is_suspended())
        {
            self.defer_show(split, entry, reload);
            return;
        }
        if !was_open {
            self.current.replace(Some(entry.clone()));
            self.open_drawer(split.as_ref());
        }
        if reload {
            self.load(entry, 0);
        }
    }

    /// Keeps `entry` as the target until the split has room to show it.
    fn defer_show(self: &Rc<Self>, split: &gtk::Paned, entry: FileEntry, reload: bool) {
        if reload {
            self.current_request.set(None);
            self.load.borrow_mut().take();
            self.cancel_loading();
            self.pdf_loads.borrow_mut().clear();
            self.clear_raw_details();
            self.clear_content();
            self.sizing.defer_load();
        }
        self.current.replace(Some(entry));
        self.sync_split(split);
    }

    fn open_drawer(self: &Rc<Self>, split: Option<&gtk::Paned>) {
        if let Some(split) = split {
            self.animate_reveal(split, true, |_| {});
        } else {
            self.show_panel();
        }
    }

    pub(super) fn stop(&self) {
        self.set_enabled(false);
        self.child_pane.set(false);
        self.clear_target();
        // Destroying a focused prompt does not always report a focus leave.
        self.content.set_focusable(false);
        self.set_keyboard_owner(false);
    }

    pub(super) fn close(self: &Rc<Self>) {
        self.media_checks.borrow_mut().stop();
        let tree_focused = self.archive_tree_focused();
        // The preview closes now, so input such as the next Escape already
        // reaches the files; only the drawer's slide out waits.
        let restore_focus = tree_focused || self.hide_intent();
        if self.is_enabled() {
            self.dismissed.set(true);
        }
        self.set_enabled(false);
        self.child_pane.set(false);
        self.content.set_focusable(false);
        self.set_keyboard_owner(false);
        if restore_focus && let Some(browser) = self.sizing.browser() {
            browser.focus_file_view();
        }
        if self.revealer.reveals_child()
            && let Some(split) = self.split.borrow().clone()
        {
            self.closing.set(true);
            self.animate_reveal(&split, false, |state| {
                if state.closing.replace(false) {
                    state.finish_close();
                }
            });
        } else {
            self.finish_close();
        }
    }

    fn archive_tree_focused(&self) -> bool {
        self.archive_browser
            .borrow()
            .as_ref()
            .is_some_and(|browser| {
                self.pane
                    .root()
                    .and_then(|root| root.focus())
                    .is_some_and(|focused| {
                        browser.root().upcast_ref::<gtk::Widget>() == &focused
                            || focused.is_ancestor(browser.root())
                    })
            })
    }

    pub(super) fn finish_close(&self) {
        self.stop();
        self.pane.set_size_request(MIN_WIDTH, -1);
    }
}

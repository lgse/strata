// SPDX-License-Identifier: MIT

#[cfg(test)]
use crate::model::{EntryKind, MetadataValue};
use crate::model::{FileEntry, Location};
use crate::ui::blur::BlurBin;
use crate::ui::browser::ViewState;
use crate::ui::focus_navigation::vim_focus_direction;
use crate::ui::keys::{self, bindings::ESCAPE_ANY};
use crate::ui::modal::{
    ModalHost, dismiss_modal_layer, dismiss_modal_layer_then, modal_layer, show_error_dialog,
};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

mod delete_dialog;
mod empty;
mod restore;
#[cfg(test)]
use delete_dialog::DELETE_CONFIRMATION_MAX_ROWS;
use delete_dialog::DeleteDialog;

/// A presented trash dialog's layer and the overlay it was added to.
#[derive(Clone)]
pub(super) struct TrashModal {
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<BlurBin>,
}

pub(super) type TrashLoadingView = TrashModal;

impl TrashModal {
    fn new(content: &gtk::Box, host: ModalHost) -> Self {
        let layer = modal_layer(content, &host.overlay, host.blurred_root.clone(), None);
        Self {
            layer,
            overlay: host.overlay,
            blurred_root: host.blurred_root,
        }
    }

    fn show(&self) {
        self.overlay.add_overlay(&self.layer);
    }

    fn dismiss(&self) {
        dismiss_modal_layer(&self.layer, &self.overlay, self.blurred_root.as_ref());
    }

    fn dismiss_then(&self, on_done: impl FnOnce() + 'static) {
        dismiss_modal_layer_then(
            &self.layer,
            &self.overlay,
            self.blurred_root.as_ref(),
            on_done,
        );
    }

    /// Escape dismisses and refocuses the listing; vim keys move focus inside the dialog.
    fn install_focus_keys(&self, browser: &Rc<crate::app::Browser>, capture: bool) {
        let keys = gtk::EventControllerKey::new();
        if capture {
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        }
        let modal = self.clone();
        let browser = browser.clone();
        keys::on_press(&keys, move |_, press| {
            if press.is(ESCAPE_ANY) {
                modal.dismiss();
                browser.focus_active();
                glib::Propagation::Stop
            } else if let Some(direction) = vim_focus_direction(press) {
                modal.layer.child_focus(direction);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.layer.add_controller(keys);
    }
}

/// Narrows a just-attempted delete's entries down to the ones a completed
/// operation named as retryable, so a permanent-delete retry (issue #179)
/// re-targets exactly those and not, say, ones that already succeeded or
/// failed for an unrelated reason.
pub(super) fn retryable_delete_entries(
    entries: Vec<FileEntry>,
    retryable_locations: &[Location],
) -> Vec<FileEntry> {
    entries
        .into_iter()
        .filter(|entry| retryable_locations.contains(&entry.location))
        .collect()
}

/// How far a deletion that bypasses Trash reaches. A network or cloud
/// provider may keep its own copy, so only local storage is "permanent".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeletionReach {
    /// Includes `fuseblk` drives and local FUSE overlays such as mergerfs.
    Local,
    Remote,
}

/// A mixed selection uses the stronger local warning.
pub(super) fn deletion_reach(entries: &[FileEntry]) -> DeletionReach {
    let mounts = crate::adapters::mount_watch::latest();
    let remote = |location: &Location| match location.native_path() {
        Some(path) => mounts.is_remote_share(path),
        None => {
            !location.is_administrator()
                && !location
                    .uri_value()
                    .and_then(glib::Uri::parse_scheme)
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("trash"))
        }
    };
    if !entries.is_empty() && entries.iter().all(|entry| remote(&entry.location)) {
        DeletionReach::Remote
    } else {
        DeletionReach::Local
    }
}

/// The partial-failure dialog's button that opens the confirmation.
pub(super) fn delete_retry_label(entries: &[FileEntry]) -> String {
    crate::i18n::tr(match deletion_reach(entries) {
        DeletionReach::Local => "Delete Permanently",
        DeletionReach::Remote => "Delete without Trash",
    })
}

fn reach_description(reach: DeletionReach, single: bool) -> String {
    crate::i18n::tr(match (reach, single) {
        (DeletionReach::Local, true) => {
            "This item will be permanently deleted. This action cannot be undone."
        }
        (DeletionReach::Local, false) => {
            "These items will be permanently deleted. This action cannot be undone."
        }
        (DeletionReach::Remote, true) => {
            "This item will be deleted without using Trash. Strata can't undo this; whether it can be recovered depends on the storage provider."
        }
        (DeletionReach::Remote, false) => {
            "These items will be deleted without using Trash. Strata can't undo this; whether they can be recovered depends on the storage provider."
        }
    })
}

impl ViewState {
    pub(super) fn trash_dropped_locations(self: &Rc<Self>, sources: Vec<Location>) {
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let mut entries = Vec::with_capacity(sources.len());
            for source in sources {
                match crate::adapters::query_file_entry(source).await {
                    Ok(entry) => entries.push(entry),
                    Err(error) => {
                        if let Some(state) = weak.upgrade() {
                            show_error_dialog(
                                &state.overlay,
                                &crate::i18n::tr("Unable to move to Trash"),
                                &error.to_string(),
                            );
                        }
                        return;
                    }
                }
            }
            if let Some(state) = weak.upgrade() {
                state.request_delete(entries, false);
            }
        });
    }

    pub(super) fn delete_animation_source(&self) -> Option<gtk::Widget> {
        self.delete_animation_source_at(self.browser.active_depth())
    }

    fn delete_animation_source_at(&self, depth: Option<usize>) -> Option<gtk::Widget> {
        if self.mode_views.borrow().mode() == crate::ui::browser_modes::BrowserMode::Columns {
            let depth = depth?;
            self.columns
                .borrow()
                .get(depth)
                .map(|column| column.shell.clone().upcast())
        } else {
            Some(self.overlay.clone().upcast())
        }
    }

    /// A docked deletion keeps the dissolve it claimed until its own terminal event.
    pub(super) fn clear_delete_animation(&self) {
        if self.delete_dissolve_request.get().is_none() {
            self.pending_delete_dissolve.take();
        }
        self.pending_file_operation_animation.take();
    }

    /// Frees the animation slots for an operation about to start, taking them
    /// from a docked deletion too.
    fn reset_delete_animation(&self) {
        self.delete_dissolve_request.set(None);
        self.pending_delete_dissolve.take();
        self.pending_file_operation_animation.take();
    }

    pub(super) fn play_pending_file_operation_animation(&self) {
        let Some(animation) = self.pending_file_operation_animation.take() else {
            return;
        };
        animation.play(|| {});
    }

    pub(super) fn delete_animation_defers_empty_state(&self, depth: usize) -> bool {
        self.pending_delete_dissolve
            .borrow()
            .as_ref()
            .is_some_and(|(pending_depth, _)| *pending_depth == depth)
            || self.deferred_delete_empty_depth.get() == Some(depth)
    }

    pub(super) fn request_delete(self: &Rc<Self>, entries: Vec<FileEntry>, permanent: bool) {
        if entries.iter().any(|entry| {
            entry.is_computer_item() || !super::paths::can_remove_location(&entry.location)
        }) {
            return;
        }
        if permanent {
            self.show_delete_confirmation(entries);
        } else if self.trash_unsupported_for(&entries) {
            self.show_trash_unavailable_confirmation(entries);
        } else {
            self.move_to_trash(entries);
        }
    }

    pub(super) fn request_confirmed_delete(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        permanent: bool,
    ) {
        if entries.iter().any(|entry| {
            entry.is_computer_item() || !super::paths::can_remove_location(&entry.location)
        }) {
            return;
        }
        let kind = if permanent {
            DeleteDialog::Permanent
        } else if self.trash_unsupported_for(&entries) {
            DeleteDialog::TrashUnavailable
        } else {
            DeleteDialog::Trash
        };
        self.show_delete_dialog(entries, kind);
    }

    /// Deleting the folder whose column is active acts on its row in the
    /// parent, so the cursor goes there first: a deletion moves it on to a
    /// neighbour, and a cancelled one leaves focus on the folder's row.
    pub(super) fn leave_folder_being_deleted(&self, entries: &[FileEntry]) {
        if self.leaves_folder_being_deleted(entries) {
            self.browser.focus_parent();
        }
    }

    fn leaves_folder_being_deleted(&self, entries: &[FileEntry]) -> bool {
        let open = self
            .browser
            .active_depth()
            .and_then(|depth| self.browser.location_at(depth));
        open.is_some_and(|open| entries.iter().any(|entry| entry.location == open))
            && self.browser.can_start_operation()
    }

    /// The depth that shows `entries` once `leave_folder_being_deleted` has run.
    fn deletion_depth(&self, entries: &[FileEntry]) -> Option<usize> {
        let depth = self.browser.active_depth()?;
        Some(if self.leaves_folder_being_deleted(entries) {
            depth.saturating_sub(1)
        } else {
            depth
        })
    }

    fn move_to_trash(self: &Rc<Self>, entries: Vec<FileEntry>) {
        self.leave_folder_being_deleted(&entries);
        self.pending_delete_entries.replace(entries.clone());
        let source = self.delete_animation_source();
        let trash_button = self.trash_button.upgrade();
        let animation = source.zip(trash_button).and_then(|(source, trash_button)| {
            super::fly_to_trash::prepare_fly_to_trash(&source, entries.iter(), &trash_button)
        });
        self.pending_file_operation_animation.replace(animation);
        self.browser.delete(entries, false);
        self.browser.focus_active();
    }

    /// Only a column reporting delete-but-not-trash qualifies; read-only folders also
    /// report no trash, so they, `None`, and entries outside open columns keep the
    /// trash attempt and its fallback (#179).
    fn trash_unsupported_for(&self, entries: &[FileEntry]) -> bool {
        !entries.is_empty()
            && entries.iter().all(|entry| {
                if entry.location.is_administrator() {
                    return true;
                }
                let parent = entry.location.parent();
                (0..)
                    .map_while(|depth| {
                        self.browser
                            .location_at(depth)
                            .map(|location| (depth, location))
                    })
                    .find(|(_, location)| Some(location) == parent.as_ref())
                    .is_some_and(|(depth, _)| {
                        self.browser.can_trash_at(depth) == Some(false)
                            && self.browser.can_delete_at(depth) == Some(true)
                    })
            })
    }

    pub(super) fn show_delete_confirmation(self: &Rc<Self>, entries: Vec<FileEntry>) {
        self.show_delete_dialog(entries, DeleteDialog::Permanent);
    }

    pub(super) fn show_trash_unavailable_confirmation(self: &Rc<Self>, entries: Vec<FileEntry>) {
        self.show_delete_dialog(entries, DeleteDialog::TrashUnavailable);
    }
}

#[cfg(test)]
mod tests;

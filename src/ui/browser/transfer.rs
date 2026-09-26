// SPDX-License-Identifier: MIT

use crate::adapters::gio_file_for_location;
use crate::app::Browser;
use crate::model::{FileEntry, Location};
use crate::services::{
    DropCommit, MoveRecord, PasteItem, TransferConflict, UndoMoveItem, VolumeRelation,
    transferable_drop_sources,
};
use crate::ui::browser::ViewState;
use crate::ui::browser::destination::{
    DestinationLocationBar, TransferSearchScope, folder_input_path, hand_off_destination_focus,
    resolve_destination_path, setup_transfer_search,
};
use crate::ui::browser::entry::item_count_label;
use crate::ui::browser::paths::{
    can_remove_location, compact_display_path, compact_native_path, is_trash_location,
};
use crate::ui::controls::{
    ModalTone, focus_button, form_check_button, form_entry, form_label, message_dialog_description,
    message_dialog_layout, modal_layout,
};
use crate::ui::modal::{
    ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog, submit_on_enter,
};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

type RemovableRootResolver = Rc<dyn Fn(&str) -> Option<PathBuf>>;

const SEND_TO_SUCCESS_DURATION: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
enum ConflictChoice {
    Replace,
    Merge,
    Skip,
    KeepBoth,
}

#[derive(Clone)]
struct TransferCollision {
    source: Location,
    /// Both colliding items are directories, so their contents can be merged.
    mergeable: bool,
}

#[derive(Clone)]
pub(super) struct SendToTransferContext {
    pub(super) device_name: String,
}

#[derive(Clone)]
pub(super) struct PendingSendToCompletion {
    pub(super) device_name: String,
    pub(super) item_count: usize,
}

#[derive(Clone)]
pub(super) struct FinishedSendToCompletion {
    pub(super) completion: PendingSendToCompletion,
    pub(super) progress_shown: bool,
}

fn send_to_display_name(id: &str, root: &Path) -> String {
    crate::ui::removable_destinations()
        .into_iter()
        .find(|destination| destination.id == id)
        .map(|destination| destination.name)
        .or_else(|| {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| root.to_string_lossy().into_owned())
}

/// Which non-destructive resolutions a conflict prompt offers.
#[derive(Clone, Copy, Default)]
struct ConflictActions {
    keep_both: bool,
    merge: bool,
}

struct TransferDialogOptions {
    base: PathBuf,
    search_root: PathBuf,
    root_limit: Option<PathBuf>,
    root_label: Option<String>,
    allow_create: bool,
    completion: TransferDialogCompletion,
}

#[derive(Clone)]
enum TransferDialogCompletion {
    CopyMove {
        move_sources: bool,
    },
    SendTo {
        device_id: String,
        opened_root: PathBuf,
        resolve: RemovableRootResolver,
    },
}

fn location_exists(location: &Location) -> bool {
    gio_file_for_location(location).query_exists(None::<&gio::Cancellable>)
}

fn transfer_is_noop(source: &Location, destination: &Location, move_sources: bool) -> bool {
    let source = gio_file_for_location(source);
    let destination = gio_file_for_location(destination);
    source.equal(&destination)
        || destination.has_prefix(&source)
        || (move_sources
            && source
                .parent()
                .is_some_and(|parent| parent.equal(&destination)))
}

fn transfer_collision(source: &Location, destination: &Location) -> Option<TransferCollision> {
    let source_file = gio_file_for_location(source);
    let destination_file = gio_file_for_location(destination);
    let name = source_file.basename()?;
    let target = destination_file.child(name);
    if source_file.equal(&target)
        || source_file.equal(&destination_file)
        || destination_file.has_prefix(&source_file)
        || !target.query_exists(None::<&gio::Cancellable>)
    {
        return None;
    }
    let is_directory = |file: &gio::File| {
        file.query_file_type(
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            None::<&gio::Cancellable>,
        ) == gio::FileType::Directory
    };
    Some(TransferCollision {
        source: source.clone(),
        mergeable: is_directory(&source_file) && is_directory(&target),
    })
}

fn cross_volume_drop_description(volume: VolumeRelation) -> &'static str {
    match volume {
        VolumeRelation::Different => "The destination is on a different device.",
        VolumeRelation::Same | VolumeRelation::Unknown => {
            "Strata could not determine whether the destination is on the same device."
        }
    }
}

pub(super) fn duplicate_transfer(entries: &[FileEntry]) -> Option<(Location, Vec<Location>)> {
    let destination = entries.first()?.location.parent()?;
    if is_trash_location(&destination)
        || !entries
            .iter()
            .all(|entry| entry.location.parent().as_ref() == Some(&destination))
    {
        return None;
    }
    let sources = entries.iter().map(|entry| entry.location.clone()).collect();
    Some((destination, sources))
}

impl ViewState {
    pub(super) fn commit_file_drop(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        commit: DropCommit,
    ) {
        self.stop_drag_autoscroll();
        self.drop_active_depths.set(None);
        self.horizontal_scroll_generation
            .set(self.horizontal_scroll_generation.get().saturating_add(1));
        let sources = transferable_drop_sources(&destination, &sources);
        if sources.is_empty() {
            self.suppress_scroll_after_drop.set(false);
            return;
        }
        match commit {
            DropCommit::Copy => self.start_drop_transfer(destination, sources, false),
            DropCommit::Move => self.start_drop_transfer(destination, sources, true),
            DropCommit::Ask { volume, .. } => {
                self.confirm_cross_volume_drop(destination, sources, volume);
            }
            DropCommit::Forbidden => {
                self.suppress_scroll_after_drop.set(false);
            }
        }
    }

    fn confirm_cross_volume_drop(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        volume: VolumeRelation,
    ) {
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            show_error_dialog(
                &self.overlay,
                "Unable to transfer",
                "The transfer could not be confirmed.",
            );
            return;
        };

        let count = sources.len();
        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            "Copy or move?",
            &format!(
                "{} to {}",
                item_count_label(count),
                compact_display_path(&destination)
            ),
            "Copy",
            ModalTone::Accent,
        );
        layout
            .body
            .append(&message_dialog_description(cross_volume_drop_description(
                volume,
            )));
        let move_button = gtk::Button::with_label("Move");
        move_button.add_css_class("action-dialog-cancel");
        layout
            .actions
            .insert_child_after(&move_button, Some(&layout.cancel));
        let content = layout.content;
        let cancel = layout.cancel;
        let copy = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);

        let dismiss_layer = layer.clone();
        let dismiss_overlay = window_overlay.clone();
        let dismiss_root = blurred_root.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(&dismiss_layer, &dismiss_overlay, dismiss_root.as_ref());
        });
        let closed_layer = layer.clone();
        let closed_overlay = window_overlay.clone();
        let closed_root = blurred_root.clone();
        layout.close.connect_clicked(move |_| {
            dismiss_modal_layer(&closed_layer, &closed_overlay, closed_root.as_ref());
        });

        for (button, move_sources) in [(move_button.clone(), true), (copy.clone(), false)] {
            let chosen_layer = layer.clone();
            let chosen_overlay = window_overlay.clone();
            let chosen_root = blurred_root.clone();
            let chosen_state = self.clone();
            let chosen_destination = destination.clone();
            let chosen_sources = sources.clone();
            button.connect_clicked(move |_| {
                dismiss_modal_layer(&chosen_layer, &chosen_overlay, chosen_root.as_ref());
                chosen_state.start_drop_transfer(
                    chosen_destination.clone(),
                    chosen_sources.clone(),
                    move_sources,
                );
            });
        }

        let escape = gtk::EventControllerKey::new();
        escape.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        copy.grab_focus();
    }

    pub(super) fn start_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        self.start_transfer_with_reveal(destination, sources, move_sources, true, None);
    }

    pub(super) fn send_to_removable_device(self: &Rc<Self>, id: String, sources: Vec<Location>) {
        self.send_to_removable_device_with_resolver(
            &id,
            sources,
            crate::ui::resolve_removable_destination,
        );
    }

    pub(super) fn send_to_removable_device_with_resolver(
        self: &Rc<Self>,
        id: &str,
        sources: Vec<Location>,
        resolve: impl FnOnce(&str) -> Option<PathBuf>,
    ) {
        let Some(destination) = resolve(id) else {
            show_error_dialog(
                &self.overlay,
                "Destination unavailable",
                "The removable device is no longer available.",
            );
            return;
        };
        let device_name = send_to_display_name(id, &destination);
        self.send_to(
            Location::local(destination),
            sources,
            SendToTransferContext { device_name },
        );
    }

    pub(super) fn send_to_recent_destination(
        self: &Rc<Self>,
        id: String,
        relative: PathBuf,
        sources: Vec<Location>,
    ) {
        self.send_to_recent_destination_with_resolver(
            &id,
            &relative,
            sources,
            crate::ui::resolve_removable_destination,
        );
    }

    pub(super) fn send_to_recent_destination_with_resolver(
        self: &Rc<Self>,
        id: &str,
        relative: &Path,
        sources: Vec<Location>,
        resolve: impl FnOnce(&str) -> Option<PathBuf>,
    ) {
        if !crate::ui::preferences::is_valid_send_to_relative_path(relative) {
            show_error_dialog(
                &self.overlay,
                "Destination unavailable",
                "The removable device or folder is no longer available.",
            );
            return;
        }
        let Some(current_root) = resolve(id)
            .and_then(|root| crate::ui::browser::destination::canonical_existing_directory(&root))
        else {
            show_error_dialog(
                &self.overlay,
                "Destination unavailable",
                "The removable device or folder is no longer available.",
            );
            return;
        };
        let Some(destination) = crate::ui::browser::destination::canonical_directory_within(
            &current_root,
            &current_root.join(relative),
        ) else {
            show_error_dialog(
                &self.overlay,
                "Destination unavailable",
                "The removable device or folder is no longer available.",
            );
            return;
        };
        let Ok(relative_destination) = destination.strip_prefix(&current_root) else {
            show_error_dialog(
                &self.overlay,
                "Destination unavailable",
                "The removable device or folder is no longer available.",
            );
            return;
        };
        if !crate::ui::preferences::is_valid_send_to_relative_path(relative_destination) {
            show_error_dialog(
                &self.overlay,
                "Destination unavailable",
                "The removable device or folder is no longer available.",
            );
            return;
        }
        crate::ui::preferences::PreferenceManager::shared().remember_send_to_destination(
            id,
            relative_destination,
            Some(relative),
        );
        let device_name = send_to_display_name(id, &current_root);
        self.send_to(
            Location::local(destination),
            sources,
            SendToTransferContext { device_name },
        );
    }

    pub(super) fn send_to(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        send_to: SendToTransferContext,
    ) {
        self.start_transfer_with_reveal(destination, sources, false, false, Some(send_to));
    }

    fn send_to_success_text(device_name: &str, item_count: usize) -> String {
        if item_count == 1 {
            format!("Copied to {device_name}")
        } else {
            format!("{item_count} items copied to {device_name}")
        }
    }

    /// Shows transient success feedback for a fast Send-to whose transfer
    /// finished before the normal progress UI became visible. Replaces any
    /// still-visible notice and restarts its dismissal timer.
    pub(super) fn show_send_to_success(self: &Rc<Self>, device_name: &str, item_count: usize) {
        if let Some(widget) = self.send_to_success_widget.take() {
            self.overlay.remove_overlay(&widget);
        }
        let generation = self.send_to_success_generation.get().saturating_add(1);
        self.send_to_success_generation.set(generation);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("send-to-success");
        row.set_halign(gtk::Align::Center);
        row.set_valign(gtk::Align::Start);
        row.set_can_target(false);
        let icon = crate::assets::primary_icon(crate::assets::icons::CHECK, 16);
        icon.set_can_target(false);
        row.append(&icon);
        let label = gtk::Label::new(Some(&Self::send_to_success_text(device_name, item_count)));
        label.set_can_target(false);
        row.append(&label);
        self.overlay.add_overlay(&row);
        self.send_to_success_widget
            .replace(Some(row.clone().upcast()));
        let weak_state = Rc::downgrade(self);
        let weak_row = row.downgrade();
        glib::timeout_add_local_once(SEND_TO_SUCCESS_DURATION, move || {
            let (Some(state), Some(row)) = (weak_state.upgrade(), weak_row.upgrade()) else {
                return;
            };
            if state.send_to_success_generation.get() != generation {
                return;
            }
            state.send_to_success_widget.take();
            state.overlay.remove_overlay(&row);
        });
    }

    fn start_drop_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        let reveal = crate::ui::preferences::PreferenceManager::shared().open_folder_after_drop();
        self.start_transfer_with_reveal(destination, sources, move_sources, reveal, None);
    }

    /// Paste and explicit "move/copy to" reveal their result independently of
    /// the drop preference, which is captured when the transfer starts.
    pub(super) fn start_transfer_with_reveal(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
        reveal: bool,
        send_to: Option<SendToTransferContext>,
    ) {
        if is_trash_location(&destination)
            || destination.is_recent_location()
            || (move_sources && sources.iter().any(|source| !can_remove_location(source)))
        {
            return;
        }
        let sources: Vec<Location> = sources
            .into_iter()
            .filter(|source| !transfer_is_noop(source, &destination, move_sources))
            .collect();
        if sources.is_empty() {
            return;
        }
        let mut accepted = Vec::new();
        let mut collisions = Vec::new();
        for source in sources {
            match transfer_collision(&source, &destination) {
                Some(collision) => collisions.push(collision),
                None => accepted.push(PasteItem {
                    source,
                    conflict: TransferConflict::FailIfExists,
                }),
            }
        }
        self.resolve_transfer_collisions(
            destination,
            collisions,
            accepted,
            move_sources,
            reveal,
            send_to,
        );
    }

    fn resolve_transfer_collisions(
        self: &Rc<Self>,
        destination: Location,
        mut collisions: Vec<TransferCollision>,
        accepted: Vec<PasteItem>,
        move_sources: bool,
        reveal: bool,
        send_to: Option<SendToTransferContext>,
    ) {
        if collisions.is_empty() {
            let source_depth = self
                .drag_source_depth
                .replace(None)
                .or_else(|| self.browser.active_depth());
            self.drop_active_depths.set(None);
            if !accepted.is_empty() {
                self.suppress_scroll_after_drop.set(!reveal);
                if !reveal {
                    let destination_depth = (0..)
                        .map_while(|depth| self.browser.location_at(depth))
                        .position(|location| location == destination);
                    self.drop_active_depths
                        .set(source_depth.zip(destination_depth));
                }
                // A new dispatch abandons any previous operation, so stale
                // completion state must not leak into this transfer's outcome.
                self.pending_send_to_completion.take();
                self.finished_send_to_completion.take();
                if let Some(send_to) = send_to {
                    self.pending_send_to_completion
                        .replace(Some(PendingSendToCompletion {
                            device_name: send_to.device_name,
                            item_count: accepted.len(),
                        }));
                }
            }
            self.browser
                .transfer(destination, accepted, move_sources, reveal);
            return;
        }
        let collision = collisions.remove(0);
        let source = collision.source.clone();
        let name = source.display_name();
        // Merge stays copy-only: undoing a merged move cannot tell which
        // destination contents the source actually owned.
        let allow_merge = collision.mergeable && !move_sources;
        let explanation = if allow_merge {
            format!(
                "A folder named \u{201c}{name}\u{201d} already exists in {}. Merging combines the contents of both folders; incoming items overwrite items with the same name.",
                compact_display_path(&destination)
            )
        } else {
            format!(
                "An item named \u{201c}{name}\u{201d} already exists in {}. Replacing it will overwrite its contents.",
                compact_display_path(&destination)
            )
        };
        let state = self.clone();
        let send_to_conflict = send_to.clone();
        // Move undo/reveal assumes an unrenamed `transfer_target`.
        let apply_to_all_visible = !collisions.is_empty();
        let skip_visible = !accepted.is_empty() || !collisions.is_empty();
        self.confirm_replace_conflict(
            &name,
            &explanation,
            apply_to_all_visible,
            skip_visible,
            ConflictActions {
                keep_both: !move_sources,
                merge: allow_merge,
            },
            Rc::new(move |choice, apply_to_all| {
                let mut accepted = accepted.clone();
                let mut remaining = collisions.clone();
                match choice {
                    ConflictChoice::Replace => {
                        accepted.push(PasteItem {
                            source: source.clone(),
                            conflict: TransferConflict::ReplaceExisting,
                        });
                        if apply_to_all {
                            accepted.extend(remaining.drain(..).map(|collision| PasteItem {
                                source: collision.source,
                                conflict: TransferConflict::ReplaceExisting,
                            }));
                        }
                    }
                    ConflictChoice::Merge => {
                        accepted.push(PasteItem {
                            source: source.clone(),
                            conflict: TransferConflict::Merge,
                        });
                        if apply_to_all {
                            // File collisions still need their own prompt.
                            let (mergeable, rest): (Vec<_>, Vec<_>) = remaining
                                .into_iter()
                                .partition(|collision| collision.mergeable);
                            accepted.extend(mergeable.into_iter().map(|collision| PasteItem {
                                source: collision.source,
                                conflict: TransferConflict::Merge,
                            }));
                            remaining = rest;
                        }
                    }
                    ConflictChoice::KeepBoth => {
                        accepted.push(PasteItem {
                            source: source.clone(),
                            conflict: TransferConflict::KeepBoth,
                        });
                        if apply_to_all {
                            accepted.extend(remaining.drain(..).map(|collision| PasteItem {
                                source: collision.source,
                                conflict: TransferConflict::KeepBoth,
                            }));
                        }
                    }
                    ConflictChoice::Skip if apply_to_all => remaining.clear(),
                    ConflictChoice::Skip => {}
                }
                state.resolve_transfer_collisions(
                    destination.clone(),
                    remaining,
                    accepted,
                    move_sources,
                    reveal,
                    send_to_conflict.clone(),
                );
            }),
        );
    }

    /// Moves the latest completed transfer back, confirming any item that would
    /// overwrite something created since the move.
    pub(super) fn undo_move(self: &Rc<Self>, generation: u64, records: Vec<MoveRecord>) -> bool {
        self.replay_move(false, generation, records)
    }

    pub(super) fn redo_trash(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        self.replay_existing_locations(true, generation, locations, Browser::redo_trash)
    }

    pub(super) fn redo_move(self: &Rc<Self>, generation: u64, records: Vec<MoveRecord>) -> bool {
        self.replay_move(true, generation, records)
    }

    fn replay_existing_locations(
        self: &Rc<Self>,
        redo: bool,
        generation: u64,
        locations: Vec<Location>,
        dispatch: fn(&Rc<Browser>, u64, Vec<Location>) -> bool,
    ) -> bool {
        let existing = locations
            .into_iter()
            .filter(location_exists)
            .collect::<Vec<_>>();
        if existing.is_empty() {
            self.browser.discard_pending_replay(redo, generation);
            return false;
        }
        dispatch(&self.browser, generation, existing)
    }

    fn replay_move(self: &Rc<Self>, redo: bool, generation: u64, records: Vec<MoveRecord>) -> bool {
        let mut accepted = Vec::new();
        let mut collisions = Vec::new();
        for record in records {
            let (item_at, destination) = if redo {
                (&record.original, &record.current)
            } else {
                (&record.current, &record.original)
            };
            if !location_exists(item_at) {
                continue;
            }
            if location_exists(destination) {
                collisions.push(record);
            } else {
                accepted.push(UndoMoveItem {
                    record,
                    conflict: TransferConflict::FailIfExists,
                });
            }
        }
        if accepted.is_empty() && collisions.is_empty() {
            self.browser.discard_pending_replay(redo, generation);
            return false;
        }
        self.resolve_replay_collisions(redo, generation, collisions, accepted);
        true
    }

    pub(super) fn undo_copy(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        self.replay_existing_locations(false, generation, locations, Browser::undo_copy)
    }

    pub(super) fn undo_merge(
        self: &Rc<Self>,
        generation: u64,
        created: Vec<Location>,
        overwritten: Vec<Location>,
    ) -> bool {
        let created = created
            .into_iter()
            .filter(location_exists)
            .collect::<Vec<_>>();
        // Overwritten entries stay even when the incoming copy is gone: the
        // staged original may still be sitting in Trash waiting to restore.
        if created.is_empty() && overwritten.is_empty() {
            self.browser.discard_pending_replay(false, generation);
            return false;
        }
        self.browser.undo_merge(generation, created, overwritten)
    }

    fn resolve_replay_collisions(
        self: &Rc<Self>,
        redo: bool,
        generation: u64,
        mut collisions: Vec<MoveRecord>,
        accepted: Vec<UndoMoveItem>,
    ) {
        if collisions.is_empty() {
            if accepted.is_empty() {
                self.browser.discard_pending_replay(redo, generation);
            } else if redo {
                self.browser.redo_move(generation, accepted);
            } else {
                self.browser.undo_move(generation, accepted);
            }
            return;
        }
        let record = collisions.remove(0);
        let destination = if redo {
            &record.current
        } else {
            &record.original
        };
        let name = destination.display_name();
        let parent = destination.parent().unwrap_or_else(|| destination.clone());
        let action = if redo { "Redoing" } else { "Undoing" };
        let explanation = format!(
            "An item named \u{201c}{name}\u{201d} already exists in {}. {action} the move will overwrite its contents.",
            compact_display_path(&parent)
        );
        let state = self.clone();
        let apply_to_all_visible = !collisions.is_empty();
        let skip_visible = !accepted.is_empty() || !collisions.is_empty();
        self.confirm_replace_conflict(
            &name,
            &explanation,
            apply_to_all_visible,
            skip_visible,
            ConflictActions::default(),
            Rc::new(move |choice, apply_to_all| {
                let mut accepted = accepted.clone();
                let mut remaining = collisions.clone();
                match choice {
                    ConflictChoice::Replace => {
                        accepted.push(UndoMoveItem {
                            record: record.clone(),
                            conflict: TransferConflict::ReplaceExisting,
                        });
                        if apply_to_all {
                            accepted.extend(remaining.drain(..).map(|record| UndoMoveItem {
                                record,
                                conflict: TransferConflict::ReplaceExisting,
                            }));
                        }
                    }
                    ConflictChoice::Merge | ConflictChoice::KeepBoth => {
                        unreachable!("merge and keep-both are not offered for replay conflicts")
                    }
                    ConflictChoice::Skip if apply_to_all => remaining.clear(),
                    ConflictChoice::Skip => {}
                }
                state.resolve_replay_collisions(redo, generation, remaining, accepted);
            }),
        );
    }

    /// Cancelling abandons the whole operation without calling `on_choice`.
    fn confirm_replace_conflict(
        self: &Rc<Self>,
        name: &str,
        explanation: &str,
        apply_to_all_visible: bool,
        skip_visible: bool,
        actions: ConflictActions,
        on_choice: Rc<dyn Fn(ConflictChoice, bool)>,
    ) {
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };

        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            "File already exists",
            name,
            "Replace",
            ModalTone::Danger,
        );
        layout.body.append(&message_dialog_description(explanation));
        let apply_all = form_check_button("Apply to All");
        apply_all.set_visible(apply_to_all_visible);
        layout.actions.prepend(&apply_all);
        let skip = gtk::Button::with_label("Skip");
        skip.add_css_class("action-dialog-cancel");
        skip.set_visible(skip_visible);
        layout
            .actions
            .insert_child_after(&skip, Some(&layout.cancel));
        let keep_both = gtk::Button::with_label("Keep Both");
        keep_both.add_css_class("action-dialog-cancel");
        keep_both.set_visible(actions.keep_both);
        layout.actions.insert_child_after(&keep_both, Some(&skip));
        let merge = gtk::Button::with_label("Merge");
        merge.add_css_class("action-dialog-cancel");
        merge.set_visible(actions.merge);
        layout.actions.insert_child_after(&merge, Some(&keep_both));
        let content = layout.content;
        let cancel = layout.cancel;
        let replace = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        let browser = Rc::downgrade(&self.browser);
        layer.connect_parent_notify(move |layer| {
            if layer.parent().is_none()
                && let Some(browser) = browser.upgrade()
            {
                browser.focus_active();
            }
        });
        let cancel_layer = layer.clone();
        let cancel_overlay = window_overlay.clone();
        let cancel_root = blurred_root.clone();
        let dismiss_layer = cancel_layer.clone();
        let dismiss_overlay = cancel_overlay.clone();
        let dismiss_root = cancel_root.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
        });
        layout.close.connect_clicked(move |_| {
            dismiss_modal_layer(&dismiss_layer, &dismiss_overlay, dismiss_root.as_ref());
        });

        for (button, choice) in [
            (skip.clone(), ConflictChoice::Skip),
            (keep_both.clone(), ConflictChoice::KeepBoth),
            (merge.clone(), ConflictChoice::Merge),
            (replace.clone(), ConflictChoice::Replace),
        ] {
            let chosen_layer = layer.clone();
            let chosen_overlay = window_overlay.clone();
            let chosen_root = blurred_root.clone();
            let chosen_apply_all = apply_all.clone();
            let chosen = on_choice.clone();
            button.connect_clicked(move |_| {
                dismiss_modal_layer(&chosen_layer, &chosen_overlay, chosen_root.as_ref());
                chosen(choice, chosen_apply_all.is_active());
            });
        }

        let escape = gtk::EventControllerKey::new();
        escape.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        let enter_buttons = [
            skip,
            keep_both,
            merge,
            replace.clone(),
            cancel,
            layout.close,
        ];
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                if let Some(button) = enter_buttons.iter().find(|button| button.has_focus()) {
                    button.emit_clicked();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        focus_button(&replace);
    }

    pub(super) fn show_transfer_dialog(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        move_sources: bool,
    ) {
        if entries.is_empty()
            || (move_sources
                && entries
                    .iter()
                    .any(|entry| !can_remove_location(&entry.location)))
        {
            return;
        }
        let base = self
            .browser
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf))
            .unwrap_or_else(glib::home_dir);
        self.show_destination_dialog(
            entries.into_iter().map(|entry| entry.location).collect(),
            TransferDialogOptions {
                base,
                search_root: glib::home_dir(),
                root_limit: None,
                root_label: None,
                allow_create: true,
                completion: TransferDialogCompletion::CopyMove { move_sources },
            },
        );
    }

    pub(super) fn show_send_to_folder_dialog(self: &Rc<Self>, id: String, sources: Vec<Location>) {
        self.show_send_to_folder_dialog_with_resolver(
            id,
            sources,
            Rc::new(crate::ui::resolve_removable_destination),
        );
    }

    pub(super) fn show_send_to_folder_dialog_with_resolver(
        self: &Rc<Self>,
        id: String,
        sources: Vec<Location>,
        resolve: RemovableRootResolver,
    ) {
        if sources.is_empty() {
            return;
        }
        let Some(root) = resolve(&id)
            .and_then(|root| crate::ui::browser::destination::canonical_existing_directory(&root))
        else {
            show_error_dialog(
                &self.overlay,
                "Destination unavailable",
                "The removable device is no longer available.",
            );
            return;
        };
        self.show_destination_dialog(
            sources,
            TransferDialogOptions {
                base: root.clone(),
                search_root: root.clone(),
                root_limit: Some(root.clone()),
                root_label: crate::ui::removable_destinations()
                    .into_iter()
                    .find(|destination| destination.id == id)
                    .map(|destination| destination.name),
                allow_create: false,
                completion: TransferDialogCompletion::SendTo {
                    device_id: id,
                    opened_root: root,
                    resolve,
                },
            },
        );
    }

    fn show_destination_dialog(
        self: &Rc<Self>,
        sources: Vec<Location>,
        options: TransferDialogOptions,
    ) {
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };

        let TransferDialogOptions {
            base,
            search_root,
            root_limit,
            root_label,
            allow_create,
            completion,
        } = options;
        let move_sources = match &completion {
            TransferDialogCompletion::CopyMove { move_sources } => *move_sources,
            TransferDialogCompletion::SendTo { .. } => false,
        };
        let (icon, title, confirm_label) = match &completion {
            TransferDialogCompletion::CopyMove { move_sources: true } => {
                (crate::assets::icons::FOLDER_INPUT, "Move to", "Move here")
            }
            TransferDialogCompletion::CopyMove {
                move_sources: false,
            } => (crate::assets::icons::FOLDER_OUTPUT, "Copy to", "Copy here"),
            TransferDialogCompletion::SendTo { .. } => (
                crate::assets::icons::SEND_HORIZONTAL,
                "Send to",
                "Copy here",
            ),
        };
        let layout = modal_layout(
            icon,
            title,
            &format!(
                "Choose a destination for {}",
                item_count_label(sources.len())
            ),
            confirm_label,
        );
        layout.content.add_css_class("wide");
        let field_label = form_label("Destination folder");
        let field = form_entry();
        field.set_hexpand(true);
        field.set_placeholder_text(Some("Search for a folder…"));
        field.set_text(&folder_input_path(&base));
        field.set_position(-1);
        layout.body.append(&field_label);
        let location_bar = DestinationLocationBar::wrap(
            field.clone(),
            base.clone(),
            search_root.clone(),
            root_limit.clone(),
            root_label.clone(),
        );
        layout.body.append(&location_bar.widget());

        let suggestions = gtk::Box::new(gtk::Orientation::Vertical, 2);
        suggestions.add_css_class("transfer-suggestions");
        let suggestion_scroll = gtk::ScrolledWindow::builder()
            .child(&suggestions)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(150)
            .max_content_height(220)
            .propagate_natural_height(true)
            .build();
        suggestion_scroll.add_css_class("transfer-suggestion-scroll");
        layout.body.append(&suggestion_scroll);
        let error = gtk::Label::new(None);
        error.add_css_class("form-message");
        error.add_css_class("error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        layout.body.append(&error);
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let confirm = layout.confirm;

        let generation = Rc::new(Cell::new(0_u64));
        let pending_creation = Rc::new(RefCell::new(None::<std::path::PathBuf>));
        let creating_destination = Rc::new(Cell::new(false));
        let suggestions_box = suggestions.clone();
        let suggestions_error = error.clone();
        let changed_confirm = confirm.clone();
        let changed_creation = pending_creation.clone();
        let select_bar = location_bar.clone();
        setup_transfer_search(
            &field,
            &suggestions_box,
            &generation,
            TransferSearchScope {
                base: base.clone(),
                search_root: search_root.clone(),
                root_limit: root_limit.clone(),
                show_hidden: self.browser.preferences().show_hidden,
            },
            Rc::new(move |path: &Path| select_bar.select_directory(path)),
            move |field| {
                field.remove_css_class("error");
                suggestions_error.set_visible(false);
                suggestions_error.remove_css_class("warning");
                suggestions_error.add_css_class("error");
                changed_creation.borrow_mut().take();
                changed_confirm.set_label(if move_sources {
                    "Move here"
                } else {
                    "Copy here"
                });
            },
        );

        let initial_text = folder_input_path(&base);
        let dirty_field = field.clone();
        let dirty_creating = creating_destination.clone();
        let layer = modal_layer(
            &content,
            &window_overlay,
            blurred_root.clone(),
            Some(Rc::new(move || {
                dirty_creating.get() || dirty_field.text() != initial_text
            })),
        );
        window_overlay.add_overlay(&layer);
        let cancel_layer = layer.clone();
        let cancel_overlay = window_overlay.clone();
        let cancel_root = blurred_root.clone();
        let cancel_creating = creating_destination.clone();
        cancel.connect_clicked(move |_| {
            if !cancel_creating.get() {
                dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
            }
        });
        let close_layer = layer.clone();
        let close_overlay = window_overlay.clone();
        let close_root = blurred_root.clone();
        let close_creating = creating_destination.clone();
        close.connect_clicked(move |_| {
            if !close_creating.get() {
                dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
            }
        });
        let confirm_layer = layer.clone();
        let confirm_overlay = window_overlay.clone();
        let confirm_root = blurred_root.clone();
        let transfer_state = self.clone();
        let confirm_field = field.clone();
        let confirm_error = error.clone();
        let confirm_base = base.clone();
        let confirm_creation = pending_creation;
        let confirm_creating = creating_destination.clone();
        let confirm_cancel = cancel.clone();
        let confirm_close = close.clone();
        let completion = completion.clone();
        confirm.connect_clicked(move |button| {
            let path =
                resolve_destination_path(&confirm_field.text(), &confirm_base, &glib::home_dir());
            if let TransferDialogCompletion::SendTo {
                device_id,
                opened_root,
                resolve,
            } = &completion
            {
                let Some(current_root) = resolve(device_id).and_then(|root| {
                    crate::ui::browser::destination::canonical_existing_directory(&root)
                }) else {
                    confirm_error.remove_css_class("warning");
                    confirm_error.add_css_class("error");
                    confirm_error.set_text("The removable device is no longer available.");
                    confirm_error.set_visible(true);
                    confirm_field.add_css_class("error");
                    confirm_field.grab_focus();
                    return;
                };
                let Some(destination) =
                    crate::ui::browser::destination::rebind_directory_within_root(
                        opened_root,
                        &path,
                        &current_root,
                    )
                else {
                    confirm_error.remove_css_class("warning");
                    confirm_error.add_css_class("error");
                    confirm_error
                        .set_text("Choose an existing folder inside this removable device.");
                    confirm_error.set_visible(true);
                    confirm_field.add_css_class("error");
                    confirm_field.grab_focus();
                    return;
                };
                if let Ok(relative_destination) = destination.strip_prefix(&current_root) {
                    crate::ui::preferences::PreferenceManager::shared()
                        .remember_send_to_destination(device_id, relative_destination, None);
                }
                let device_name = send_to_display_name(device_id, opened_root);
                transfer_state.send_to(
                    Location::local(destination),
                    sources.clone(),
                    SendToTransferContext { device_name },
                );
                hand_off_destination_focus(&confirm_field, button);
                dismiss_modal_layer(&confirm_layer, &confirm_overlay, confirm_root.as_ref());
                return;
            }
            if path.exists() && !path.is_dir() {
                confirm_error.remove_css_class("warning");
                confirm_error.add_css_class("error");
                confirm_error.set_text("The destination exists, but it is not a folder.");
                confirm_error.set_visible(true);
                confirm_field.add_css_class("error");
                confirm_field.grab_focus();
                return;
            }
            if !path.exists() && !allow_create {
                confirm_error.remove_css_class("warning");
                confirm_error.add_css_class("error");
                confirm_error.set_text("Choose an existing folder.");
                confirm_error.set_visible(true);
                confirm_field.add_css_class("error");
                confirm_field.grab_focus();
                return;
            }
            if !path.exists() && confirm_creation.borrow().as_ref() != Some(&path) {
                confirm_creation.replace(Some(path.clone()));
                confirm_error.remove_css_class("error");
                confirm_error.add_css_class("warning");
                confirm_error.set_text(&format!(
                    "{} does not exist. It will be created before the items are transferred.",
                    compact_native_path(&path)
                ));
                confirm_error.set_visible(true);
                button.set_label(if move_sources {
                    "Create and move"
                } else {
                    "Create and copy"
                });
                button.grab_focus();
                return;
            }
            if path.is_dir() {
                transfer_state
                    .pending_navigate
                    .replace(Some(Location::local(path.clone())));
                let names: Vec<String> = sources
                    .iter()
                    .filter_map(|s| s.native_path()?.file_name()?.to_str().map(String::from))
                    .collect();
                transfer_state.pending_select.borrow_mut().extend(names);
                transfer_state.start_transfer(Location::local(path), sources.clone(), move_sources);
                hand_off_destination_focus(&confirm_field, button);
                dismiss_modal_layer(&confirm_layer, &confirm_overlay, confirm_root.as_ref());
                return;
            }

            confirm_creating.set(true);
            button.set_sensitive(false);
            button.set_label("Creating folder…");
            confirm_field.set_sensitive(false);
            confirm_cancel.set_sensitive(false);
            confirm_close.set_sensitive(false);
            let created_state = transfer_state.clone();
            let created_sources = sources.clone();
            let created_layer = confirm_layer.clone();
            let created_overlay = confirm_overlay.clone();
            let created_root = confirm_root.clone();
            let created_button = button.clone();
            let created_field = confirm_field.clone();
            let created_error = confirm_error.clone();
            let created_creating = confirm_creating.clone();
            let created_cancel = confirm_cancel.clone();
            let created_close = confirm_close.clone();
            glib::MainContext::default().spawn_local(async move {
                let created_path = path.clone();
                let result =
                    gio::spawn_blocking(move || std::fs::create_dir_all(&created_path)).await;
                match result {
                    Ok(Ok(())) => {
                        created_state
                            .pending_navigate
                            .replace(Some(Location::local(path.clone())));
                        let names: Vec<String> = created_sources
                            .iter()
                            .filter_map(|s| {
                                s.native_path()?.file_name()?.to_str().map(String::from)
                            })
                            .collect();
                        created_state.pending_select.borrow_mut().extend(names);
                        created_state.start_transfer(
                            Location::local(path),
                            created_sources,
                            move_sources,
                        );
                        hand_off_destination_focus(&created_field, &created_button);
                        dismiss_modal_layer(
                            &created_layer,
                            &created_overlay,
                            created_root.as_ref(),
                        );
                    }
                    Ok(Err(error)) => {
                        created_creating.set(false);
                        created_cancel.set_sensitive(true);
                        created_close.set_sensitive(true);
                        created_error.remove_css_class("warning");
                        created_error.add_css_class("error");
                        created_error.set_text(&format!("Unable to create that folder: {error}"));
                        created_error.set_visible(true);
                        created_field.add_css_class("error");
                        created_field.set_sensitive(true);
                        created_field.grab_focus();
                        created_button.set_sensitive(true);
                        created_button.set_label(if move_sources {
                            "Move here"
                        } else {
                            "Copy here"
                        });
                    }
                    Err(_) => {
                        created_creating.set(false);
                        created_cancel.set_sensitive(true);
                        created_close.set_sensitive(true);
                        created_error.remove_css_class("warning");
                        created_error.add_css_class("error");
                        created_error.set_text("Unable to create that folder.");
                        created_error.set_visible(true);
                        created_field.add_css_class("error");
                        created_field.set_sensitive(true);
                        created_field.grab_focus();
                        created_button.set_sensitive(true);
                        created_button.set_label(if move_sources {
                            "Move here"
                        } else {
                            "Copy here"
                        });
                    }
                }
            });
        });
        submit_on_enter(&layout.body, &confirm);
        let escape = gtk::EventControllerKey::new();
        let escape_layer = layer.clone();
        let escape_overlay = window_overlay;
        let escape_root = blurred_root;
        let escape_creating = creating_destination;
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if escape_creating.get() {
                    return glib::Propagation::Stop;
                }
                dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);

        field.emit_by_name::<()>("changed", &[]);
        location_bar.focus_browse();
    }
}

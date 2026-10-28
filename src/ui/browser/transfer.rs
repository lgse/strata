// SPDX-License-Identifier: MIT

use crate::adapters::probes::{self, Presence};
use crate::adapters::{gio_file_for_location, transfer_targets};
use crate::app::Browser;
use crate::model::{FileEntry, Location};
use crate::services::{
    DropCommit, MoveRecord, PasteItem, ShownName, TransferConflict, UndoMoveItem, VolumeRelation,
    transferable_drop_sources,
};
use crate::ui::browser::ViewState;
use crate::ui::browser::entry::item_count_label;
use crate::ui::browser::paths::{can_remove_location, compact_display_path, is_trash_location};
use crate::ui::browser::progress::FileProgressState;
use crate::ui::browser::transfer_report::shown_name;
use crate::ui::controls::{
    ModalTone, focus_button, form_check_button, message_dialog_description, message_dialog_layout,
    stack_actions_when_constrained, wrap_button_label,
};
use crate::ui::keys::{
    self,
    bindings::{ENTER_ANY, ESCAPE_ANY},
};
use crate::ui::modal::{
    ModalHost, dismiss_modal_layer, modal_layer, remember_modal_focus_for_listing,
    show_error_dialog,
};
use futures_lite::future::BoxedLocal;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
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
    /// The name the paste will use, already held by the colliding item.
    target_name: Option<ShownName>,
    /// Both colliding items are directories, so their contents can be merged.
    mergeable: bool,
    fixes_inside: bool,
}

#[derive(Clone)]
pub(super) struct SendToTransferContext {
    pub(super) device_name: String,
}

struct SendToRoot {
    device_id: String,
    root: PathBuf,
    relative: Option<PathBuf>,
}

impl SendToRoot {
    fn unavailable(&self) -> String {
        crate::i18n::tr(if self.relative.is_some() {
            "The removable device or folder is no longer available."
        } else {
            "The removable device is no longer available."
        })
    }

    async fn destination(self) -> Result<Location, String> {
        let unavailable = self.unavailable();
        let Ok(Some(current_root)) = crate::adapters::probes::writable_folder(&self.root).await
        else {
            return Err(unavailable);
        };
        let Some(relative) = self.relative else {
            return Ok(Location::local(current_root));
        };
        let inside = gio::spawn_blocking(move || {
            let destination = crate::ui::browser::destination::canonical_directory_within(
                &current_root,
                &current_root.join(&relative),
            )?;
            let inside = destination.strip_prefix(&current_root).ok()?.to_owned();
            crate::ui::preferences::is_valid_send_to_relative_path(&inside).then_some((
                destination,
                inside,
                relative,
            ))
        })
        .await
        .ok()
        .flatten();
        let Some((destination, inside, relative)) = inside else {
            return Err(unavailable);
        };
        crate::ui::preferences::PreferenceManager::shared().remember_send_to_destination(
            &self.device_id,
            &inside,
            Some(&relative),
        );
        Ok(Location::local(destination))
    }
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ConflictFocus {
    #[default]
    Replace,
    KeepBoth,
}

#[derive(Clone, Copy, Default)]
struct ConflictActions {
    keep_both: bool,
    merge: bool,
    focus: ConflictFocus,
}

#[derive(PartialEq)]
pub(super) enum Preparing {
    Transfer(Location, Vec<Location>, bool),
    Replay { redo: bool, generation: u64 },
}

#[derive(Clone, Copy)]
struct GroupReplay {
    had_missing: bool,
    skipped: bool,
}

struct TransferDialogOptions {
    base: PathBuf,
    root_limit: Option<PathBuf>,
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

fn found(locations: Vec<Location>, presence: Vec<Presence>) -> Vec<Location> {
    locations
        .into_iter()
        .zip(presence)
        .filter(|(_, presence)| !matches!(presence, Presence::Absent))
        .map(|(location, _)| location)
        .collect()
}

fn preparing_row(item: &Location, destination: (String, String)) -> (String, (String, String)) {
    (item.display_name(), destination)
}

fn folder_row(item: &Location, destination: &Location) -> (String, (String, String)) {
    let folder = destination.parent().unwrap_or_else(|| destination.clone());
    preparing_row(item, crate::app::folder_destination_texts(&folder))
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

fn conflict_explanation(
    collision: &TransferCollision,
    destination: &Location,
    merge: bool,
) -> String {
    let own_name = collision.source.file_name();
    let target_name = collision
        .target_name
        .as_ref()
        .map(|shown| shown.name.clone())
        .or_else(|| own_name.clone())
        .unwrap_or_default();
    let name = shown_name(&target_name);
    let folder = compact_display_path(destination);
    let choice = if merge {
        let merging = rust_i18n::t!(
            "A folder named “%{name}” already exists in %{folder}. Merging combines the contents of both folders; incoming items overwrite items with the same name.",
            name = name,
            folder = folder
        );
        if collision.fixes_inside {
            crate::i18n::join_sentences(
                &merging,
                &crate::i18n::tr(
                    "Items whose names change to suit this location overwrite items with the new name.",
                ),
            )
        } else {
            merging.into_owned()
        }
    } else {
        rust_i18n::t!(
            "An item named “%{name}” already exists in %{folder}. Replacing it will overwrite its contents.",
            name = name,
            folder = folder
        )
        .into_owned()
    };
    match own_name.filter(|own_name| *own_name != target_name) {
        Some(own_name) => crate::i18n::join_sentences(
            &rust_i18n::t!(
                "“%{name}” will be saved as “%{new_name}” in this location.",
                name = shown_name(&own_name),
                new_name = name
            ),
            &choice,
        ),
        None => choice,
    }
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
                &crate::i18n::tr("Unable to transfer"),
                &crate::i18n::tr("The transfer could not be confirmed."),
            );
            return;
        };

        let count = sources.len();
        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            &crate::i18n::tr("Copy or move?"),
            &rust_i18n::t!(
                "%{items} to %{destination}",
                items = item_count_label(count),
                destination = compact_display_path(&destination)
            ),
            &crate::i18n::tr("Copy"),
            ModalTone::Accent,
        );
        layout
            .body
            .append(&message_dialog_description(&crate::i18n::tr(
                cross_volume_drop_description(volume),
            )));
        let move_button = gtk::Button::with_label(&crate::i18n::tr("Move"));
        move_button.add_css_class("action-dialog-cancel");
        layout
            .actions
            .insert_child_after(&move_button, Some(&layout.cancel));
        let content = layout.content;
        let cancel = layout.cancel;
        let copy = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        remember_modal_focus_for_listing(&layer, &window_overlay);
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
        keys::on_press(&escape, move |_, press| {
            if press.is(ESCAPE_ANY) {
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
        self.start_transfer_with_reveal(
            destination,
            sources,
            move_sources,
            true,
            None,
            ConflictFocus::Replace,
        );
    }

    pub(super) fn paste_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
        focus: ConflictFocus,
    ) {
        self.start_transfer_with_reveal(destination, sources, move_sources, true, None, focus);
    }

    pub(super) fn send_to_removable_device(self: &Rc<Self>, id: String, sources: Vec<Location>) {
        self.send_to_removable_device_with_resolver(
            &id,
            sources,
            crate::ui::removable_destination_root,
        );
    }

    pub(super) fn send_to_removable_device_with_resolver(
        self: &Rc<Self>,
        id: &str,
        sources: Vec<Location>,
        resolve: impl FnOnce(&str) -> Option<PathBuf>,
    ) {
        let Some(root) = resolve(id) else {
            show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Destination unavailable"),
                &crate::i18n::tr("The removable device is no longer available."),
            );
            return;
        };
        self.send_to_device(
            SendToRoot {
                device_id: id.to_owned(),
                root,
                relative: None,
            },
            sources,
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
            crate::ui::removable_destination_root,
        );
    }

    pub(super) fn send_to_recent_destination_with_resolver(
        self: &Rc<Self>,
        id: &str,
        relative: &Path,
        sources: Vec<Location>,
        resolve: impl FnOnce(&str) -> Option<PathBuf>,
    ) {
        let root = resolve(id)
            .filter(|_| crate::ui::preferences::is_valid_send_to_relative_path(relative));
        let Some(root) = root else {
            show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Destination unavailable"),
                &crate::i18n::tr("The removable device or folder is no longer available."),
            );
            return;
        };
        self.send_to_device(
            SendToRoot {
                device_id: id.to_owned(),
                root,
                relative: Some(relative.to_owned()),
            },
            sources,
        );
    }

    fn send_to_device(self: &Rc<Self>, device: SendToRoot, sources: Vec<Location>) {
        let destination = Location::local(match &device.relative {
            Some(relative) => device.root.join(relative),
            None => device.root.clone(),
        });
        let device_name = send_to_display_name(&device.device_id, &device.root);
        self.transfer_replaces_cursor.set(false);
        self.begin_transfer(
            destination,
            sources,
            false,
            false,
            (Some(SendToTransferContext { device_name }), Some(device)),
            ConflictFocus::Replace,
        );
    }

    pub(super) fn send_to(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        send_to: SendToTransferContext,
    ) {
        self.start_transfer_with_reveal(
            destination,
            sources,
            false,
            false,
            Some(send_to),
            ConflictFocus::Replace,
        );
    }

    fn send_to_success_text(device_name: &str, item_count: usize) -> String {
        if item_count == 1 {
            rust_i18n::t!("Copied to %{device_name}", device_name = device_name).into_owned()
        } else {
            rust_i18n::t!(
                "%{items} copied to %{device_name}",
                items = crate::i18n::count("items", item_count),
                device_name = device_name
            )
            .into_owned()
        }
    }

    /// Shows transient success feedback for a fast Send-to whose transfer
    /// finished before the normal progress UI became visible.
    pub(super) fn show_send_to_success(&self, device_name: &str, item_count: usize) {
        crate::ui::notice::show_notice(
            &self.overlay,
            None,
            &Self::send_to_success_text(device_name, item_count),
            SEND_TO_SUCCESS_DURATION,
        );
    }

    fn start_drop_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        let reveal = crate::ui::preferences::PreferenceManager::shared().open_folder_after_drop();
        self.start_transfer_with_reveal(
            destination,
            sources,
            move_sources,
            reveal,
            None,
            ConflictFocus::Replace,
        );
    }

    pub(super) fn start_transfer_in_place(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        self.transfer_replaces_cursor.set(true);
        self.begin_transfer(
            destination,
            sources,
            move_sources,
            false,
            (None, None),
            ConflictFocus::KeepBoth,
        );
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
        focus: ConflictFocus,
    ) {
        self.transfer_replaces_cursor.set(false);
        self.begin_transfer(
            destination,
            sources,
            move_sources,
            reveal,
            (send_to, None),
            focus,
        );
    }

    fn begin_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
        reveal: bool,
        (send_to, device): (Option<SendToTransferContext>, Option<SendToRoot>),
        focus: ConflictFocus,
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
        let request = Preparing::Transfer(destination.clone(), sources.clone(), move_sources);
        if self
            .preparing_transfers
            .borrow()
            .iter()
            .any(|(preparing, cancellable, _)| *preparing == request && !cancellable.is_cancelled())
        {
            return;
        }
        let cancellable = gio::Cancellable::new();
        let row = (
            sources
                .first()
                .map(Location::display_name)
                .unwrap_or_default(),
            crate::app::folder_destination_texts(&destination),
        );
        let preparation = self.show_transfer_preparation(move_sources, row, &cancellable);
        self.preparing_transfers.borrow_mut().push((
            request,
            cancellable.clone(),
            Rc::downgrade(&preparation),
        ));
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let prepared = match device {
                Some(device) => device.destination().await,
                None => Ok(destination),
            };
            let prepared = match prepared {
                Ok(destination) => {
                    transfer_targets(&destination, &sources, move_sources, &cancellable)
                        .await
                        .map(|targets| (destination, targets))
                        .map_err(|error| {
                            (
                                crate::i18n::tr("Unable to transfer"),
                                error.message().to_owned(),
                            )
                        })
                }
                Err(unavailable) => Err((crate::i18n::tr("Destination unavailable"), unavailable)),
            };
            let Some(state) = weak.upgrade() else {
                preparation.dismiss_file_operation_progress();
                return;
            };
            state
                .preparing_transfers
                .borrow_mut()
                .retain(|(_, preparing, _)| *preparing != cancellable);
            if cancellable.is_cancelled() {
                preparation.dismiss_file_operation_progress();
                return;
            }
            let (destination, targets) = match prepared {
                Ok(prepared) => prepared,
                Err((title, message)) => {
                    preparation.dismiss_file_operation_progress();
                    show_error_dialog(&state.overlay, &title, &message);
                    return;
                }
            };
            let mut accepted = Vec::new();
            let mut collisions = Vec::new();
            for (source, target) in sources.into_iter().zip(targets) {
                match target.collision {
                    Some(collision) => collisions.push(TransferCollision {
                        source,
                        target_name: target.name,
                        mergeable: collision.mergeable,
                        fixes_inside: target.fixes_inside,
                    }),
                    None => accepted.push(PasteItem {
                        source,
                        conflict: TransferConflict::FailIfExists,
                        target_name: target.name,
                    }),
                }
            }
            if collisions.is_empty() {
                state.transfer_preparation.replace(Some(preparation));
            } else {
                preparation.dismiss_file_operation_progress();
            }
            state.resolve_transfer_collisions(
                destination,
                collisions,
                accepted,
                move_sources,
                reveal,
                send_to,
                focus,
            );
            if let Some(preparation) = state.transfer_preparation.take() {
                preparation.dismiss_file_operation_progress();
            }
        });
    }

    fn show_transfer_preparation(
        &self,
        moving: bool,
        row: (String, (String, String)),
        cancellable: &gio::Cancellable,
    ) -> Rc<FileProgressState> {
        let progress = Rc::new(FileProgressState::new(&self.overlay));
        let shown = Rc::downgrade(&progress);
        let cancellable = cancellable.clone();
        progress.show_transfer_preparation(
            moving,
            row,
            Rc::new(move || {
                cancellable.cancel();
                if let Some(progress) = shown.upgrade() {
                    progress.dismiss_file_operation_progress();
                }
            }),
        );
        progress
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "each prompt carries the whole paste forward to resolve the next collision"
    )]
    fn resolve_transfer_collisions(
        self: &Rc<Self>,
        destination: Location,
        mut collisions: Vec<TransferCollision>,
        accepted: Vec<PasteItem>,
        move_sources: bool,
        reveal: bool,
        send_to: Option<SendToTransferContext>,
        focus: ConflictFocus,
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
                // Parked jobs own their completion state independently; clear only
                // stale foreground metadata before this dispatch.
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
            if self.transfer_replaces_cursor.get() {
                self.browser
                    .transfer_replacing_cursor(destination, accepted, move_sources, reveal);
            } else {
                self.browser
                    .transfer(destination, accepted, move_sources, reveal);
            }
            return;
        }
        let collision = collisions.remove(0);
        let source = collision.source.clone();
        let target_name = collision.target_name.clone();
        let name = source.display_name();
        // Merge stays copy-only: undoing a merged move cannot tell which
        // destination contents the source actually owned.
        let allow_merge = collision.mergeable && !move_sources;
        let explanation = conflict_explanation(&collision, &destination, allow_merge);
        let without_trash = destination
            .native_path()
            .and_then(crate::adapters::trash_support::unsupported_reason_for)
            .is_some();
        let explanation = if without_trash {
            format!(
                "{explanation}\n\n{}",
                crate::i18n::tr("Replaced items can't be moved to Trash here and will be deleted.")
            )
        } else {
            explanation
        };
        let state = self.clone();
        let send_to_conflict = send_to.clone();
        let apply_to_all_visible = !collisions.is_empty();
        let skip_visible = !accepted.is_empty() || !collisions.is_empty();
        self.confirm_replace_conflict(
            &name,
            &explanation,
            apply_to_all_visible,
            skip_visible,
            ConflictActions {
                keep_both: true,
                merge: allow_merge,
                focus,
            },
            Rc::new(move |choice, apply_to_all| {
                let mut accepted = accepted.clone();
                let mut remaining = collisions.clone();
                match choice {
                    ConflictChoice::Replace => {
                        accepted.push(PasteItem {
                            source: source.clone(),
                            conflict: TransferConflict::ReplaceExisting,
                            target_name: target_name.clone(),
                        });
                        if apply_to_all {
                            accepted.extend(remaining.drain(..).map(|collision| PasteItem {
                                source: collision.source,
                                conflict: TransferConflict::ReplaceExisting,
                                target_name: collision.target_name,
                            }));
                        }
                    }
                    ConflictChoice::Merge => {
                        accepted.push(PasteItem {
                            source: source.clone(),
                            conflict: TransferConflict::Merge,
                            target_name: target_name.clone(),
                        });
                        if apply_to_all {
                            // File collisions still need their own prompt.
                            let (mergeable, rest): (Vec<_>, Vec<_>) = remaining
                                .into_iter()
                                .partition(|collision| collision.mergeable);
                            accepted.extend(mergeable.into_iter().map(|collision| PasteItem {
                                source: collision.source,
                                conflict: TransferConflict::Merge,
                                target_name: collision.target_name,
                            }));
                            remaining = rest;
                        }
                    }
                    ConflictChoice::KeepBoth => {
                        accepted.push(PasteItem {
                            source: source.clone(),
                            conflict: TransferConflict::KeepBoth,
                            target_name: target_name.clone(),
                        });
                        if apply_to_all {
                            accepted.extend(remaining.drain(..).map(|collision| PasteItem {
                                source: collision.source,
                                conflict: TransferConflict::KeepBoth,
                                target_name: collision.target_name,
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
                    focus,
                );
            }),
        );
    }

    /// Moves the latest completed transfer back, confirming any item that would
    /// overwrite something created since the move.
    pub(super) fn undo_move(self: &Rc<Self>, generation: u64, records: Vec<MoveRecord>) -> bool {
        self.replay_move(false, generation, records, false)
    }

    pub(super) fn undo_group(self: &Rc<Self>, generation: u64, records: Vec<MoveRecord>) -> bool {
        self.replay_move(false, generation, records, true)
    }

    pub(super) fn redo_trash(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        self.replay_existing_locations(true, generation, locations, Browser::redo_trash)
    }

    pub(super) fn redo_move(self: &Rc<Self>, generation: u64, records: Vec<MoveRecord>) -> bool {
        self.replay_move(true, generation, records, false)
    }

    fn check_replay(
        self: &Rc<Self>,
        (redo, generation): (bool, u64),
        row: (String, (String, String)),
        locations: Vec<Location>,
        resume: impl FnOnce(&Rc<Self>, Vec<Presence>) + 'static,
    ) -> bool {
        let replay = Preparing::Replay { redo, generation };
        if self
            .preparing_transfers
            .borrow()
            .iter()
            .any(|(preparing, cancellable, _)| *preparing == replay && !cancellable.is_cancelled())
        {
            return true;
        }
        let cancellable = gio::Cancellable::new();
        let preparation = self.show_transfer_preparation(true, row, &cancellable);
        self.preparing_transfers.borrow_mut().push((
            replay,
            cancellable.clone(),
            Rc::downgrade(&preparation),
        ));
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let presence = probes::presence(&locations, &cancellable).await;
            let Some(state) = weak.upgrade() else {
                preparation.dismiss_file_operation_progress();
                return;
            };
            state
                .preparing_transfers
                .borrow_mut()
                .retain(|(_, preparing, _)| *preparing != cancellable);
            if cancellable.is_cancelled() {
                preparation.dismiss_file_operation_progress();
                return;
            }
            state.transfer_preparation.replace(Some(preparation));
            resume(&state, presence);
            if let Some(preparation) = state.transfer_preparation.take() {
                preparation.dismiss_file_operation_progress();
            }
        });
        true
    }

    fn replay_existing_locations(
        self: &Rc<Self>,
        redo: bool,
        generation: u64,
        locations: Vec<Location>,
        dispatch: fn(&Rc<Browser>, u64, Vec<Location>) -> bool,
    ) -> bool {
        let Some(first) = locations.first() else {
            self.browser.discard_pending_replay(redo, generation);
            return false;
        };
        let row = preparing_row(first, crate::app::trash_destination_texts());
        self.check_replay(
            (redo, generation),
            row,
            locations.clone(),
            move |state, presence| {
                let existing = found(locations, presence);
                if existing.is_empty() {
                    state.browser.discard_pending_replay(redo, generation);
                } else {
                    dispatch(&state.browser, generation, existing);
                }
            },
        )
    }

    fn replay_move(
        self: &Rc<Self>,
        redo: bool,
        generation: u64,
        records: Vec<MoveRecord>,
        grouped: bool,
    ) -> bool {
        let ends = |record: &MoveRecord| {
            if redo {
                (record.original.clone(), record.current.clone())
            } else {
                (record.current.clone(), record.original.clone())
            }
        };
        let Some((item, destination)) = records.first().map(ends) else {
            self.browser.discard_pending_replay(redo, generation);
            return false;
        };
        let locations = records
            .iter()
            .flat_map(|record| <[Location; 2]>::from(ends(record)))
            .collect();
        self.check_replay(
            (redo, generation),
            folder_row(&item, &destination),
            locations,
            move |state, presence| {
                let mut accepted = Vec::new();
                let mut collisions = Vec::new();
                let mut missing = 0;
                for (record, ends) in records.into_iter().zip(presence.as_chunks::<2>().0) {
                    match ends {
                        [Presence::Absent, _] => missing += 1,
                        [_, Presence::Present] => collisions.push(record),
                        _ => accepted.push(UndoMoveItem {
                            record,
                            conflict: TransferConflict::FailIfExists,
                        }),
                    }
                }
                if accepted.is_empty() && collisions.is_empty() {
                    if grouped && !redo && missing > 0 {
                        state.browser.discard_group_keep_folder(redo, generation);
                    } else {
                        state.browser.discard_pending_replay(redo, generation);
                    }
                    return;
                }
                if !collisions.is_empty()
                    && let Some(preparation) = state.transfer_preparation.take()
                {
                    preparation.dismiss_file_operation_progress();
                }
                state.resolve_replay_collisions(
                    redo,
                    generation,
                    collisions,
                    accepted,
                    grouped.then_some(GroupReplay {
                        had_missing: missing > 0,
                        skipped: false,
                    }),
                );
            },
        )
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
        let Some(first) = created.first() else {
            if overwritten.is_empty() {
                self.browser.discard_pending_replay(false, generation);
                return false;
            }
            return self.browser.undo_merge(generation, created, overwritten);
        };
        let row = folder_row(first, first);
        self.check_replay(
            (false, generation),
            row,
            created.clone(),
            move |state, presence| {
                let created = found(created, presence);
                // Overwritten entries stay even when the incoming copy is gone: the
                // staged original may still be sitting in Trash waiting to restore.
                if created.is_empty() && overwritten.is_empty() {
                    state.browser.discard_pending_replay(false, generation);
                } else {
                    state.browser.undo_merge(generation, created, overwritten);
                }
            },
        )
    }

    fn resolve_replay_collisions(
        self: &Rc<Self>,
        redo: bool,
        generation: u64,
        mut collisions: Vec<MoveRecord>,
        accepted: Vec<UndoMoveItem>,
        group: Option<GroupReplay>,
    ) {
        if collisions.is_empty() {
            if accepted.is_empty() {
                self.browser.discard_pending_replay(redo, generation);
            } else if let Some(state) = group {
                if state.skipped {
                    self.browser.undo_group(generation, accepted);
                } else if state.had_missing {
                    self.browser
                        .undo_group_retaining_folder(generation, accepted);
                } else {
                    self.browser.undo_group(generation, accepted);
                }
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
        let folder = compact_display_path(&parent);
        let explanation = if redo {
            rust_i18n::t!(
                "An item named “%{name}” already exists in %{folder}. Redoing the move will overwrite its contents.",
                name = name,
                folder = folder
            )
        } else {
            rust_i18n::t!(
                "An item named “%{name}” already exists in %{folder}. Undoing the move will overwrite its contents.",
                name = name,
                folder = folder
            )
        };
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
                let mut group = group;
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
                    ConflictChoice::Skip => {
                        if apply_to_all {
                            remaining.clear();
                        }
                        if let Some(group) = group.as_mut() {
                            group.skipped = true;
                        }
                    }
                }
                state.resolve_replay_collisions(redo, generation, remaining, accepted, group);
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
            &crate::i18n::tr("File already exists"),
            name,
            &crate::i18n::tr("Replace"),
            ModalTone::Danger,
        );
        layout.body.append(&message_dialog_description(explanation));
        let apply_all = form_check_button(&crate::i18n::tr("Apply to All"));
        apply_all.set_visible(apply_to_all_visible);
        layout.body.append(&apply_all);
        let skip = gtk::Button::with_label(&crate::i18n::tr("Skip"));
        skip.add_css_class("action-dialog-cancel");
        skip.set_visible(skip_visible);
        layout
            .actions
            .insert_child_after(&skip, Some(&layout.cancel));
        let keep_both = gtk::Button::with_label(&crate::i18n::tr("Keep Both"));
        keep_both.add_css_class("action-dialog-cancel");
        keep_both.set_visible(actions.keep_both);
        layout.actions.insert_child_after(&keep_both, Some(&skip));
        let merge = gtk::Button::with_label(&crate::i18n::tr("Merge"));
        merge.add_css_class("action-dialog-cancel");
        merge.set_visible(actions.merge);
        layout.actions.insert_child_after(&merge, Some(&keep_both));
        for button in [&layout.cancel, &skip, &keep_both, &merge, &layout.confirm] {
            wrap_button_label(button);
        }
        stack_actions_when_constrained(&layout);
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
        let keep_both_focus = keep_both.clone();
        let enter_buttons = [
            skip,
            keep_both,
            merge,
            replace.clone(),
            cancel,
            layout.close,
        ];
        keys::on_press(&escape, move |_, press| {
            if press.is(ESCAPE_ANY) {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else if press.is(ENTER_ANY) {
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
        if actions.focus == ConflictFocus::KeepBoth && actions.keep_both {
            focus_button(&keep_both_focus);
        } else {
            focus_button(&replace);
        }
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
                root_limit: None,
                allow_create: true,
                completion: TransferDialogCompletion::CopyMove { move_sources },
            },
        );
    }

    pub(super) fn show_send_to_folder_dialog(self: &Rc<Self>, id: String, sources: Vec<Location>) {
        self.show_send_to_folder_dialog_with_resolver(
            id,
            sources,
            Rc::new(crate::ui::removable_destination_root),
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
        let root = resolve(&id);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let root = match root {
                Some(root) => crate::adapters::probes::writable_folder(&root)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            let Some(state) = weak.upgrade() else {
                return;
            };
            let Some(root) = root else {
                show_error_dialog(
                    &state.overlay,
                    &crate::i18n::tr("Destination unavailable"),
                    &crate::i18n::tr("The removable device is no longer available."),
                );
                return;
            };
            state.show_destination_dialog(
                sources,
                TransferDialogOptions {
                    base: root.clone(),
                    root_limit: Some(root.clone()),
                    allow_create: false,
                    completion: TransferDialogCompletion::SendTo {
                        device_id: id,
                        opened_root: root,
                        resolve,
                    },
                },
            );
        });
    }

    fn show_destination_dialog(
        self: &Rc<Self>,
        sources: Vec<Location>,
        options: TransferDialogOptions,
    ) {
        let Some(parent) = self.overlay.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let TransferDialogOptions {
            base,
            root_limit,
            allow_create,
            completion,
            ..
        } = options;
        let move_sources = matches!(
            completion,
            TransferDialogCompletion::CopyMove { move_sources: true }
        );
        let title = match &completion {
            TransferDialogCompletion::SendTo { .. } => crate::i18n::tr("Send to folder"),
            _ if move_sources => crate::i18n::tr("Move to"),
            _ => crate::i18n::tr("Copy to"),
        };
        let validation_completion = completion.clone();
        let validated_root = Rc::new(RefCell::new(None));
        let validated = validated_root.clone();
        let validate = Rc::new(move |path: &Path| -> BoxedLocal<Result<PathBuf, String>> {
            let TransferDialogCompletion::SendTo {
                device_id,
                opened_root,
                resolve,
            } = &validation_completion
            else {
                return crate::ui::browser::destination::existing_folder(path);
            };
            let root = resolve(device_id);
            let relative = path.strip_prefix(opened_root).map(Path::to_path_buf);
            let validated = validated.clone();
            Box::pin(async move {
                let unavailable =
                    || crate::i18n::tr("The removable device is no longer available.");
                let root = root.ok_or_else(unavailable)?;
                let current_root = crate::adapters::probes::writable_folder(&root)
                    .await
                    .ok()
                    .flatten()
                    .ok_or_else(unavailable)?;
                let outside =
                    || crate::i18n::tr("Choose an existing folder inside this removable device.");
                let relative = relative.map_err(|_| outside())?;
                let destination =
                    crate::adapters::probes::destination_folder(&current_root.join(relative))
                        .await
                        .ok()
                        .filter(|destination| destination.starts_with(&current_root))
                        .ok_or_else(outside)?;
                validated.replace(Some(current_root));
                Ok(destination)
            })
        });
        let state = self.clone();
        crate::ui::chooser::present_destination_chooser(
            crate::ui::chooser::DestinationRequest {
                parent,
                title,
                accept_label: crate::i18n::tr(if move_sources {
                    "Move here"
                } else {
                    "Copy here"
                }),
                initial_directory: base,
                root_limit,
                allow_create,
                validate,
            },
            move |path| {
                if let TransferDialogCompletion::SendTo {
                    device_id,
                    opened_root,
                    ..
                } = &completion
                {
                    let Some(current_root) = validated_root.take() else {
                        show_error_dialog(
                            &state.overlay,
                            &crate::i18n::tr("Destination unavailable"),
                            &crate::i18n::tr("The removable device is no longer available."),
                        );
                        return;
                    };
                    if let Ok(relative) = path.strip_prefix(&current_root) {
                        crate::ui::preferences::PreferenceManager::shared()
                            .remember_send_to_destination(device_id, relative, None);
                    }
                    state.send_to(
                        Location::local(path),
                        sources,
                        SendToTransferContext {
                            device_name: send_to_display_name(device_id, opened_root),
                        },
                    );
                } else {
                    state.pending_navigate.replace(Some(Location::local(&path)));
                    let names = sources.iter().filter_map(|source| {
                        source
                            .native_path()?
                            .file_name()?
                            .to_str()
                            .map(String::from)
                    });
                    state.pending_select.borrow_mut().extend(names);
                    state.start_transfer(Location::local(path), sources, move_sources);
                }
            },
        );
    }
}

// SPDX-License-Identifier: MIT

use crate::adapters::{TransferTargetCollision, gio_file_for_location, transfer_targets};
use crate::model::{FileEntry, Location};
use crate::services::{PasteItem, ShownName, TransferConflict};
use crate::ui::blur::BlurBin;
use crate::ui::browser::ViewState;
use crate::ui::browser::paths::{can_remove_location, is_trash_location};
use crate::ui::browser::progress::FileProgressState;
use crate::ui::keys::{self, bindings::ESCAPE_ANY};
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::path::PathBuf;
use std::rc::Rc;

mod collisions;
mod conflict_dialog;
mod destination_dialog;
mod drop;
mod replay;
mod send_to;
use collisions::{TransferCollision, TransferPlan};
use send_to::SendToRoot;
pub(super) use send_to::{
    FinishedSendToCompletion, PendingSendToCompletion, SendToTransferContext,
};
#[cfg(test)]
use {
    crate::services::VolumeRelation, collisions::conflict_explanation,
    drop::cross_volume_drop_description,
};

type RemovableRootResolver = Rc<dyn Fn(&str) -> Option<PathBuf>>;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ConflictFocus {
    #[default]
    Replace,
    KeepBoth,
}

#[derive(PartialEq)]
pub(super) enum Preparing {
    Transfer(Location, Vec<Location>, bool),
    Replay { redo: bool, generation: u64 },
}

/// A copy or move as requested, before its sources are filtered and checked.
pub(super) struct TransferRequest {
    pub(super) destination: Location,
    pub(super) sources: Vec<Location>,
    pub(super) move_sources: bool,
    pub(super) reveal: bool,
    pub(super) send_to: Option<SendToTransferContext>,
    pub(super) focus: ConflictFocus,
}

impl TransferRequest {
    fn refused(&self) -> bool {
        let unremovable = || {
            self.sources
                .iter()
                .any(|source| !can_remove_location(source))
        };
        is_trash_location(&self.destination)
            || self.destination.is_recent_location()
            || self.destination.is_computer_root()
            || (self.move_sources && unremovable())
    }
}

/// A presented dialog's layer and the overlay it was added to.
#[derive(Clone)]
struct DialogLayer {
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<BlurBin>,
}

impl DialogLayer {
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

    fn dismiss_on_click(&self, button: &gtk::Button) {
        let modal = self.clone();
        button.connect_clicked(move |_| modal.dismiss());
    }

    fn dismiss_on_escape(&self) {
        let escape = gtk::EventControllerKey::new();
        escape.set_propagation_phase(gtk::PropagationPhase::Capture);
        let modal = self.clone();
        keys::on_press(&escape, move |_, press| {
            if press.is(ESCAPE_ANY) {
                modal.dismiss();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.layer.add_controller(escape);
    }
}

fn found(
    locations: Vec<Location>,
    presence: Vec<crate::adapters::probes::Presence>,
) -> Vec<Location> {
    locations
        .into_iter()
        .zip(presence)
        .filter(|(_, presence)| !matches!(presence, crate::adapters::probes::Presence::Absent))
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
    let moves_into_own_parent = || {
        source
            .parent()
            .is_some_and(|parent| parent.equal(&destination))
    };
    source.equal(&destination)
        || destination.has_prefix(&source)
        || (move_sources && moves_into_own_parent())
}

pub(super) fn duplicate_transfer(entries: &[FileEntry]) -> Option<(Location, Vec<Location>)> {
    if entries.iter().any(FileEntry::is_computer_item) {
        return None;
    }
    let destination = entries.first()?.location.parent()?;
    let shares_parent = entries
        .iter()
        .all(|entry| entry.location.parent().as_ref() == Some(&destination));
    if is_trash_location(&destination)
        || destination.is_recent_location()
        || destination.is_computer_root()
        || !shares_parent
    {
        return None;
    }
    let sources = entries.iter().map(|entry| entry.location.clone()).collect();
    Some((destination, sources))
}

/// What preparing a transfer found at the destination.
struct PreparedTransfer {
    destination: Location,
    accepted: Vec<PasteItem>,
    collisions: Vec<TransferCollision>,
}

/// One source's target as `(name, collision, fixes inside)`.
type TargetFacts = (Option<ShownName>, Option<TransferTargetCollision>, bool);

/// Splits prepared targets into items that go straight through and those that collide.
fn sort_transfer_targets(
    sources: Vec<Location>,
    targets: impl IntoIterator<Item = TargetFacts>,
) -> (Vec<PasteItem>, Vec<TransferCollision>) {
    let mut accepted = Vec::new();
    let mut collisions = Vec::new();
    for (source, (target_name, collision, fixes_inside)) in sources.into_iter().zip(targets) {
        match collision {
            Some(collision) => collisions.push(TransferCollision {
                source,
                target_name,
                kinds: collision,
                fixes_inside,
            }),
            None => accepted.push(PasteItem {
                source,
                conflict: TransferConflict::FailIfExists,
                target_name,
            }),
        }
    }
    (accepted, collisions)
}

async fn prepare_transfer(
    (device, destination): (Option<SendToRoot>, Location),
    sources: Vec<Location>,
    move_sources: bool,
    cancellable: &gio::Cancellable,
) -> Result<PreparedTransfer, (String, String)> {
    let prepared = match device {
        Some(device) => device.destination().await,
        None => Ok(destination),
    };
    let destination = prepared
        .map_err(|unavailable| (crate::i18n::tr("Destination unavailable"), unavailable))?;
    let targets = transfer_targets(&destination, &sources, move_sources, cancellable)
        .await
        .map_err(|error| {
            (
                crate::i18n::tr("Unable to transfer"),
                error.message().to_owned(),
            )
        })?;
    let facts = targets
        .into_iter()
        .map(|target| (target.name, target.collision, target.fixes_inside));
    let (accepted, collisions) = sort_transfer_targets(sources, facts);
    Ok(PreparedTransfer {
        destination,
        accepted,
        collisions,
    })
}

impl ViewState {
    pub(super) fn start_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        self.paste_transfer(destination, sources, move_sources, ConflictFocus::Replace);
    }

    pub(super) fn paste_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
        focus: ConflictFocus,
    ) {
        self.start_transfer_with_reveal(TransferRequest {
            destination,
            sources,
            move_sources,
            reveal: true,
            send_to: None,
            focus,
        });
    }

    pub(super) fn start_transfer_in_place(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        self.transfer_replaces_cursor.set(true);
        self.begin_transfer(
            TransferRequest {
                destination,
                sources,
                move_sources,
                reveal: false,
                send_to: None,
                focus: ConflictFocus::KeepBoth,
            },
            None,
        );
    }

    /// Paste and explicit "move/copy to" reveal their result independently of
    /// the drop preference, which is captured when the transfer starts.
    pub(super) fn start_transfer_with_reveal(self: &Rc<Self>, request: TransferRequest) {
        self.transfer_replaces_cursor.set(false);
        self.begin_transfer(request, None);
    }

    fn transfer_in_progress(&self, request: &Preparing) -> bool {
        self.preparing_transfers
            .borrow()
            .iter()
            .any(|(preparing, cancellable, _)| preparing == request && !cancellable.is_cancelled())
    }

    fn begin_transfer(self: &Rc<Self>, request: TransferRequest, device: Option<SendToRoot>) {
        if request.refused() {
            return;
        }
        let TransferRequest {
            destination,
            sources,
            move_sources,
            reveal,
            send_to,
            focus,
        } = request;
        let sources: Vec<Location> = sources
            .into_iter()
            .filter(|source| !transfer_is_noop(source, &destination, move_sources))
            .collect();
        if sources.is_empty() {
            return;
        }
        let preparing = Preparing::Transfer(destination.clone(), sources.clone(), move_sources);
        if self.transfer_in_progress(&preparing) {
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
            preparing,
            cancellable.clone(),
            Rc::downgrade(&preparation),
        ));
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let prepared =
                prepare_transfer((device, destination), sources, move_sources, &cancellable).await;
            let Some(state) = weak.upgrade() else {
                preparation.dismiss_file_operation_progress();
                return;
            };
            if !state.finish_preparing(&cancellable, &preparation) {
                return;
            }
            let PreparedTransfer {
                destination,
                accepted,
                collisions,
            } = match prepared {
                Ok(prepared) => prepared,
                Err((title, message)) => {
                    preparation.dismiss_file_operation_progress();
                    show_error_dialog(&state.overlay, &title, &message);
                    return;
                }
            };
            if collisions.is_empty() {
                state.transfer_preparation.replace(Some(preparation));
            } else {
                preparation.dismiss_file_operation_progress();
            }
            let plan = TransferPlan {
                destination,
                move_sources,
                reveal,
                send_to,
                focus,
            };
            state.resolve_transfer_collisions(plan, collisions, accepted);
            state.dismiss_transfer_preparation();
        });
    }

    /// Forgets a finished preparation; a cancelled one is dismissed and reports `false`.
    fn finish_preparing(
        &self,
        cancellable: &gio::Cancellable,
        preparation: &FileProgressState,
    ) -> bool {
        self.preparing_transfers
            .borrow_mut()
            .retain(|(_, preparing, _)| preparing != cancellable);
        if cancellable.is_cancelled() {
            preparation.dismiss_file_operation_progress();
            return false;
        }
        true
    }

    fn dismiss_transfer_preparation(&self) {
        if let Some(preparation) = self.transfer_preparation.take() {
            preparation.dismiss_file_operation_progress();
        }
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
}

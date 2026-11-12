// SPDX-License-Identifier: MIT

use crate::{
    model::Location,
    services::{LoadHandle, OperationRequestId},
};

use super::undo::{MergeUndoState, finish_redo, finish_undo};
use super::{Browser, BrowserEvent};

pub(super) enum OperationDestination<'a> {
    Folder(&'a Location),
    Trash,
    PermanentDeletion,
    /// A network or cloud provider may still keep a copy.
    DeletionWithoutTrash,
}

pub fn folder_destination_texts(location: &Location) -> (String, String) {
    let path = location.display_path();
    (
        rust_i18n::t!("Destination: %{path}", path = path).into_owned(),
        format!("→ {path}"),
    )
}

pub fn trash_destination_texts() -> (String, String) {
    (
        crate::i18n::tr("Destination: Trash"),
        format!("→ {}", crate::i18n::tr("Trash")),
    )
}

impl Browser {
    pub fn cancel_file_operation(&self) {
        self.operation_cancel_requested
            .set(self.current_operation.get().is_some());
        if self.current_operation.get().is_some() && self.transfer_operation.get().is_some() {
            self.transfer_cancel_pending.set(true);
        }
        let load = self.operation_load.take();
        drop(load);
    }

    pub(super) fn set_operation_destination(&self, destination: OperationDestination<'_>) {
        let (description, label) = match destination {
            OperationDestination::Folder(location) => folder_destination_texts(location),
            OperationDestination::Trash => trash_destination_texts(),
            OperationDestination::PermanentDeletion => {
                let text = crate::i18n::tr("Permanent deletion");
                (text.clone(), text)
            }
            OperationDestination::DeletionWithoutTrash => {
                let text = crate::i18n::tr("Deletion without Trash");
                (text.clone(), text)
            }
        };
        self.operation_destination_description.replace(description);
        self.operation_destination_label.replace(label);
    }

    pub fn operation_destination_label(&self) -> String {
        self.operation_destination_label.borrow().clone()
    }

    pub(super) fn try_begin_operation(&self) -> Option<OperationRequestId> {
        if self.cancellation_pending() {
            self.emit(BrowserEvent::TransferCancellationPending);
            return None;
        }
        Some(self.begin_operation())
    }

    /// Whether a file operation requested now would start, without reporting why not.
    pub fn can_start_operation(&self) -> bool {
        self.operation_provider.borrow().is_some() && !self.cancellation_pending()
    }

    fn cancellation_pending(&self) -> bool {
        self.transfer_cancel_pending.get()
            || self
                .background_operations
                .borrow()
                .values()
                .any(|job| job.cancel_pending.get())
    }

    pub fn is_current_operation(&self, request_id: OperationRequestId) -> bool {
        self.current_operation.get() == Some(request_id)
    }

    pub fn last_started_operation(&self) -> Option<OperationRequestId> {
        self.last_started_operation.get()
    }

    pub(super) fn begin_operation(&self) -> OperationRequestId {
        let request_id = OperationRequestId(self.next_request.get());
        self.next_request
            .set(self.next_request.get().saturating_add(1));
        self.last_started_operation.set(Some(request_id));
        let previous_operation = self.current_operation.take();
        let previous_rename = self.rename_operation.take();
        self.operation_load.borrow_mut().take();
        if previous_operation == previous_rename
            && let Some(request_id) = previous_rename
        {
            self.emit(BrowserEvent::RenameAbandoned { request_id });
        }
        if let Some((generation, _)) = self.undo_claim.take() {
            finish_undo(generation, false);
        }
        if let Some((generation, _)) = self.redo_claim.take() {
            finish_redo(generation, false);
        }
        self.transfer_operation.set(None);
        self.transfer_destination.replace(None);
        self.created_locations.borrow_mut().clear();
        *self.merged_undo.borrow_mut() = MergeUndoState::default();
        self.moved_items.take();
        self.transfer_report.take();
        self.deletion_operation.set(false);
        self.deletion_permanent.set(false);
        self.deferred_file_operation_changes.borrow_mut().clear();
        self.operation_rescan_depths.borrow_mut().clear();
        self.restoration_operation.set(false);
        self.archive_operation.set(false);
        self.operation_backgroundable.set(false);
        self.operation_cancel_requested.set(false);
        self.operation_description.borrow_mut().clear();
        self.operation_destination_description.borrow_mut().clear();
        self.operation_destination_label.borrow_mut().clear();
        self.current_operation.set(Some(request_id));
        request_id
    }

    pub(super) fn install_operation_load(&self, request_id: OperationRequestId, load: LoadHandle) {
        if self.is_current_operation(request_id) {
            if !self.operation_cancel_requested.get() {
                self.operation_load.replace(Some(load));
            }
        } else {
            let background = self
                .background_operations
                .borrow()
                .get(&request_id)
                .cloned();
            if let Some(job) = background
                && !job.cancel_requested.get()
            {
                job.load.replace(Some(load));
            }
        }
    }
}

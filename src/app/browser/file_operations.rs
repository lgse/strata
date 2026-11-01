// SPDX-License-Identifier: MIT

use std::{collections::HashSet, rc::Rc};

use crate::{
    model::{FileEntry, Location},
    services::{
        ArchiveFormat, CompressRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest,
        ExtractRequest, MoveRecord, OperationEvent, OperationRequestId, PasteItem, PasteRequest,
        PlaceEntryRequest, RenameRecord, RenameRequest, RestoreRequest, RestoreSource,
        RestoreTrashItem, TransferConflict, TransferReport, TypedPath, validate_basename,
    },
};

use super::operation_start::OperationDestination;
use super::undo::{UndoEntry, push_pending_undo, retarget_created_undo};
use super::{Browser, BrowserEvent};

impl Browser {
    pub fn rename(
        self: &Rc<Self>,
        entry: FileEntry,
        new_name: String,
    ) -> Option<OperationRequestId> {
        if let Err(message) = validate_basename(&new_name) {
            self.emit(BrowserEvent::RenameFailed {
                request_id: None,
                message: crate::i18n::tr(message),
            });
            return None;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::RenameFailed {
                request_id: None,
                message: crate::i18n::tr("File operations are unavailable"),
            });
            return None;
        };
        let request_id = self.try_begin_operation()?;
        self.rename_operation.set(Some(request_id));
        let refresh_locations = entry.location.parent().into_iter().collect();
        let emit = self.operation_callback(request_id, true, refresh_locations);
        let mut renamed = entry.clone();
        let new_location = entry
            .location
            .parent()
            .and_then(|parent| parent.child(std::ffi::OsStr::new(&new_name)));
        renamed.native_name = std::ffi::OsString::from(&new_name);
        renamed.display_name = new_name.clone();
        renamed.is_hidden = new_name.starts_with('.');
        let old_location = entry.location.clone();
        let original_native_name = entry.native_name.clone();
        let original_display_name = entry.display_name.clone();
        let original_is_hidden = entry.is_hidden;
        let weak = Rc::downgrade(self);
        let publish = Rc::new(move |event: OperationEvent| {
            if matches!(&event, OperationEvent::Renamed { request_id: id } if *id == request_id)
                && let Some(browser) = weak.upgrade()
                && let Some(location) = new_location.as_ref()
                && location != &old_location
            {
                browser.publish_relocation(&old_location, location);
                if browser.is_current_operation(request_id) {
                    push_pending_undo(UndoEntry::Rename(RenameRecord {
                        original: old_location.clone(),
                        current: location.clone(),
                        native_name: original_native_name.clone(),
                        display_name: original_display_name.clone(),
                        is_hidden: original_is_hidden,
                    }));
                    let mut renamed = renamed.clone();
                    renamed.location = location.clone();
                    browser.publish_rename(&old_location, renamed);
                }
            }
            emit(event);
        });
        let load = provider.rename(
            RenameRequest {
                id: request_id,
                entry,
                new_name,
            },
            publish,
        );
        self.install_operation_load(request_id, load);
        Some(request_id)
    }

    /// Moves an item to a typed relative path, which a rename cannot.
    /// `new_item` is one just created for naming, whose creation undo then
    /// covers the placement too.
    pub fn place_entry(
        self: &Rc<Self>,
        entry: FileEntry,
        path: TypedPath,
        new_item: bool,
    ) -> Option<OperationRequestId> {
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::RenameFailed {
                request_id: None,
                message: crate::i18n::tr("File operations are unavailable"),
            });
            return None;
        };
        let request_id = self.try_begin_operation()?;
        self.rename_operation.set(Some(request_id));
        // The view selects what took the item's place, not the row beside it.
        self.state
            .borrow_mut()
            .set_selectionless_removals([entry.location.clone()]);
        let refresh_locations = entry.location.parent().into_iter().collect();
        let emit = self.operation_callback(request_id, true, refresh_locations);
        let original = entry.location.clone();
        let weak = Rc::downgrade(self);
        let publish = Rc::new(move |event: OperationEvent| {
            let Some(browser) = weak.upgrade() else {
                return emit(event);
            };
            match &event {
                OperationEvent::EntryPlaced {
                    request_id: id,
                    placed,
                } if *id == request_id => {
                    browser.publish_placement(request_id, &original, placed, new_item);
                }
                _ => browser
                    .state
                    .borrow_mut()
                    .retain_selectionless_removals(std::iter::empty()),
            }
            emit(event);
        });
        let load = provider.place_entry(
            PlaceEntryRequest {
                id: request_id,
                entry,
                path,
                new_item,
            },
            publish,
        );
        self.install_operation_load(request_id, load);
        Some(request_id)
    }

    pub(super) fn publish_placement(
        self: &Rc<Self>,
        request_id: OperationRequestId,
        original: &Location,
        placed: &[Location],
        new_item: bool,
    ) {
        let (Some(outermost), Some(current)) = (placed.first(), placed.last()) else {
            return;
        };
        // As for a rename, a placement that another operation superseded
        // still moved the item, but records no undo and leaves the view alone.
        if !self.is_current_operation(request_id) {
            if !new_item {
                self.publish_relocation(original, current);
            }
            return;
        }
        // A placed folder's open columns close; the view then opens the
        // outermost folder that took its row, as a fresh chain.
        if let Some(depth) = (0..)
            .map_while(|depth| self.location_at(depth))
            .position(|location| location.is_within(original))
        {
            self.truncate_columns(depth, false);
        }
        if new_item {
            retarget_created_undo(original, outermost.clone());
            return;
        }
        let record = MoveRecord {
            original: original.clone(),
            current: current.clone(),
            renamed: Vec::new(),
        };
        // Undoing what made folders also trashes them, as for a group folder.
        push_pending_undo(if placed.len() > 1 {
            UndoEntry::Group {
                folder: outermost.clone(),
                records: vec![record],
            }
        } else {
            UndoEntry::Move(vec![record])
        });
        self.publish_relocation(original, current);
        if let (Some(from), Some(to)) = (original.native_path(), current.native_path()) {
            crate::services::refresh_search_indexes_for_rename(from, to);
        }
        self.retire_recent_target(original);
    }

    pub fn create_new_folder(self: &Rc<Self>, parent: Location) {
        self.create_directory_with_naming(parent, crate::i18n::tr("new folder"), true);
    }

    pub(super) fn create_directory_with_naming(
        self: &Rc<Self>,
        parent: Location,
        name: String,
        unique_name: bool,
    ) {
        if parent.is_recent_location() {
            return;
        }
        if let Err(message) = validate_basename(&name) {
            self.emit(BrowserEvent::OperationFailed {
                message: crate::i18n::tr(message),
                password_failure: None,
                report: TransferReport::default(),
            });
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: crate::i18n::tr("File operations are unavailable"),
                password_failure: None,
                report: TransferReport::default(),
            });
            return;
        };
        let Some(request_id) = self.try_begin_operation() else {
            return;
        };
        let refresh_parent = parent.clone();
        let load = provider.create_directory(
            CreateDirectoryRequest {
                id: request_id,
                parent,
                name,
                unique_name,
            },
            self.operation_callback(request_id, false, HashSet::from([refresh_parent])),
        );
        self.install_operation_load(request_id, load);
    }

    pub fn create_exact_entry(self: &Rc<Self>, parent: Location, name: String, directory: bool) {
        if directory {
            self.create_directory_with_naming(parent, name, false);
        } else {
            self.create_file_with_naming(parent, name, false);
        }
    }

    pub fn create_new_file(self: &Rc<Self>, parent: Location) {
        self.create_file_with_naming(parent, crate::i18n::tr("new file"), true);
    }

    pub(super) fn create_file_with_naming(
        self: &Rc<Self>,
        parent: Location,
        name: String,
        unique_name: bool,
    ) {
        if parent.is_recent_location() {
            return;
        }
        if let Err(message) = validate_basename(&name) {
            self.emit(BrowserEvent::OperationFailed {
                message: crate::i18n::tr(message),
                password_failure: None,
                report: TransferReport::default(),
            });
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: crate::i18n::tr("File operations are unavailable"),
                password_failure: None,
                report: TransferReport::default(),
            });
            return;
        };
        let Some(request_id) = self.try_begin_operation() else {
            return;
        };
        let refresh_parent = parent.clone();
        let load = provider.create_file(
            CreateFileRequest {
                id: request_id,
                parent,
                name,
                unique_name,
            },
            self.operation_callback(request_id, false, HashSet::from([refresh_parent])),
        );
        self.install_operation_load(request_id, load);
    }

    pub fn transfer(
        self: &Rc<Self>,
        destination: Location,
        items: Vec<PasteItem>,
        move_sources: bool,
        reveal: bool,
    ) {
        self.start_transfer(destination, items, move_sources, reveal, false);
    }

    pub fn transfer_replacing_cursor(
        self: &Rc<Self>,
        destination: Location,
        items: Vec<PasteItem>,
        move_sources: bool,
        reveal: bool,
    ) {
        self.start_transfer(destination, items, move_sources, reveal, true);
    }

    pub(super) fn start_transfer(
        self: &Rc<Self>,
        destination: Location,
        items: Vec<PasteItem>,
        move_sources: bool,
        reveal: bool,
        replace_cursor: bool,
    ) {
        if items.is_empty() || destination.is_recent_location() {
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: crate::i18n::tr("File operations are unavailable"),
                password_failure: None,
                report: TransferReport::default(),
            });
            return;
        };
        let Some(request_id) = self.try_begin_operation() else {
            return;
        };
        self.transfer_operation.set(Some(move_sources));
        self.operation_backgroundable.set(!move_sources);
        self.operation_description.replace(
            items
                .first()
                .map(|item| item.source.display_name())
                .unwrap_or_default(),
        );
        self.state.borrow_mut().set_selectionless_removals(
            items
                .iter()
                .filter(|_| move_sources && !replace_cursor)
                .map(|item| item.source.clone()),
        );
        self.transfer_destination.replace(Some(destination.clone()));
        self.set_operation_destination(OperationDestination::Folder(&destination));
        self.transfer_reveal.set(reveal);
        self.emit(BrowserEvent::TransferStarted {
            total: items.len(),
            moving: move_sources,
        });
        let mut refresh_locations = HashSet::from([destination.clone()]);
        if move_sources {
            for parent in items.iter().filter_map(|item| item.source.parent()) {
                refresh_locations.insert(parent);
            }
        }
        let load = provider.paste(
            PasteRequest {
                id: request_id,
                destination,
                items,
                move_sources,
            },
            self.operation_callback(request_id, false, refresh_locations),
        );
        self.install_operation_load(request_id, load);
    }

    pub fn delete(self: &Rc<Self>, entries: Vec<FileEntry>, permanent: bool) {
        let destination = if permanent {
            OperationDestination::PermanentDeletion
        } else {
            OperationDestination::Trash
        };
        self.delete_to(entries, destination);
    }

    /// `remote` names the operation for storage that may keep its own copy.
    pub fn delete_permanently(self: &Rc<Self>, entries: Vec<FileEntry>, remote: bool) {
        let destination = if remote {
            OperationDestination::DeletionWithoutTrash
        } else {
            OperationDestination::PermanentDeletion
        };
        self.delete_to(entries, destination);
    }

    pub(super) fn delete_to(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        destination: OperationDestination,
    ) {
        let permanent = !matches!(destination, OperationDestination::Trash);
        if entries.is_empty() {
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: crate::i18n::tr("File operations are unavailable"),
                password_failure: None,
                report: TransferReport::default(),
            });
            return;
        };
        let total = entries.len();
        let Some(request_id) = self.try_begin_operation() else {
            return;
        };
        self.deletion_operation.set(true);
        self.deletion_permanent.set(permanent);
        self.set_operation_destination(destination);
        self.operation_backgroundable.set(true);
        self.operation_description.replace(
            entries
                .first()
                .map(|entry| entry.display_name.clone())
                .unwrap_or_default(),
        );
        self.emit(BrowserEvent::DeletionStarted { total });
        let load = provider.delete(
            DeleteRequest {
                id: request_id,
                entries,
                permanent,
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.install_operation_load(request_id, load);
    }

    pub fn restore(self: &Rc<Self>, items: Vec<RestoreTrashItem>) {
        if items.is_empty() {
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: crate::i18n::tr("File operations are unavailable"),
                password_failure: None,
                report: TransferReport::default(),
            });
            return;
        };
        let total = items.len();
        let Some(request_id) = self.try_begin_operation() else {
            return;
        };
        self.restoration_operation.set(true);
        self.emit(BrowserEvent::RestorationStarted { total });
        let load = provider.restore(
            RestoreRequest {
                id: request_id,
                source: RestoreSource::TrashEntries(items),
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.install_operation_load(request_id, load);
    }

    pub fn compress(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        destination: Location,
        archive_name: String,
        conflict: TransferConflict,
        format: ArchiveFormat,
        password: Option<String>,
    ) {
        if entries.is_empty() {
            return;
        }
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: crate::i18n::tr("File operations are unavailable"),
                password_failure: None,
                report: TransferReport::default(),
            });
            return;
        };
        let Some(request_id) = self.try_begin_operation() else {
            return;
        };
        self.archive_operation.set(true);
        self.operation_backgroundable.set(true);
        self.operation_description.replace(archive_name.clone());
        self.set_operation_destination(OperationDestination::Folder(&destination));
        let refresh = HashSet::from([destination.clone()]);
        let load = provider.compress(
            CompressRequest {
                id: request_id,
                entries,
                destination,
                archive_name,
                conflict,
                format,
                password,
            },
            self.operation_callback(request_id, false, refresh),
        );
        self.install_operation_load(request_id, load);
    }

    pub fn extract(
        self: &Rc<Self>,
        entry: FileEntry,
        destination: Location,
        created_destination: bool,
        password: Option<String>,
    ) {
        let Some(provider) = self.operation_provider.borrow().clone() else {
            self.emit(BrowserEvent::OperationFailed {
                message: crate::i18n::tr("File operations are unavailable"),
                password_failure: None,
                report: TransferReport::default(),
            });
            return;
        };
        let Some(request_id) = self.try_begin_operation() else {
            return;
        };
        self.archive_operation.set(true);
        let load = provider.extract(
            ExtractRequest {
                id: request_id,
                entry,
                destination,
                created_destination,
                password,
            },
            self.operation_callback(request_id, false, HashSet::new()),
        );
        self.install_operation_load(request_id, load);
    }
}

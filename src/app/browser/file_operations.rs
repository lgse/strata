// SPDX-License-Identifier: MIT

use std::{collections::HashSet, rc::Rc};

use crate::{
    model::{FileEntry, Location},
    services::{
        ArchiveFormat, CompressRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest,
        ExtractRequest, MoveRecord, OperationEvent, OperationProvider, OperationRequestId,
        PasteItem, PasteRequest, PlaceEntryRequest, RenameRecord, RenameRequest, RestoreRequest,
        RestoreSource, RestoreTrashItem, TransferConflict, TransferReport, TypedPath,
        validate_basename,
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
        self.start_rename(entry, new_name, false)
    }

    /// Names an item just created for naming, whose creation undo then
    /// covers the name too, as [`Self::place_entry`] does for a path.
    pub fn name_new_item(
        self: &Rc<Self>,
        entry: FileEntry,
        new_name: String,
    ) -> Option<OperationRequestId> {
        self.start_rename(entry, new_name, true)
    }

    fn start_rename(
        self: &Rc<Self>,
        entry: FileEntry,
        new_name: String,
        new_item: bool,
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
        let pending = PendingRename::new(&entry, &new_name, new_item);
        let weak = Rc::downgrade(self);
        let publish = Rc::new(move |event: OperationEvent| {
            if matches!(&event, OperationEvent::Renamed { request_id: id } if *id == request_id)
                && let Some(browser) = weak.upgrade()
            {
                browser.publish_completed_rename(request_id, &pending);
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
                    let placement = Placement {
                        original: &original,
                        placed,
                        new_item,
                    };
                    browser.publish_placement(request_id, placement);
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

    fn publish_completed_rename(
        self: &Rc<Self>,
        request_id: OperationRequestId,
        pending: &PendingRename,
    ) {
        let Some(location) = pending.moved_to() else {
            return;
        };
        let old_location = &pending.original.location;
        self.publish_relocation(old_location, location);
        if self.is_current_operation(request_id) {
            if pending.new_item {
                retarget_created_undo(old_location, location.clone());
            } else {
                push_pending_undo(UndoEntry::Rename(pending.record(location)));
            }
            let mut renamed = pending.renamed.clone();
            renamed.location = location.clone();
            self.publish_rename(old_location, renamed);
        }
    }

    fn publish_placement(
        self: &Rc<Self>,
        request_id: OperationRequestId,
        placement: Placement<'_>,
    ) {
        let Placement {
            original,
            placed,
            new_item,
        } = placement;
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
            replaced: None,
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
        self.create_entry(NewEntry::new(parent, name, unique_name, true));
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
        self.create_entry(NewEntry::new(parent, name, unique_name, false));
    }

    fn create_entry(self: &Rc<Self>, entry: NewEntry) {
        if entry.parent.is_recent_location() || entry.parent.is_computer_root() {
            return;
        }
        if let Err(message) = validate_basename(&entry.name) {
            self.emit_operation_failed(crate::i18n::tr(message));
            return;
        }
        let Some(provider) = self.available_operation_provider() else {
            return;
        };
        let Some(request_id) = self.try_begin_operation() else {
            return;
        };
        let refresh = HashSet::from([entry.parent.clone()]);
        let NewEntry {
            parent,
            name,
            unique_name,
            directory,
        } = entry;
        let emit = self.operation_callback(request_id, false, refresh);
        let load = if directory {
            let request = CreateDirectoryRequest {
                id: request_id,
                parent,
                name,
                unique_name,
            };
            provider.create_directory(request, emit)
        } else {
            let request = CreateFileRequest {
                id: request_id,
                parent,
                name,
                unique_name,
            };
            provider.create_file(request, emit)
        };
        self.install_operation_load(request_id, load);
    }

    pub fn transfer(
        self: &Rc<Self>,
        destination: Location,
        items: Vec<PasteItem>,
        move_sources: bool,
        reveal: bool,
    ) {
        let transfer = Transfer::new(destination, items, move_sources, reveal);
        self.start_transfer(transfer, false);
    }

    pub fn transfer_replacing_cursor(
        self: &Rc<Self>,
        destination: Location,
        items: Vec<PasteItem>,
        move_sources: bool,
        reveal: bool,
    ) {
        let transfer = Transfer::new(destination, items, move_sources, reveal);
        self.start_transfer(transfer, true);
    }

    fn start_transfer(self: &Rc<Self>, transfer: Transfer, replace_cursor: bool) {
        let Transfer {
            destination,
            items,
            move_sources,
            reveal,
        } = transfer;
        if items.is_empty() || destination.is_recent_location() || destination.is_computer_root() {
            return;
        }
        let Some(provider) = self.available_operation_provider() else {
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

    fn delete_to(self: &Rc<Self>, entries: Vec<FileEntry>, destination: OperationDestination) {
        let permanent = !matches!(destination, OperationDestination::Trash);
        let entries: Vec<FileEntry> = entries
            .into_iter()
            .filter(|entry| !entry.is_computer_item())
            .collect();
        if entries.is_empty() {
            return;
        }
        let Some(provider) = self.available_operation_provider() else {
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
        let Some(provider) = self.available_operation_provider() else {
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
        let Some(provider) = self.available_operation_provider() else {
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
        let Some(provider) = self.available_operation_provider() else {
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

    /// The operation provider, or a reported failure when there is none.
    fn available_operation_provider(&self) -> Option<Rc<dyn OperationProvider>> {
        let provider = self.operation_provider.borrow().clone();
        if provider.is_none() {
            self.emit_operation_failed(crate::i18n::tr("File operations are unavailable"));
        }
        provider
    }

    fn emit_operation_failed(&self, message: String) {
        self.emit(BrowserEvent::OperationFailed {
            message,
            password_failure: None,
            report: TransferReport::default(),
        });
    }
}

/// A rename in flight: the entry as listed, and as it will read once renamed.
struct PendingRename {
    original: FileEntry,
    renamed: FileEntry,
    new_location: Option<Location>,
    new_item: bool,
}

impl PendingRename {
    fn new(entry: &FileEntry, new_name: &str, new_item: bool) -> Self {
        let mut renamed = entry.clone();
        renamed.native_name = std::ffi::OsString::from(new_name);
        renamed.display_name = new_name.to_owned();
        renamed.is_hidden = new_name.starts_with('.');
        Self {
            original: entry.clone(),
            renamed,
            new_location: entry
                .location
                .parent()
                .and_then(|parent| parent.child(std::ffi::OsStr::new(new_name))),
            new_item,
        }
    }

    /// Where the item went, unless the rename kept its location.
    fn moved_to(&self) -> Option<&Location> {
        self.new_location
            .as_ref()
            .filter(|location| **location != self.original.location)
    }

    fn record(&self, current: &Location) -> RenameRecord {
        RenameRecord {
            original: self.original.location.clone(),
            current: current.clone(),
            native_name: self.original.native_name.clone(),
            display_name: self.original.display_name.clone(),
            is_hidden: self.original.is_hidden,
        }
    }
}

/// Where a placed item went: the typed path's folders from outermost to the
/// item itself.
struct Placement<'a> {
    original: &'a Location,
    placed: &'a [Location],
    new_item: bool,
}

struct NewEntry {
    parent: Location,
    name: String,
    unique_name: bool,
    directory: bool,
}

impl NewEntry {
    fn new(parent: Location, name: String, unique_name: bool, directory: bool) -> Self {
        Self {
            parent,
            name,
            unique_name,
            directory,
        }
    }
}

struct Transfer {
    destination: Location,
    items: Vec<PasteItem>,
    move_sources: bool,
    reveal: bool,
}

impl Transfer {
    fn new(destination: Location, items: Vec<PasteItem>, move_sources: bool, reveal: bool) -> Self {
        Self {
            destination,
            items,
            move_sources,
            reveal,
        }
    }
}

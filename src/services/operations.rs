// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{
    any::Any,
    collections::{HashMap, HashSet},
    ffi::OsString,
    fmt,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use crate::model::{FileEntry, Location};

use super::{ItemIdentity, LoadHandle};

pub fn validate_basename(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        Err("Enter a name")
    } else if name.contains('/') {
        Err("Names cannot contain /")
    } else if matches!(name, "." | "..") {
        Err("That name is reserved")
    } else if name.contains('\0') {
        Err("Names cannot contain NUL characters")
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OperationRequestId(pub u64);

#[derive(Clone, Debug)]
pub struct RenameRequest {
    pub id: OperationRequestId,
    pub entry: FileEntry,
    pub new_name: String,
}

#[derive(Clone, Debug)]
pub struct CreateDirectoryRequest {
    pub id: OperationRequestId,
    pub parent: Location,
    pub name: String,
    pub unique_name: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferConflict {
    FailIfExists,
    ReplaceExisting,
    KeepBoth,
    /// Union of two folders into the existing destination. Incoming items
    /// overwrite same-named destination items; destination-only items stay.
    /// Overwritten originals are staged in Trash and written paths are
    /// reported via [`OperationEvent::Merged`] so undo can restore the
    /// pre-merge state without trashing the whole destination folder.
    Merge,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PasteItem {
    pub source: Location,
    pub conflict: TransferConflict,
    /// What the conflict prompt showed, which the paste uses as it is; `None` lets it plan one.
    pub target_name: Option<ShownName>,
}

/// A name the conflict prompt showed for an item, and why it is not the item's own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShownName {
    pub name: OsString,
    pub cause: Option<RenameCause>,
}

/// A completed move: where an item started and where it ended up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoveRecord {
    pub original: Location,
    pub current: Location,
    /// Entries the move saved under other names, beneath `current`.
    pub renamed: Vec<RenamedItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenameRecord {
    pub original: Location,
    pub current: Location,
    pub native_name: OsString,
    pub display_name: String,
    pub is_hidden: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UndoMoveItem {
    pub record: MoveRecord,
    pub conflict: TransferConflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameMode {
    Fix,
    Keep,
}

#[derive(Clone, Debug)]
pub struct UndoMoveRequest {
    pub id: OperationRequestId,
    pub items: Vec<UndoMoveItem>,
    pub cleanup_locations: Vec<Location>,
    pub names: NameMode,
}

#[derive(Clone, Debug)]
pub struct UndoRenameRequest {
    pub id: OperationRequestId,
    pub current: Location,
    pub original: Location,
}

#[derive(Clone, Debug)]
pub struct UndoCopyRequest {
    pub id: OperationRequestId,
    pub locations: Vec<Location>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplacedOriginal {
    Trashed(Option<ItemIdentity>),
    /// Removed without Trash, so undo must not offer to delete the only copy.
    Deleted,
}

/// Only the provider that recorded it can read it.
#[derive(Clone)]
pub struct PlacedSnapshot(Arc<dyn Any + Send + Sync>);

impl PlacedSnapshot {
    pub fn new(snapshot: impl Any + Send + Sync) -> Self {
        Self(Arc::new(snapshot))
    }

    pub fn get<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

impl PartialEq for PlacedSnapshot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for PlacedSnapshot {}

impl fmt::Debug for PlacedSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PlacedSnapshot")
    }
}

#[derive(Clone, Debug)]
pub struct UndoMergeRequest {
    pub id: OperationRequestId,
    /// Paths the merge wrote fresh; undo moves them to Trash, or without
    /// Trash deletes them as far as `placed` records them unchanged.
    pub created: Vec<Location>,
    /// Paths whose originals went to Trash, or aside where Trash is
    /// unavailable, before being overwritten; undo deletes the incoming copy
    /// and restores an original from Trash.
    pub overwritten: Vec<Location>,
    pub originals: HashMap<Location, ItemIdentity>,
    /// What the operation put at created and overwritten paths; an item
    /// without one, or changed since, stays, and so does its original in
    /// Trash.
    pub placed: HashMap<Location, PlacedSnapshot>,
}

#[derive(Clone, Debug)]
pub struct CreateFileRequest {
    pub id: OperationRequestId,
    pub parent: Location,
    pub name: String,
    pub unique_name: bool,
}

#[derive(Clone, Debug)]
pub struct PasteRequest {
    pub id: OperationRequestId,
    pub destination: Location,
    pub items: Vec<PasteItem>,
    pub move_sources: bool,
}

#[derive(Clone, Debug)]
pub struct DeleteRequest {
    pub id: OperationRequestId,
    pub entries: Vec<FileEntry>,
    pub permanent: bool,
}

#[derive(Clone, Debug)]
pub struct RestoreTrashItem {
    pub entry: FileEntry,
    pub destination: PathBuf,
}

#[derive(Clone, Debug)]
pub enum RestoreSource {
    TrashEntries(Vec<RestoreTrashItem>),
    OriginalLocations {
        locations: Vec<Location>,
        originals: HashMap<Location, ItemIdentity>,
    },
}

#[derive(Clone, Debug)]
pub struct RestoreRequest {
    pub id: OperationRequestId,
    pub source: RestoreSource,
}

#[derive(Clone, Debug)]
pub struct RecoverRequest {
    pub id: OperationRequestId,
    pub container: Location,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedLeftover {
    pub name: OsString,
    pub modified: Option<i64>,
    pub inside: LeftoverInside,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeftoverInside {
    Item,
    Items { names: Vec<PathBuf>, more: usize },
    StillOpen(String),
    Gone,
    Unreadable(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveFormat {
    Zip,
    SevenZ,
    TarGz,
    Tar,
    Rar,
}

impl ArchiveFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZ => "7z",
            Self::TarGz => "tar.gz",
            Self::Tar => "tar",
            Self::Rar => "rar",
        }
    }

    pub fn supports_password(self) -> bool {
        matches!(self, Self::Zip | Self::SevenZ | Self::Rar)
    }

    pub fn from_extension(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if lower.ends_with(".tar") {
            Some(Self::Tar)
        } else if lower.ends_with(".zip") {
            Some(Self::Zip)
        } else if lower.ends_with(".7z") {
            Some(Self::SevenZ)
        } else if cfg!(feature = "rar") && lower.ends_with(".rar") {
            Some(Self::Rar)
        } else {
            None
        }
    }

    pub fn for_entry(entry: &FileEntry) -> Option<Self> {
        if !entry.is_file() || entry.location.native_path().is_none() {
            return None;
        }
        Self::from_extension(&entry.display_name)
    }
}

#[derive(Clone, Debug)]
pub struct CompressRequest {
    pub id: OperationRequestId,
    pub entries: Vec<FileEntry>,
    pub destination: Location,
    pub archive_name: String,
    pub conflict: TransferConflict,
    pub format: ArchiveFormat,
    pub password: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ExtractRequest {
    pub id: OperationRequestId,
    pub entry: FileEntry,
    pub destination: Location,
    /// Caller-reserved destinations are eligible for empty-folder cleanup.
    pub created_destination: bool,
    pub password: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CancelledOperation {
    pub completed: Vec<Location>,
    pub failed: Vec<Location>,
    pub not_attempted: Vec<Location>,
    pub affected_locations: HashSet<Location>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenameCause {
    /// The destination takes the name for another item's, as a
    /// case-insensitive filesystem takes `makefile` for `Makefile`.
    Alias,
    Taken,
    /// Characters, trailing dots or spaces, a reserved name or an encoding
    /// the destination cannot store.
    Invalid,
    TooLong,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenamedItem {
    pub requested: Location,
    pub saved_as: Location,
    pub cause: RenameCause,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecialKind {
    Pipe,
    Socket,
    Device,
    Symlink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedItem {
    pub location: Location,
    pub kind: SpecialKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Holder {
    Strata,
    Program(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeptCause {
    Changed,
    /// What replaced it may have changed afterwards, so undo kept that instead.
    ReplacementChanged,
    /// Deleting it failed with this OS error number.
    Undeletable(i32),
    Skipped(SpecialKind),
    /// Its copy was no longer where the move put it.
    CopyGone,
    StillOpen(Option<Holder>),
    NameNotRestored {
        saved_as: Location,
        error: String,
    },
    OriginalNotRemoved(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeptItem {
    pub location: Location,
    pub cause: KeptCause,
}

/// Keep Both numbering, which the user chose, is not reported.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransferReport {
    pub renamed: Vec<RenamedItem>,
    pub skipped: Vec<SkippedItem>,
    pub kept: Vec<KeptItem>,
    pub kept_in_trash: Vec<KeptItem>,
    /// The moved items, at their sources, that undo leaves out, as each
    /// kept entries there.
    pub kept_out_of_undo: Vec<Location>,
}

impl TransferReport {
    pub fn is_empty(&self) -> bool {
        self.renamed.is_empty()
            && self.skipped.is_empty()
            && self.kept.is_empty()
            && self.kept_in_trash.is_empty()
    }

    pub fn extend(&mut self, other: Self) {
        self.renamed.extend(other.renamed);
        self.skipped.extend(other.skipped);
        self.kept.extend(other.kept);
        self.kept_in_trash.extend(other.kept_in_trash);
        self.kept_out_of_undo.extend(other.kept_out_of_undo);
    }

    /// Joins the entries of an item saved under one name after another, as
    /// a fixed name that the destination then took for another's, into one
    /// from the first name to the last, with the first cause, whatever order
    /// they were reported in.
    pub fn follow_renames(&mut self) {
        let mut chains = Vec::<Option<RenamedItem>>::with_capacity(self.renamed.len());
        let mut by_start = HashMap::<Location, usize>::new();
        let mut by_end = HashMap::<Location, usize>::new();
        for item in std::mem::take(&mut self.renamed) {
            let before = by_end.remove(&item.requested);
            let after = by_start
                .remove(&item.saved_as)
                .filter(|after| Some(*after) != before);
            match (before, after) {
                (Some(before), after) => {
                    let saved_as = match after.and_then(|after| chains[after].take()) {
                        Some(after) => {
                            by_end.remove(&after.saved_as);
                            after.saved_as
                        }
                        None => item.saved_as,
                    };
                    if let Some(chain) = &mut chains[before] {
                        chain.saved_as = saved_as.clone();
                    }
                    by_end.insert(saved_as, before);
                }
                (None, Some(after)) => {
                    if let Some(chain) = &mut chains[after] {
                        chain.requested = item.requested.clone();
                        chain.cause = item.cause;
                    }
                    by_start.insert(item.requested, after);
                }
                (None, None) => {
                    by_start.insert(item.requested.clone(), chains.len());
                    by_end.insert(item.saved_as.clone(), chains.len());
                    chains.push(Some(item));
                }
            }
        }
        self.renamed = chains
            .into_iter()
            .flatten()
            .filter(|chain| chain.requested != chain.saved_as)
            .collect();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasswordFailure {
    Required,
    /// Includes ambiguous decryption failures that may instead indicate damaged data.
    Incorrect,
}

#[derive(Clone, Debug)]
pub enum OperationEvent {
    Renamed {
        request_id: OperationRequestId,
    },
    Created {
        request_id: OperationRequestId,
    },
    EntryCreated {
        request_id: OperationRequestId,
        location: Location,
    },
    Pasted {
        request_id: OperationRequestId,
        locations: Vec<Location>,
    },
    /// One moved item landed at `to`, the destination the conflict policy
    /// actually chose (Keep both and FAT names differ from the source name).
    /// Reported per item, so a partial move still says what moved. `merged`
    /// means `to` is an existing folder the item was merged into.
    ItemMoved {
        request_id: OperationRequestId,
        from: Location,
        to: Location,
        merged: bool,
    },
    /// An operation moved the local item at `location` to Trash, including an
    /// item a Replace or a cleanup displaced. `identity` survives the move, so
    /// a restore can tell this item from another that left the same path.
    ItemTrashed {
        request_id: OperationRequestId,
        location: Location,
        identity: ItemIdentity,
    },
    ItemRestored {
        request_id: OperationRequestId,
        location: Location,
        identity: ItemIdentity,
    },
    /// A merge undo left the item at `location`, as it may have changed
    /// since, so nothing of it is left to undo. A created item so kept still
    /// counts as failed.
    ItemKept {
        request_id: OperationRequestId,
        location: Location,
    },
    FlushingToDevice {
        request_id: OperationRequestId,
    },
    ItemsReported {
        request_id: OperationRequestId,
        report: TransferReport,
    },
    /// A folder merge finished (or staged its backups): `created` are paths
    /// the merge wrote fresh, `overwritten` are paths whose originals now
    /// sit in Trash, and `placed` what it put at them. Reported per merged
    /// source so undo can rebuild the pre-merge state even after a partial
    /// transfer. Where there is no Trash, a paste with a merge also reports
    /// here only `placed` for each item it copied beside it.
    Merged {
        request_id: OperationRequestId,
        source: Location,
        created: Vec<Location>,
        overwritten: Vec<Location>,
        placed: HashMap<Location, PlacedSnapshot>,
    },
    TransferFailed {
        request_id: OperationRequestId,
        completed_locations: Vec<Location>,
        message: String,
    },
    TransferProgress {
        request_id: OperationRequestId,
        completed_items: usize,
        completed_files: usize,
        total_files: Option<usize>,
        current_file: Option<String>,
        transferred_bytes: u64,
        total_bytes: Option<u64>,
        created_location: Option<Location>,
    },
    DeleteProgress {
        request_id: OperationRequestId,
        completed: usize,
        total: usize,
        deleted_locations: Vec<Location>,
    },
    /// Entries removed so far inside the items being deleted, which a single
    /// large folder reports long before `DeleteProgress` counts it as done.
    DeleteEntriesRemoved {
        request_id: OperationRequestId,
        removed: usize,
    },
    RestoreProgress {
        request_id: OperationRequestId,
        completed: usize,
        total: usize,
        restored_location: Option<Location>,
    },
    Deleted {
        request_id: OperationRequestId,
        locations: Vec<Location>,
    },
    CompletedWithErrors {
        request_id: OperationRequestId,
        deleted_locations: Vec<Location>,
        /// Entries that failed only because this location doesn't support
        /// Trash, so a retry with `permanent: true` on just these would
        /// likely succeed.
        retryable_locations: Vec<Location>,
        has_non_retryable_failures: bool,
        message: String,
    },
    Restored {
        request_id: OperationRequestId,
        /// What left its folder: Trash entries, or what a merge undo removed.
        locations: Vec<Location>,
        /// Where the restored items landed, recorded for undo.
        restored: Vec<Location>,
    },
    RestoreCompletedWithErrors {
        request_id: OperationRequestId,
        /// Trash entries that left the trash view.
        restored_locations: Vec<Location>,
        /// Where the restored items landed, recorded for undo.
        restored: Vec<Location>,
        message: String,
    },
    Cancelled {
        request_id: OperationRequestId,
        result: CancelledOperation,
    },
    Failed {
        request_id: OperationRequestId,
        message: String,
        password_failure: Option<PasswordFailure>,
    },
    Compressed {
        request_id: OperationRequestId,
        archive_name: String,
        /// The finished archive, recorded so undo can trash it.
        archive: Location,
        original: Option<ReplacedOriginal>,
        placed: Option<PlacedSnapshot>,
    },
    Extracted {
        request_id: OperationRequestId,
        first_name: Option<String>,
        created: Vec<Location>,
    },
    ArchiveStarted {
        request_id: OperationRequestId,
        total: usize,
    },
    ArchiveProgress {
        request_id: OperationRequestId,
        completed: usize,
        total: usize,
    },
}

pub trait OperationProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle;
    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle;
    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    /// Moves completed transfers back to their original locations.
    fn undo_move(&self, request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn undo_rename(
        &self,
        request: UndoRenameRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle;
    fn undo_copy(&self, request: UndoCopyRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    /// Reverts a merge: trashes what the merge created, deletes the incoming
    /// copies at overwritten paths, and restores the staged originals.
    fn undo_merge(&self, request: UndoMergeRequest, emit: Rc<dyn Fn(OperationEvent)>)
    -> LoadHandle;
    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn recover(&self, request: RecoverRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn sweep_probes(&self, _folder: &Location, _names: &[OsString]) {}
    fn review_leftovers(
        &self,
        _folder: &Location,
        _names: Vec<OsString>,
        done: Box<dyn FnOnce(Vec<ReviewedLeftover>)>,
    ) -> LoadHandle {
        done(Vec::new());
        LoadHandle::new(|| {})
    }
    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
}

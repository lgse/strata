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

pub use crate::model::ArchiveFormat;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypedPath {
    /// How many levels above the item's folder the path starts.
    pub up: usize,
    /// Folders between that start and the item, outermost first.
    pub folders: Vec<String>,
    pub name: String,
    /// The item ends up a folder: it is one, or the text ends with `/`.
    pub folder: bool,
}

impl TypedPath {
    /// Only a `new_item` file may be named as a folder, as that replaces it.
    pub fn parse(text: &str, is_directory: bool, new_item: bool) -> Result<Self, &'static str> {
        let trimmed = text.strip_suffix('/');
        let mut components = trimmed.unwrap_or(text).split('/').collect::<Vec<_>>();
        let name = components.pop().unwrap_or_default();
        validate_basename(name)?;
        let (mut up, mut folders) = (0, Vec::<String>::new());
        for component in components {
            match component {
                "." => {}
                ".." if folders.pop().is_none() => up += 1,
                ".." => {}
                folder => {
                    validate_basename(folder)?;
                    folders.push(folder.to_owned());
                }
            }
        }
        if trimmed.is_some() && !is_directory && !new_item {
            return Err("An existing file can’t become a folder");
        }
        Ok(Self {
            up,
            folders,
            name: name.to_owned(),
            folder: is_directory || trimmed.is_some(),
        })
    }

    pub fn within(mut self, folder: &Location) -> Self {
        while self.up > 0 {
            let reentered = (1..self.up)
                .try_fold(folder.clone(), |folder, _| folder.parent())
                .and_then(|ancestor| ancestor.file_name())
                .is_some_and(|name| self.folders.first().is_some_and(|first| name == **first));
            if !reentered {
                break;
            }
            self.up -= 1;
            self.folders.remove(0);
        }
        self
    }

    /// The component that appears in the folder the path starts from.
    pub fn first(&self) -> &str {
        self.folders.first().unwrap_or(&self.name)
    }

    /// An ordinary rename: it stays in its folder and keeps its kind.
    pub fn is_rename_of(&self, is_directory: bool) -> bool {
        self.up == 0 && self.folders.is_empty() && self.folder == is_directory
    }

    /// The folder the path starts from, for an item in `folder`.
    pub fn start(&self, folder: &Location) -> Option<Location> {
        (0..self.up).try_fold(folder.clone(), |folder, _| folder.parent())
    }

    /// Every component below the start, the item's name last.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.folders
            .iter()
            .map(String::as_str)
            .chain([self.name.as_str()])
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
pub struct PlaceEntryRequest {
    pub id: OperationRequestId,
    pub entry: FileEntry,
    pub path: TypedPath,
    pub new_item: bool,
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
    /// The staged original to restore at the path the next replay vacates.
    pub replaced: Option<ItemIdentity>,
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
    pub created: Vec<Location>,
    pub overwritten: Vec<Location>,
    pub originals: HashMap<Location, ItemIdentity>,
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
    Alias,
    Taken,
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

/// A nested native entry reported by a failed deletion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteBlocker {
    pub root: PathBuf,
    pub child: PathBuf,
    /// Whether the message lists other failures too, so an action has to name this one.
    pub one_of_several: bool,
}

#[derive(Clone, Debug)]
pub enum OperationEvent {
    Renamed {
        request_id: OperationRequestId,
    },
    EntryPlaced {
        request_id: OperationRequestId,
        placed: Vec<Location>,
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
    ItemMoved {
        request_id: OperationRequestId,
        from: Location,
        to: Location,
        merged: bool,
    },
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
        retryable_locations: Vec<Location>,
        has_non_retryable_failures: bool,
        message: String,
        /// The first nested failure shown in the message, if its path is revealable.
        blocker: Option<DeleteBlocker>,
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
    fn place_entry(
        &self,
        request: PlaceEntryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle;
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

// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::PathBuf,
    rc::Rc,
};

use crate::model::{FileEntry, Location};

use super::LoadHandle;

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
}

/// A completed move: where an item started and where it ended up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoveRecord {
    pub original: Location,
    pub current: Location,
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

#[derive(Clone, Debug)]
pub struct UndoMoveRequest {
    pub id: OperationRequestId,
    pub items: Vec<UndoMoveItem>,
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

/// Identity preserved by a local move to Trash, independent of deletion timestamps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrashedOriginal {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug)]
pub struct UndoMergeRequest {
    pub id: OperationRequestId,
    /// Paths the merge wrote fresh; undo moves them to Trash.
    pub created: Vec<Location>,
    /// Paths whose originals were staged in Trash before being overwritten;
    /// undo deletes the incoming copy and restores the original.
    pub overwritten: Vec<Location>,
    pub originals: HashMap<Location, TrashedOriginal>,
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
    OriginalLocations(Vec<Location>),
}

#[derive(Clone, Debug)]
pub struct RestoreRequest {
    pub id: OperationRequestId,
    pub source: RestoreSource,
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
        } else if lower.ends_with(".rar") {
            Some(Self::Rar)
        } else {
            None
        }
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

#[derive(Clone, Debug)]
pub enum OperationEvent {
    Renamed {
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
    FlushingToDevice {
        request_id: OperationRequestId,
    },
    /// A folder merge finished (or staged its backups): `created` are paths
    /// the merge wrote fresh, `overwritten` are paths whose originals now
    /// sit in Trash. Reported per merged source so undo can rebuild the
    /// pre-merge state even after a partial transfer.
    Merged {
        request_id: OperationRequestId,
        source: Location,
        created: Vec<Location>,
        overwritten: Vec<Location>,
    },
    TransferFailed {
        request_id: OperationRequestId,
        completed_locations: Vec<Location>,
        message: String,
    },
    TransferProgress {
        request_id: OperationRequestId,
        completed_items: usize,
        transferred_bytes: u64,
        total_bytes: Option<u64>,
        created_location: Option<Location>,
    },
    DeleteProgress {
        request_id: OperationRequestId,
        completed: usize,
        total: usize,
        deleted_location: Option<Location>,
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
        /// Trash entries that left the trash view.
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
    },
    Compressed {
        request_id: OperationRequestId,
        archive_name: String,
        /// The finished archive, recorded so undo can trash it.
        archive: Location,
        /// The exact original to restore, when publication replaced an archive.
        original: Option<TrashedOriginal>,
    },
    Extracted {
        request_id: OperationRequestId,
        first_name: Option<String>,
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
    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
}

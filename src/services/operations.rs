// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::rc::Rc;

use crate::model::{FileEntry, Location};

use super::LoadHandle;

pub fn validate_basename(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferConflict {
    FailIfExists,
    ReplaceExisting,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PasteItem {
    pub source: Location,
    pub conflict: TransferConflict,
}

#[derive(Clone, Debug)]
pub struct CreateFileRequest {
    pub id: OperationRequestId,
    pub parent: Location,
    pub name: String,
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
pub struct RestoreRequest {
    pub id: OperationRequestId,
    pub entries: Vec<FileEntry>,
}

#[derive(Clone, Debug)]
pub enum OperationEvent {
    Renamed {
        request_id: OperationRequestId,
    },
    Created {
        request_id: OperationRequestId,
    },
    Pasted {
        request_id: OperationRequestId,
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
        message: String,
    },
    Restored {
        request_id: OperationRequestId,
        locations: Vec<Location>,
    },
    RestoreCompletedWithErrors {
        request_id: OperationRequestId,
        restored_locations: Vec<Location>,
        message: String,
    },
    Failed {
        request_id: OperationRequestId,
        message: String,
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
    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
}

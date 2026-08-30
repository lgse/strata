// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fmt, rc::Rc};

use crate::model::{FileEntry, Location};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RequestId(pub u64);

#[derive(Clone, Debug)]
pub struct DirectoryRequest {
    pub id: RequestId,
    pub location: Location,
    pub batch_size: usize,
    pub include_hidden: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocationValidationError {
    Empty,
    NotAbsolute,
    Missing,
    NotDirectory,
    Inaccessible,
    NotMounted(Location),
    Mountable(Location),
    Unavailable(String),
}

impl fmt::Display for LocationValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Enter a location."),
            Self::NotAbsolute => formatter.write_str("Enter an absolute path."),
            Self::Missing => formatter.write_str("That location does not exist."),
            Self::NotDirectory => formatter.write_str("That location is not a directory."),
            Self::Inaccessible => {
                formatter.write_str("You do not have permission to open that location.")
            }
            Self::NotMounted(_) => formatter.write_str("That location is not mounted yet."),
            Self::Mountable(_) => formatter.write_str("That location needs to be mounted first."),
            Self::Unavailable(message) => {
                write!(formatter, "Unable to open that location: {message}")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum DirectoryChange {
    Upsert(FileEntry),
    Remove(Location),
    Move { from: Location, entry: FileEntry },
    Rescan,
}

#[derive(Clone, Debug)]
pub enum DirectoryEvent {
    Batch {
        request_id: RequestId,
        entries: Vec<FileEntry>,
    },
    Finished {
        request_id: RequestId,
    },
    Failed {
        request_id: RequestId,
        message: String,
    },
}

/// A cancellable directory load. Dropping it cancels any unfinished provider work.
pub struct LoadHandle {
    cancel: Option<Box<dyn FnOnce()>>,
}

impl LoadHandle {
    pub fn new(cancel: impl FnOnce() + 'static) -> Self {
        Self {
            cancel: Some(Box::new(cancel)),
        }
    }
}

impl Drop for LoadHandle {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

pub trait FileSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError>;

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle;

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        _notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        None
    }
}

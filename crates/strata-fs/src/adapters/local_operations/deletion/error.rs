// SPDX-License-Identifier: MIT

use std::{fmt, path::PathBuf};

use super::super::io_error;
use crate::adapters::trash_support::TrashError;

#[derive(Clone, Debug)]
pub(in crate::adapters::local_operations) enum LocalDeleteError {
    Cancelled,
    Failed {
        message: String,
        child: Option<PathBuf>,
    },
}

impl LocalDeleteError {
    pub(super) fn failed(message: String, child: Option<PathBuf>) -> Self {
        Self::Failed { message, child }
    }

    pub(super) fn into_gio(self) -> glib::Error {
        match self {
            Self::Cancelled => glib::Error::new(gio::IOErrorEnum::Cancelled, "Delete cancelled"),
            Self::Failed { message, .. } => io_error(message),
        }
    }

    pub(super) fn into_trash_error(self) -> TrashError {
        match self {
            Self::Failed { message, child } => TrashError {
                child,
                ..TrashError::permanent(io_error(message))
            },
            Self::Cancelled => TrashError::permanent(self.into_gio()),
        }
    }
}

impl From<String> for LocalDeleteError {
    fn from(message: String) -> Self {
        Self::failed(message, None)
    }
}

impl fmt::Display for LocalDeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "Delete cancelled",
            Self::Failed { message, .. } => message,
        })
    }
}

impl std::error::Error for LocalDeleteError {}

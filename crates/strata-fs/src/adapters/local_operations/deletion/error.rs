// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, fmt, path::PathBuf};

use super::super::io_error;
use crate::adapters::trash_support::TrashError;

#[derive(Clone, Debug)]
pub(in crate::adapters::local_operations) enum LocalDeleteError {
    Cancelled,
    Failed {
        message: String,
        /// What `message` says apart from the entry's name, for a label that already has it.
        reason: Option<String>,
        child: Option<PathBuf>,
    },
}

impl LocalDeleteError {
    pub(super) fn refused(name: &OsStr, reason: String) -> Self {
        let message = rust_i18n::t!(
            "Could not delete “%{name}”: %{error}",
            name = name.to_string_lossy(),
            error = reason
        )
        .into_owned();
        Self::Failed {
            message,
            reason: Some(reason),
            child: None,
        }
    }

    pub(super) fn at(self, path: Option<PathBuf>) -> Self {
        match self {
            Self::Cancelled => Self::Cancelled,
            Self::Failed {
                message, reason, ..
            } => Self::Failed {
                message,
                reason,
                child: path,
            },
        }
    }

    pub(super) fn into_gio(self) -> glib::Error {
        match self {
            Self::Cancelled => glib::Error::new(gio::IOErrorEnum::Cancelled, "Delete cancelled"),
            Self::Failed { message, .. } => io_error(message),
        }
    }

    pub(super) fn into_trash_error(self) -> TrashError {
        match self {
            Self::Failed {
                message,
                reason,
                child,
            } => {
                let detail = reason.filter(|_| child.is_some()).unwrap_or(message);
                TrashError {
                    child,
                    ..TrashError::permanent(io_error(detail))
                }
            }
            Self::Cancelled => TrashError::permanent(self.into_gio()),
        }
    }
}

impl From<String> for LocalDeleteError {
    fn from(message: String) -> Self {
        Self::Failed {
            message,
            reason: None,
            child: None,
        }
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

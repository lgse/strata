// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

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
    UnsupportedShorthand(String),
    EmbeddedCredential,
    BackendUnavailable(String),
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
            Self::UnsupportedShorthand(message) => formatter.write_str(message),
            Self::EmbeddedCredential => formatter.write_str(
                "Passwords typed into the address bar aren't accepted. Enter the address \
                 without a password, you'll be prompted to sign in securely.",
            ),
            Self::BackendUnavailable(message) => formatter.write_str(message),
        }
    }
}

/// Maps a location's URI scheme to the distribution package that provides its
/// GVfs backend, for the schemes we currently support connecting to.
fn backend_package_hint(scheme: &str) -> Option<&'static str> {
    match scheme.to_ascii_lowercase().as_str() {
        "smb" => Some("gvfs-smb"),
        _ => None,
    }
}

/// Builds a "this backend isn't installed" message naming the scheme and, when
/// known, the package that provides it, without repeating the host/share/path.
pub fn backend_unavailable_message(uri: &str) -> String {
    let scheme = uri.split("://").next().unwrap_or(uri);
    match backend_package_hint(scheme) {
        Some(package) => format!(
            "The {scheme}:// backend isn't installed. Install the {package} package to \
             connect to {scheme}:// locations."
        ),
        None => format!(
            "The {scheme}:// backend isn't installed on this system, so {scheme}:// \
             locations can't be opened."
        ),
    }
}

/// Detects a `user:password@host` (or `user:password@host:port`) userinfo
/// segment in a URI's authority. Per lgse/strata#20, a URI typed with an
/// embedded password must never be accepted, stored, or echoed back.
pub fn uri_has_embedded_password(uri: &str) -> bool {
    let Some(after_scheme) = uri.split_once("://").map(|(_, rest)| rest) else {
        return false;
    };
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let Some((userinfo, _host)) = authority.rsplit_once('@') else {
        return false;
    };
    userinfo.contains(':')
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

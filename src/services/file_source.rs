// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{fmt, rc::Rc, time::Duration};

use crate::model::{FileEntry, Location, MetadataValue, uri_contains_credentials};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RequestId(pub u64);

#[derive(Clone, Debug)]
pub struct DirectoryRequest {
    pub id: RequestId,
    pub location: Location,
    pub batch_size: usize,
    /// When true, entries arrive with size and modification time. Size/date sorts set it;
    /// other sorts stream identity only and fill the visible window afterwards.
    pub include_metadata: bool,
    /// Caps how many entries a single load will retain/render, bounding worst-case time and
    /// memory on an adversarially large or unbounded directory.
    pub max_entries: usize,
    /// Caps how long a single load may run before it is reported as truncated.
    pub time_budget: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocationValidationError {
    Empty,
    InvalidUri,
    NotAbsolute,
    Missing,
    NotDirectory,
    Inaccessible,
    NotMounted(Location),
    Mountable(Location),
    Unavailable(String),
    UnsupportedShorthand(String),
    UnsupportedScheme(String),
    EmbeddedCredential,
    BackendUnavailable(String),
}

impl fmt::Display for LocationValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Enter a location."),
            Self::InvalidUri => formatter.write_str("Enter a valid URI."),
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
            Self::UnsupportedShorthand(message) | Self::UnsupportedScheme(message) => {
                formatter.write_str(message)
            }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UriCredentials {
    pub username: String,
    pub password: String,
}

/// Removes URI user-info secrets while preserving the username separately.
pub fn sanitize_uri_credentials(
    input: &str,
) -> Result<(String, Option<UriCredentials>), LocationValidationError> {
    let uri = glib::Uri::parse(
        input,
        glib::UriFlags::HAS_PASSWORD | glib::UriFlags::HAS_AUTH_PARAMS,
    )
    .map_err(|_| LocationValidationError::InvalidUri)?;
    if !uri_contains_credentials(&uri) {
        return Ok((uri.to_str().to_string(), None));
    }

    let mut username = uri.user().map(|user| user.to_string()).unwrap_or_default();
    let mut password = uri.password().map(|password| password.to_string());
    let mut auth_params = uri.auth_params().map(|params| params.to_string());

    if password.is_none()
        && let Some((user, embedded_password)) = username.split_once(':')
    {
        password = Some(embedded_password.to_owned());
        username = user.to_owned();
    }
    if auth_params.is_none()
        && let Some((user, embedded_params)) = username.split_once(';')
    {
        auth_params = Some(embedded_params.to_owned());
        username = user.to_owned();
    }
    if password.is_none() {
        password = auth_params
            .as_deref()
            .and_then(|params| params.strip_prefix("password="))
            .map(str::to_owned);
    }

    let sanitized = glib::Uri::build_with_user(
        glib::UriFlags::empty(),
        &uri.scheme(),
        (!username.is_empty()).then_some(username.as_str()),
        None,
        None,
        uri.host().as_deref(),
        uri.port(),
        &uri.path(),
        uri.query().as_deref(),
        uri.fragment().as_deref(),
    )
    .to_str()
    .to_string();
    let credentials = UriCredentials {
        username,
        password: password.unwrap_or_default(),
    };
    Ok((sanitized, Some(credentials)))
}

/// Rejects URI password and authentication-parameter fields, including encoded delimiters.
pub fn validate_uri_credentials(uri: &str) -> Result<(), LocationValidationError> {
    match sanitize_uri_credentials(uri)? {
        (_, Some(_)) => Err(LocationValidationError::EmbeddedCredential),
        (_, None) => Ok(()),
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
        /// `true` if the load stopped short of covering the full directory, because it hit the
        /// entry or time budget; already-emitted `Batch` entries are then a lower bound.
        truncated: bool,
        /// Whether this location supports moving entries to Trash, resolved from an
        /// entry in the directory. `None` when the location is empty or the capability
        /// couldn't be answered; treated as "assume trashable" by consumers, since that
        /// matches offering Trash and letting the operation itself fail if unsupported.
        can_trash: Option<bool>,
    },
    /// Consumers must not present these partial values as a completed sort.
    MetadataIncomplete { request_id: RequestId },
    Failed {
        request_id: RequestId,
        message: String,
    },
    /// Size/mtime arrivals for already-listed entries, positioned by the receiver
    /// against stable locations. Zero or more chunks, then exactly one `MetadataFinished`
    /// (dropping the `LoadHandle` first cancels the fill with no terminal event).
    MetadataFilled {
        request_id: RequestId,
        updates: Vec<MetadataUpdate>,
    },
    /// Terminal outcome for a metadata fill: exactly one per fill, including empty fills.
    /// Sorts wait for this event, never for a chunk, so a partial pass can never be
    /// mistaken for a complete one.
    MetadataFinished {
        request_id: RequestId,
        outcome: MetadataOutcome,
    },
}

/// Terminal outcome for a metadata fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataOutcome {
    Complete,
    /// The budget expired partway; emitted chunks still apply, the rest keep placeholders.
    Truncated,
    /// The provider cannot stat these entries; rows keep their placeholders.
    Unsupported,
    /// No trustworthy pass; rows keep placeholders and waiting sorts are abandoned.
    Failed,
    /// The fill was dropped before finishing. Providers never emit this; owners synthesize
    /// it when they discard a fill's handle while a sort still waits on it.
    Cancelled,
}

/// Fresh metadata for one listed entry. Fields may stay `Unknown`/`Unavailable`
/// when the stat failed; the row keeps its placeholder for a later retry.
#[derive(Clone, Debug)]
pub struct MetadataUpdate {
    pub location: Location,
    pub size: MetadataValue<u64>,
    pub modified_unix_seconds: MetadataValue<i64>,
    pub mode: MetadataValue<u32>,
}

#[derive(Clone, Debug)]
pub struct MetadataRequest {
    /// Fills for a superseded load are dropped, never applied to a reloaded column.
    pub id: RequestId,
    pub entries: Vec<Location>,
    /// When true, stat the whole list (a sort's full pass); otherwise a viewport window.
    pub full: bool,
    pub time_budget: Duration,
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

    /// Validates a location without blocking the caller. Providers should override this when
    /// validation can involve network or removable-media I/O.
    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), LocationValidationError>)>,
    ) -> LoadHandle {
        emit(self.validate_location(&location));
        LoadHandle::new(|| {})
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle;
    fn supports_metadata_fill(&self, _location: &Location) -> bool {
        false
    }

    /// Overrides must emit zero or more `MetadataFilled` chunks followed by exactly one
    /// `MetadataFinished` terminal outcome, including for empty fills. Dropping the
    /// returned `LoadHandle` aborts the fill without any terminal event.
    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        emit(DirectoryEvent::MetadataFinished {
            request_id: request.id,
            outcome: MetadataOutcome::Unsupported,
        });
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        _notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        None
    }
}

// SPDX-License-Identifier: MIT

//! Saved remote connections: named destinations stored in Strata's config.
//! The file holds display metadata and a sanitized destination only; secrets
//! stay with GVfs and the desktop keyring.

#[cfg(test)]
mod tests;

use std::{fmt, io, path::PathBuf};

use serde_json::{Map, Value};

use super::remote::{RemoteDestination, RemoteProtocol, webdav_uri_for_endpoint};
use crate::model::Location;

pub const CONNECTIONS_VERSION: u64 = 1;
const CONNECTIONS_FILE: &str = "connections.json";
const MAX_NAME_CHARS: usize = 120;

/// Keys that must never be persisted, even when an unknown field carries them.
const SECRET_FIELDS: [&str; 7] = [
    "password",
    "passphrase",
    "secret",
    "token",
    "auth_token",
    "private_key",
    "credentials",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedConnection {
    pub id: String,
    pub name: String,
    destination: RemoteDestination,
}

impl SavedConnection {
    pub fn destination(&self) -> &RemoteDestination {
        &self.destination
    }

    pub fn protocol(&self) -> RemoteProtocol {
        self.destination.protocol()
    }

    pub fn location(&self) -> Location {
        self.destination.location()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionDraft {
    pub name: String,
    pub destination: RemoteDestination,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionStoreError {
    /// A newer Strata wrote the file; rewriting it could discard its data.
    NewerVersion(u64),
    /// The file exists but can't be read or parsed, so it is left untouched.
    Unreadable(String),
    Duplicate(String),
    NotFound,
    InvalidName(&'static str),
}

impl fmt::Display for ConnectionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NewerVersion(version) => write!(
                formatter,
                "Connections were saved by a newer version of Strata (format {version}). \
                 Update Strata to change them."
            ),
            Self::Unreadable(reason) => write!(
                formatter,
                "The saved connections file couldn't be read, so Strata won't change it: \
                 {reason}"
            ),
            Self::Duplicate(name) => write!(formatter, "“{name}” already connects there."),
            Self::NotFound => formatter.write_str("That connection no longer exists."),
            Self::InvalidName(message) => formatter.write_str(message),
        }
    }
}

#[derive(Clone, Debug)]
enum StoredEntry {
    Known {
        connection: SavedConnection,
        extra: Map<String, Value>,
    },
    /// Entries this version can't interpret are preserved verbatim.
    Unknown(Value),
}

#[derive(Clone, Debug)]
pub struct ConnectionStore {
    entries: Vec<StoredEntry>,
    extra: Map<String, Value>,
    read_only: Option<ConnectionStoreError>,
}

impl Default for ConnectionStore {
    fn default() -> Self {
        Self::empty()
    }
}

impl ConnectionStore {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            extra: Map::new(),
            read_only: None,
        }
    }

    fn unreadable(reason: String) -> Self {
        Self {
            read_only: Some(ConnectionStoreError::Unreadable(reason)),
            ..Self::empty()
        }
    }

    pub fn parse(contents: &[u8]) -> Self {
        let value: Value = match serde_json::from_slice(contents) {
            Ok(value) => value,
            Err(error) => return Self::unreadable(error.to_string()),
        };
        let Value::Object(mut root) = value else {
            return Self::unreadable("expected a JSON object".into());
        };
        // Files written before versioning are read as version 1.
        let version = match root.remove("version") {
            None => 1,
            Some(Value::Number(number)) => match number.as_u64() {
                Some(version) => version,
                None => return Self::unreadable("invalid format version".into()),
            },
            Some(_) => return Self::unreadable("invalid format version".into()),
        };
        let entries = match root.remove("connections") {
            None => Vec::new(),
            Some(Value::Array(entries)) => entries,
            Some(_) => return Self::unreadable("expected a list of connections".into()),
        };
        let entries = entries.into_iter().map(parse_entry).collect();
        let mut store = Self {
            entries,
            extra: root,
            read_only: None,
        };
        if version > CONNECTIONS_VERSION {
            store.read_only = Some(ConnectionStoreError::NewerVersion(version));
        }
        store
    }

    pub fn connections(&self) -> Vec<SavedConnection> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                StoredEntry::Known { connection, .. } => Some(connection.clone()),
                StoredEntry::Unknown(_) => None,
            })
            .collect()
    }

    pub fn read_only_reason(&self) -> Option<&ConnectionStoreError> {
        self.read_only.as_ref()
    }

    fn ensure_writable(&self) -> Result<(), ConnectionStoreError> {
        match &self.read_only {
            Some(reason) => Err(reason.clone()),
            None => Ok(()),
        }
    }

    pub fn duplicate_of(
        &self,
        destination: &RemoteDestination,
        except_id: Option<&str>,
    ) -> Option<SavedConnection> {
        self.connections().into_iter().find(|connection| {
            Some(connection.id.as_str()) != except_id
                && connection.destination.same_destination(destination)
        })
    }

    /// The saved connection whose destination contains `location`, if any.
    pub fn containing(&self, location: &Location) -> Option<SavedConnection> {
        let destination = RemoteDestination::for_location(location)?;
        self.connections()
            .into_iter()
            .find(|connection| destination.is_served_by(&connection.destination))
    }

    pub fn add(&mut self, draft: ConnectionDraft) -> Result<SavedConnection, ConnectionStoreError> {
        self.add_with_id(draft, new_connection_id())
    }

    fn add_with_id(
        &mut self,
        draft: ConnectionDraft,
        id: String,
    ) -> Result<SavedConnection, ConnectionStoreError> {
        self.ensure_writable()?;
        let name = validate_name(&draft.name)?;
        if let Some(existing) = self.duplicate_of(&draft.destination, None) {
            return Err(ConnectionStoreError::Duplicate(existing.name));
        }
        let connection = SavedConnection {
            id,
            name,
            destination: draft.destination,
        };
        self.entries.push(StoredEntry::Known {
            connection: connection.clone(),
            extra: Map::new(),
        });
        Ok(connection)
    }

    pub fn update(
        &mut self,
        id: &str,
        draft: ConnectionDraft,
    ) -> Result<SavedConnection, ConnectionStoreError> {
        self.ensure_writable()?;
        let name = validate_name(&draft.name)?;
        if let Some(existing) = self.duplicate_of(&draft.destination, Some(id)) {
            return Err(ConnectionStoreError::Duplicate(existing.name));
        }
        let connection = self.known_mut(id)?;
        connection.name = name;
        connection.destination = draft.destination;
        Ok(connection.clone())
    }

    pub fn rename(
        &mut self,
        id: &str,
        name: &str,
    ) -> Result<SavedConnection, ConnectionStoreError> {
        self.ensure_writable()?;
        let name = validate_name(name)?;
        let connection = self.known_mut(id)?;
        connection.name = name;
        Ok(connection.clone())
    }

    /// Removes only Strata's record; remote content and keyring secrets are untouched.
    pub fn remove(&mut self, id: &str) -> Result<SavedConnection, ConnectionStoreError> {
        self.ensure_writable()?;
        let index = self
            .entries
            .iter()
            .position(|entry| matches!(entry, StoredEntry::Known { connection, .. } if connection.id == id))
            .ok_or(ConnectionStoreError::NotFound)?;
        match self.entries.remove(index) {
            StoredEntry::Known { connection, .. } => Ok(connection),
            StoredEntry::Unknown(_) => Err(ConnectionStoreError::NotFound),
        }
    }

    fn known_mut(&mut self, id: &str) -> Result<&mut SavedConnection, ConnectionStoreError> {
        self.entries
            .iter_mut()
            .find_map(|entry| match entry {
                StoredEntry::Known { connection, .. } if connection.id == id => Some(connection),
                _ => None,
            })
            .ok_or(ConnectionStoreError::NotFound)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ConnectionStoreError> {
        self.ensure_writable()?;
        let mut root = strip_secret_fields(self.extra.clone());
        root.insert("version".into(), Value::from(CONNECTIONS_VERSION));
        let entries = self
            .entries
            .iter()
            .map(|entry| match entry {
                StoredEntry::Known { connection, extra } => {
                    let mut object = strip_secret_fields(extra.clone());
                    object.insert("id".into(), Value::from(connection.id.clone()));
                    object.insert("name".into(), Value::from(connection.name.clone()));
                    object.insert(
                        "protocol".into(),
                        Value::from(connection.protocol().scheme()),
                    );
                    object.insert(
                        "uri".into(),
                        Value::from(connection.destination.canonical_uri()),
                    );
                    Value::Object(object)
                }
                StoredEntry::Unknown(value) => match value {
                    Value::Object(object) => Value::Object(strip_secret_fields(object.clone())),
                    value => value.clone(),
                },
            })
            .collect();
        root.insert("connections".into(), Value::Array(entries));
        let mut contents = serde_json::to_vec_pretty(&Value::Object(root))
            .map_err(|error| ConnectionStoreError::Unreadable(error.to_string()))?;
        contents.push(b'\n');
        Ok(contents)
    }
}

fn parse_entry(value: Value) -> StoredEntry {
    let Value::Object(mut object) = value else {
        return StoredEntry::Unknown(value);
    };
    let known = (|| {
        let id = object.get("id")?.as_str()?.trim().to_owned();
        let name = object.get("name")?.as_str()?.trim().to_owned();
        let protocol = RemoteProtocol::from_scheme(object.get("protocol")?.as_str()?)?;
        let destination = RemoteDestination::parse(object.get("uri")?.as_str()?)?;
        (!id.is_empty() && !name.is_empty() && destination.protocol() == protocol).then_some(
            SavedConnection {
                id,
                name,
                destination,
            },
        )
    })();
    match known {
        Some(connection) => {
            for key in ["id", "name", "protocol", "uri"] {
                object.remove(key);
            }
            StoredEntry::Known {
                connection,
                extra: object,
            }
        }
        None => StoredEntry::Unknown(Value::Object(object)),
    }
}

fn strip_secret_fields(mut object: Map<String, Value>) -> Map<String, Value> {
    object.retain(|key, _| {
        let key = key.to_ascii_lowercase();
        !SECRET_FIELDS.iter().any(|secret| key.contains(secret))
    });
    object
}

fn validate_name(name: &str) -> Result<String, ConnectionStoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ConnectionStoreError::InvalidName("Enter a name."));
    }
    if name.chars().any(char::is_control) {
        return Err(ConnectionStoreError::InvalidName(
            "Names can't contain line breaks or control characters.",
        ));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ConnectionStoreError::InvalidName("Use a shorter name."));
    }
    Ok(name.to_owned())
}

fn new_connection_id() -> String {
    glib::uuid_string_random().to_string()
}

pub fn connections_path() -> PathBuf {
    crate::storage::config_directory().join(CONNECTIONS_FILE)
}

pub fn load_connection_store() -> ConnectionStore {
    match std::fs::read(connections_path()) {
        Ok(contents) => ConnectionStore::parse(&contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => ConnectionStore::empty(),
        Err(error) => ConnectionStore::unreadable(error.to_string()),
    }
}

pub fn save_connection_store(store: &ConnectionStore) -> Result<(), SaveError> {
    let contents = store.to_json().map_err(SaveError::Store)?;
    let path = connections_path();
    if let Some(parent) = path.parent() {
        create_private_directory(parent).map_err(SaveError::Io)?;
    }
    crate::storage::atomic_write(&path, &contents).map_err(SaveError::Io)
}

fn create_private_directory(path: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

#[derive(Debug)]
pub enum SaveError {
    Store(ConnectionStoreError),
    Io(io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(formatter),
            Self::Io(error) => write!(formatter, "Unable to save connections: {error}"),
        }
    }
}

/// Loads, changes, and saves the shared file so concurrent windows merge
/// rather than overwrite each other's edits.
pub fn update_connections<T>(
    change: impl FnOnce(&mut ConnectionStore) -> Result<T, ConnectionStoreError>,
) -> Result<T, SaveError> {
    let mut store = load_connection_store();
    let value = change(&mut store).map_err(SaveError::Store)?;
    save_connection_store(&store)?;
    Ok(value)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectionForm {
    pub protocol: Option<RemoteProtocol>,
    pub server: String,
    pub port: String,
    pub username: String,
    /// SMB share name.
    pub share: String,
    /// SMB folder inside the share, or the remote path/endpoint for other protocols.
    pub path: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionFormField {
    Server,
    Port,
    Username,
    Share,
    Path,
    Name,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionFormError {
    pub field: ConnectionFormField,
    pub message: String,
}

impl ConnectionFormError {
    fn new(field: ConnectionFormField, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

impl ConnectionForm {
    pub fn from_connection(connection: &SavedConnection) -> Self {
        let destination = connection.destination();
        let protocol = destination.protocol();
        let path = destination.path();
        let (share, path) = if protocol == RemoteProtocol::Smb {
            let trimmed = path.trim_start_matches('/');
            match trimmed.split_once('/') {
                Some((share, folder)) => (share.to_owned(), folder.to_owned()),
                None => (trimmed.to_owned(), String::new()),
            }
        } else {
            (String::new(), path)
        };
        Self {
            protocol: Some(protocol),
            server: destination.host().to_owned(),
            port: destination
                .port()
                .map(|port| port.to_string())
                .unwrap_or_default(),
            username: destination.user().unwrap_or_default(),
            share,
            path,
            name: connection.name.clone(),
        }
    }

    pub fn from_location(location: &Location) -> Option<Self> {
        let destination = RemoteDestination::for_location(location)?;
        let connection = SavedConnection {
            id: String::new(),
            name: String::new(),
            destination,
        };
        Some(Self::from_connection(&connection))
    }

    /// Validates the form and builds a sanitized destination. The display
    /// name falls back to one derived from the destination.
    pub fn validate(&self) -> Result<ConnectionDraft, ConnectionFormError> {
        use ConnectionFormField as Field;

        let mut protocol = self
            .protocol
            .ok_or_else(|| ConnectionFormError::new(Field::Server, "Choose a protocol."))?;
        let mut server = self.server.trim().to_owned();
        let mut endpoint_path = None;
        let mut endpoint_port = None;
        if server.contains("://") {
            let uri = match protocol {
                RemoteProtocol::Dav | RemoteProtocol::Davs => webdav_uri_for_endpoint(&server),
                _ => None,
            }
            .or_else(|| {
                let scheme = glib::Uri::parse_scheme(&server)?;
                (RemoteProtocol::from_scheme(&scheme) == Some(protocol)).then(|| server.clone())
            })
            .ok_or_else(|| {
                ConnectionFormError::new(
                    Field::Server,
                    format!(
                        "Enter only the server name, or a {}:// address.",
                        protocol.scheme()
                    ),
                )
            })?;
            let parsed = glib::Uri::parse(
                &uri,
                glib::UriFlags::ENCODED
                    | glib::UriFlags::PARSE_RELAXED
                    | glib::UriFlags::HAS_PASSWORD
                    | glib::UriFlags::HAS_AUTH_PARAMS,
            )
            .map_err(|_| {
                ConnectionFormError::new(Field::Server, "Enter a valid server address.")
            })?;
            if parsed.password().is_some() || parsed.auth_params().is_some() {
                return Err(ConnectionFormError::new(
                    Field::Server,
                    "Don't include a password. You'll be asked for it when connecting.",
                ));
            }
            protocol = RemoteProtocol::from_scheme(&parsed.scheme()).unwrap_or(protocol);
            server = parsed
                .host()
                .map(|host| host.to_string())
                .unwrap_or_default();
            endpoint_port = u16::try_from(parsed.port()).ok();
            let path = parsed.path().to_string();
            if path.trim_matches('/').is_empty() {
                endpoint_path = None;
            } else {
                endpoint_path = Some(path);
            }
        }
        if server.is_empty() {
            return Err(ConnectionFormError::new(
                Field::Server,
                "Enter a server name.",
            ));
        }
        if server.contains(['/', '@', ' ', '\\', '?', '#']) {
            return Err(ConnectionFormError::new(
                Field::Server,
                "Enter a server name without a path, user, or spaces.",
            ));
        }

        let port = match self.port.trim() {
            "" => endpoint_port,
            port => Some(
                port.parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or_else(|| {
                        ConnectionFormError::new(Field::Port, "Enter a port from 1 to 65535.")
                    })?,
            ),
        };

        let username = self.username.trim();
        if username.contains([':', ';', '@', '/']) || username.chars().any(char::is_whitespace) {
            return Err(ConnectionFormError::new(
                Field::Username,
                "Enter only the username. You'll be asked for the password when connecting.",
            ));
        }

        let path = if protocol == RemoteProtocol::Smb {
            let share = self.share.trim().trim_matches('/');
            if share.is_empty() {
                return Err(ConnectionFormError::new(
                    Field::Share,
                    "Enter a share name.",
                ));
            }
            if share.contains('/') {
                return Err(ConnectionFormError::new(
                    Field::Share,
                    "Enter the share name only. Put folders in Folder.",
                ));
            }
            format!("/{share}/{}", self.path.trim().trim_matches('/'))
        } else {
            match endpoint_path {
                Some(path) if self.path.trim().is_empty() => {
                    glib::Uri::unescape_string(&path, Some("/"))
                        .map(|path| path.to_string())
                        .ok_or_else(|| {
                            ConnectionFormError::new(Field::Server, "Enter a valid server address.")
                        })?
                }
                _ => format!("/{}", self.path.trim().trim_start_matches('/')),
            }
        };
        let encoded_path = glib::Uri::escape_string(&path, Some("/!$&'()*+,;=:@"), true);
        let encoded_user = (!username.is_empty())
            .then(|| glib::Uri::escape_string(username, Some("!$&'()*+,="), true).to_string());
        let destination = RemoteDestination::new(
            protocol,
            &server,
            port,
            encoded_user.as_deref(),
            &encoded_path,
        )
        .ok_or_else(|| {
            if path.split('/').any(|segment| segment == "..") {
                ConnectionFormError::new(Field::Path, "Enter a path without “..”.")
            } else {
                ConnectionFormError::new(Field::Server, "Enter a valid server name.")
            }
        })?;
        let name = match self.name.trim() {
            "" => destination.default_name(),
            name => name.to_owned(),
        };
        validate_name(&name)
            .map_err(|error| ConnectionFormError::new(Field::Name, error.to_string()))?;
        Ok(ConnectionDraft { name, destination })
    }
}

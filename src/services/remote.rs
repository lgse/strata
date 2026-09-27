// SPDX-License-Identifier: MIT

//! Protocol policy for GVfs remote locations: canonical destination identity,
//! mount matching, and the user-facing mapping of discovery, mount, and trust
//! outcomes. Messages built here never repeat hosts, users, paths, or secrets.

#[cfg(test)]
mod tests;

use std::fmt::Write as _;

use gio::{glib, prelude::VfsExt};
use serde::{Deserialize, Serialize};

use crate::model::Location;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteProtocol {
    Smb,
    Sftp,
    Ftp,
    Ftps,
    Dav,
    Davs,
}

impl RemoteProtocol {
    pub const ALL: [Self; 6] = [
        Self::Smb,
        Self::Sftp,
        Self::Ftp,
        Self::Ftps,
        Self::Dav,
        Self::Davs,
    ];

    pub fn from_scheme(scheme: &str) -> Option<Self> {
        Some(match scheme.to_ascii_lowercase().as_str() {
            "smb" => Self::Smb,
            "sftp" => Self::Sftp,
            "ftp" => Self::Ftp,
            "ftps" => Self::Ftps,
            "dav" => Self::Dav,
            "davs" => Self::Davs,
            _ => return None,
        })
    }

    pub fn for_uri(uri: &str) -> Option<Self> {
        glib::Uri::parse_scheme(uri).and_then(|scheme| Self::from_scheme(&scheme))
    }

    pub fn for_location(location: &Location) -> Option<Self> {
        location.uri_value().and_then(Self::for_uri)
    }

    pub fn scheme(self) -> &'static str {
        match self {
            Self::Smb => "smb",
            Self::Sftp => "sftp",
            Self::Ftp => "ftp",
            Self::Ftps => "ftps",
            Self::Dav => "dav",
            Self::Davs => "davs",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Smb => "SMB",
            Self::Sftp => "SFTP",
            Self::Ftp => "FTP",
            Self::Ftps => "FTPS",
            Self::Dav => "WebDAV",
            Self::Davs => "WebDAVS",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Smb => "Windows or Samba share",
            Self::Sftp => "SSH file transfer",
            Self::Ftp => "Unencrypted FTP",
            Self::Ftps => "FTP over TLS",
            Self::Dav => "Unencrypted WebDAV",
            Self::Davs => "WebDAV over HTTPS",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Smb => 445,
            Self::Sftp => 22,
            Self::Ftp | Self::Ftps => 21,
            Self::Dav => 80,
            Self::Davs => 443,
        }
    }

    /// Credentials and file contents cross the network unencrypted.
    pub fn is_plaintext(self) -> bool {
        matches!(self, Self::Ftp | Self::Dav)
    }

    fn backend_name(self) -> &'static str {
        match self {
            Self::Smb => "SMB",
            Self::Sftp => "SFTP",
            Self::Ftp | Self::Ftps => "FTP",
            Self::Dav | Self::Davs => "WebDAV",
        }
    }
}

/// A remote destination reduced to the parts that decide whether two
/// addresses reach the same place. Passwords and URI authentication
/// parameters are never retained.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RemoteDestination {
    protocol: RemoteProtocol,
    host: String,
    port: Option<u16>,
    user: Option<String>,
    path: String,
}

impl RemoteDestination {
    pub fn parse(uri: &str) -> Option<Self> {
        let parsed = glib::Uri::parse(
            uri,
            glib::UriFlags::ENCODED
                | glib::UriFlags::PARSE_RELAXED
                | glib::UriFlags::HAS_PASSWORD
                | glib::UriFlags::HAS_AUTH_PARAMS,
        )
        .ok()?;
        let protocol = RemoteProtocol::from_scheme(&parsed.scheme())?;
        let host = parsed.host()?.to_string();
        let port = u16::try_from(parsed.port()).ok();
        let user = parsed
            .user()
            .map(|user| user.to_string())
            .map(|user| {
                // Relaxed parsing leaves `user:secret` and `user;params` in the user field.
                user.split([':', ';']).next().unwrap_or_default().to_owned()
            })
            .filter(|user| !user.is_empty());
        Self::new(protocol, &host, port, user.as_deref(), &parsed.path())
    }

    pub fn for_location(location: &Location) -> Option<Self> {
        Self::parse(location.uri_value()?)
    }

    /// `path` and `user` may be percent-encoded; both are normalized.
    pub fn new(
        protocol: RemoteProtocol,
        host: &str,
        port: Option<u16>,
        user: Option<&str>,
        path: &str,
    ) -> Option<Self> {
        let host = normalize_host(host)?;
        let port = port.filter(|port| *port != protocol.default_port() && *port != 0);
        let user = user
            .map(normalize_percent_encoding)
            .filter(|user| !user.is_empty());
        Some(Self {
            protocol,
            host,
            port,
            user,
            path: normalize_path(path)?,
        })
    }

    pub fn protocol(&self) -> RemoteProtocol {
        self.protocol
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn user(&self) -> Option<String> {
        self.user
            .as_deref()
            .and_then(|user| glib::Uri::unescape_string(user, None::<&str>))
            .map(|user| user.to_string())
    }

    /// The decoded path, always starting with `/`.
    pub fn path(&self) -> String {
        glib::Uri::unescape_string(&self.path, Some("/"))
            .map(|path| path.to_string())
            .unwrap_or_else(|| self.path.clone())
    }

    pub fn canonical_uri(&self) -> String {
        let mut uri = format!("{}://", self.protocol.scheme());
        if let Some(user) = &self.user {
            uri.push_str(user);
            uri.push('@');
        }
        if self.host.contains(':') {
            let _ = write!(uri, "[{}]", self.host);
        } else {
            uri.push_str(&self.host);
        }
        if let Some(port) = self.port {
            let _ = write!(uri, ":{port}");
        }
        uri.push_str(&self.path);
        uri
    }

    pub fn location(&self) -> Location {
        Location::uri(self.canonical_uri())
    }

    /// Identity used for per-destination decisions such as the plaintext warning.
    pub fn server_key(&self) -> String {
        let mut key = self.canonical_uri();
        key.truncate(key.len() - self.path.len());
        key
    }

    /// The case-insensitive comparison form of the path. SMB share and folder
    /// names are case-insensitive; other protocols compare exactly.
    fn comparable_path(&self) -> String {
        if self.protocol == RemoteProtocol::Smb {
            self.path.to_lowercase()
        } else {
            self.path.clone()
        }
    }

    fn same_server(&self, other: &Self) -> bool {
        self.protocol == other.protocol && self.host == other.host && self.port == other.port
    }

    /// Two saved connections are duplicates only when every identity part matches.
    pub fn same_destination(&self, other: &Self) -> bool {
        self.same_server(other)
            && self.user == other.user
            && self.comparable_path() == other.comparable_path()
    }

    /// Whether a GVfs mount rooted at `root` already serves this destination.
    /// A destination without a user reuses a mount made for any user.
    pub fn is_served_by(&self, root: &Self) -> bool {
        if !self.same_server(root) {
            return false;
        }
        if self.user.is_some() && root.user.is_some() && self.user != root.user {
            return false;
        }
        path_is_within(&self.comparable_path(), &root.comparable_path())
    }

    /// `self` rebased onto the mount root that serves it, keeping the mount's user.
    pub fn on_mount(&self, root: &Self) -> Self {
        Self {
            user: root.user.clone().or_else(|| self.user.clone()),
            ..self.clone()
        }
    }

    pub fn default_name(&self) -> String {
        let path = self.path();
        let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        match segments.last() {
            Some(last) => format!("{last} on {}", self.host),
            None => self.host.clone(),
        }
    }
}

fn path_is_within(path: &str, root: &str) -> bool {
    if root == "/" || path == root {
        return true;
    }
    path.strip_prefix(root)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn normalize_host(host: &str) -> Option<String> {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.');
    if host.is_empty()
        || host
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '/' | '@' | '?' | '#' | '\\'))
    {
        return None;
    }
    Some(normalize_percent_encoding(host).to_lowercase())
}

/// Collapses empty and `.` segments, resolves `..`, and drops trailing separators.
fn normalize_path(path: &str) -> Option<String> {
    let mut segments: Vec<String> = Vec::new();
    for segment in path.split('/') {
        let segment = normalize_percent_encoding(segment);
        match segment.as_str() {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            _ => segments.push(segment),
        }
    }
    Some(format!("/{}", segments.join("/")))
}

/// Decodes unreserved characters, uppercases remaining escapes, and escapes
/// bytes that may not appear literally in a URI component.
fn normalize_percent_encoding(component: &str) -> String {
    let bytes = component.as_bytes();
    let mut normalized = String::with_capacity(component.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%'
            && let Some(decoded) = bytes
                .get(index + 1..index + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            push_uri_byte(&mut normalized, decoded, true);
            index += 3;
            continue;
        }
        push_uri_byte(&mut normalized, byte, false);
        index += 1;
    }
    normalized
}

fn push_uri_byte(output: &mut String, byte: u8, was_escaped: bool) {
    let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
    let literal_allowed = !was_escaped
        && matches!(
            byte,
            b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b'=' | b':' | b'@'
        );
    if unreserved || literal_allowed {
        output.push(byte as char);
    } else {
        let _ = write!(output, "%{byte:02X}");
    }
}

/// Accepts `http(s)://` endpoints for WebDAV and returns the GVfs scheme.
pub fn webdav_uri_for_endpoint(endpoint: &str) -> Option<String> {
    let scheme = glib::Uri::parse_scheme(endpoint)?;
    let rest = &endpoint[scheme.len()..];
    let target = match scheme.to_ascii_lowercase().as_str() {
        "https" | "davs" => "davs",
        "http" | "dav" => "dav",
        _ => return None,
    };
    Some(format!("{target}{rest}"))
}

/// A hint for web addresses typed where file locations are expected. Web
/// URLs are never reinterpreted as WebDAV implicitly.
pub fn web_address_hint(scheme: &str) -> Option<&'static str> {
    matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https").then_some(
        "Web addresses aren't file locations. To browse a WebDAV server, use davs:// \
         (or dav:// for an unencrypted server), or add it with Add connection.",
    )
}

/// Builds a "this backend isn't installed" message that names the missing
/// GVfs support without assuming one distribution's package names.
pub fn backend_unavailable_message(uri: &str) -> String {
    let scheme = glib::Uri::parse_scheme(uri)
        .map(|scheme| scheme.to_ascii_lowercase())
        .unwrap_or_else(|| uri.split("://").next().unwrap_or(uri).to_owned());
    match RemoteProtocol::from_scheme(&scheme) {
        Some(RemoteProtocol::Smb) => format!(
            "The {scheme}:// backend isn't installed. Install GVfs SMB support \
             (for example gvfs-smb on Arch Linux and Fedora, or gvfs-backends on \
             Debian and Ubuntu) to connect to {scheme}:// locations."
        ),
        Some(protocol) => format!(
            "The {scheme}:// backend isn't installed. Install GVfs {} support \
             (for example gvfs on Arch Linux and Fedora, or gvfs-backends on Debian \
             and Ubuntu) to connect to {scheme}:// locations.",
            protocol.backend_name()
        ),
        None => format!(
            "The {scheme}:// backend isn't installed on this system, so {scheme}:// \
             locations can't be opened."
        ),
    }
}

pub const NETWORK_ROOT_URI: &str = "network:///";

pub fn is_network_root(location: &Location) -> bool {
    location
        .uri_value()
        .is_some_and(|uri| uri.trim_end_matches('/') == "network:")
}

const DIRECT_ENTRY_HINT: &str =
    "Press Ctrl+L to enter a server address directly, such as smb://server/share or sftp://server/folder.";

/// Schemes whose GVfs backends advertise hosts and shares under `network:///`.
const DISCOVERY_SCHEMES: [&str; 3] = ["dns-sd", "smb", "wsdd"];

pub fn discovery_backends_available<S: AsRef<str>>(supported_schemes: &[S]) -> bool {
    supported_schemes
        .iter()
        .any(|scheme| DISCOVERY_SCHEMES.contains(&scheme.as_ref()))
}

pub fn directory_empty_text<S: AsRef<str>>(
    location: Option<&Location>,
    supported_schemes: &[S],
) -> String {
    if !location.is_some_and(is_network_root) {
        return "This directory is empty".into();
    }
    if discovery_backends_available(supported_schemes) {
        format!(
            "No computers or shared folders were found on this network.\nSome servers don't \
             advertise themselves. {DIRECT_ENTRY_HINT}"
        )
    } else {
        format!(
            "Network discovery isn't available because no GVfs discovery backend is \
             installed (DNS-SD, SMB, or WS-Discovery).\n{DIRECT_ENTRY_HINT}"
        )
    }
}

pub fn directory_failure_text(location: Option<&Location>, message: &str) -> String {
    if location.is_some_and(is_network_root) {
        return message.to_owned();
    }
    let heading = if location.is_some_and(|location| RemoteProtocol::for_location(location).is_some())
        && message == RemoteFailure::Disconnected.guidance(None)
    {
        "This location is unavailable"
    } else {
        "Unable to read this directory"
    };
    format!("{heading}\n{message}")
}

/// The actionable message for a failed directory load. Remote failures are
/// sanitized; local failures keep their GIO description.
pub fn load_failure_message(location: &Location, error: &glib::Error) -> String {
    if is_network_root(location) {
        return network_discovery_failure(error);
    }
    let Some(protocol) = RemoteProtocol::for_location(location) else {
        return error.to_string();
    };
    match classify_remote_error(error, RemoteErrorContext::Browse) {
        RemoteFailure::BackendMissing if !scheme_is_supported(protocol.scheme()) => {
            backend_unavailable_message(location.uri_value().unwrap_or_default())
        }
        RemoteFailure::BackendMissing => redact_endpoints(&error.to_string()),
        RemoteFailure::Other => redact_endpoints(&error.to_string()),
        failure => failure.guidance(Some(protocol)),
    }
}

fn network_discovery_failure(error: &glib::Error) -> String {
    if error.matches(gio::IOErrorEnum::NotSupported) {
        format!(
            "Network discovery isn't available because GVfs network support isn't \
             installed or running.\n{DIRECT_ENTRY_HINT}"
        )
    } else {
        format!(
            "Network discovery failed or was blocked. Check your network connection and \
             firewall, then try again.\n{DIRECT_ENTRY_HINT}"
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteErrorContext {
    Mount,
    Browse,
    Unmount,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteFailure {
    Cancelled,
    AuthenticationFailed,
    HostNotFound,
    ConnectionRefused,
    TimedOut,
    PermissionDenied,
    HostKeyRejected,
    CertificateRejected,
    BackendMissing,
    Disconnected,
    Busy,
    Other,
}

impl RemoteFailure {
    pub fn title(self) -> &'static str {
        match self {
            Self::Cancelled => "Connection cancelled",
            Self::AuthenticationFailed => "Sign-in failed",
            Self::HostNotFound => "Server not found",
            Self::ConnectionRefused => "Connection refused",
            Self::TimedOut => "Connection timed out",
            Self::PermissionDenied => "Permission denied",
            Self::HostKeyRejected => "Server identity changed",
            Self::CertificateRejected => "Certificate not trusted",
            Self::BackendMissing => "Protocol support missing",
            Self::Disconnected => "Disconnected",
            Self::Busy => "Connection in use",
            Self::Other => "Unable to connect",
        }
    }

    pub fn guidance(self, protocol: Option<RemoteProtocol>) -> String {
        match self {
            Self::Cancelled => "The connection was cancelled.".into(),
            Self::AuthenticationFailed => {
                "The server didn't accept those credentials. Check the username and \
                 password, then try again."
                    .into()
            }
            Self::HostNotFound => "Strata couldn't find that server. Check the server name \
                 and your network connection, then try again."
                .into(),
            Self::ConnectionRefused => format!(
                "The server refused the connection. Check the port number and that the {} \
                 service is running on the server.",
                protocol.map_or("file sharing", RemoteProtocol::label)
            ),
            Self::TimedOut => "The server didn't respond in time. Check your network \
                 connection, or try again later."
                .into(),
            Self::PermissionDenied => {
                "You don't have permission to open this location on the server.".into()
            }
            Self::HostKeyRejected => "The server's identity doesn't match the key saved for \
                 it, so Strata didn't connect. The server may have been reinstalled, or \
                 someone may be intercepting the connection. Confirm the new key with the \
                 server's administrator before removing the old entry from \
                 ~/.ssh/known_hosts."
                .into(),
            Self::CertificateRejected => "The server's security certificate couldn't be \
                 verified, so Strata didn't connect. Check the address, and ask the \
                 server's administrator to install a valid certificate."
                .into(),
            Self::BackendMissing => "Support for this protocol isn't installed.".into(),
            Self::Disconnected => {
                "This location was disconnected. Use Retry to reconnect.".into()
            }
            Self::Busy => "Files on this connection are still in use. Close them, then try \
                 again."
                .into(),
            Self::Other => "Strata couldn't complete the request.".into(),
        }
    }
}

pub fn classify_remote_error(error: &glib::Error, context: RemoteErrorContext) -> RemoteFailure {
    use gio::IOErrorEnum as Io;

    let message = error.message().to_ascii_lowercase();
    let mentions = |needles: &[&str]| needles.iter().any(|needle| message.contains(needle));
    if error.matches(Io::Cancelled) || error.matches(Io::FailedHandled) {
        return RemoteFailure::Cancelled;
    }
    if error.matches(Io::NotSupported) && context != RemoteErrorContext::Unmount {
        return RemoteFailure::BackendMissing;
    }
    if mentions(&["host key verification failed", "host key", "remote host identification"]) {
        return RemoteFailure::HostKeyRejected;
    }
    if error.kind::<gio::TlsError>().is_some()
        || mentions(&["certificate", "tls handshake", "ssl handshake"])
    {
        return RemoteFailure::CertificateRejected;
    }
    if error.matches(Io::HostNotFound)
        || error.matches(Io::HostUnreachable)
        || error.matches(Io::NetworkUnreachable)
        || mentions(&["hostname not known", "no route to host", "name or service not known"])
    {
        return RemoteFailure::HostNotFound;
    }
    if error.matches(Io::ConnectionRefused) || mentions(&["connection refused"]) {
        return RemoteFailure::ConnectionRefused;
    }
    if error.matches(Io::TimedOut) || mentions(&["timed out", "timeout"]) {
        return RemoteFailure::TimedOut;
    }
    if error.matches(Io::Busy) {
        return RemoteFailure::Busy;
    }
    if error.matches(Io::NotMounted)
        || error.matches(Io::Closed)
        || error.matches(Io::BrokenPipe)
        || mentions(&["connection is closed", "connection closed"])
    {
        return RemoteFailure::Disconnected;
    }
    let authentication = mentions(&[
        "authentication failed",
        "too many authentication failures",
        "logon failure",
        "invalid credentials",
        "login incorrect",
        "login failed",
        "unauthorized",
    ]);
    if authentication {
        return RemoteFailure::AuthenticationFailed;
    }
    if error.matches(Io::PermissionDenied) || mentions(&["permission denied", "access denied"]) {
        return match context {
            RemoteErrorContext::Mount => RemoteFailure::AuthenticationFailed,
            _ => RemoteFailure::PermissionDenied,
        };
    }
    RemoteFailure::Other
}

/// Replaces URIs in backend text with their scheme so hosts, users, and paths
/// don't reach dialogs or logs through an unrecognized error.
pub fn redact_endpoints(message: &str) -> String {
    message
        .split(' ')
        .map(|word| match word.find("://") {
            Some(index) => format!("{}://…", &word[..index]),
            None => word.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountQuestionKind {
    /// SSH host keys, including a key that differs from the one for its address.
    HostIdentity,
    Certificate,
    /// Applications keep files open on a connection being disconnected.
    Busy,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountQuestion {
    pub kind: MountQuestionKind,
    pub detail: String,
    pub choices: Vec<String>,
}

impl MountQuestion {
    /// GVfs sends trust questions in the backend's locale; unrecognized ones are
    /// still shown verbatim with every backend choice.
    pub fn classify(message: &str, choices: &[String]) -> Self {
        let lower = message.to_lowercase();
        let kind = if lower.contains("certificate") {
            MountQuestionKind::Certificate
        } else if lower.contains("identity")
            || lower.contains("host key")
            || lower.contains("fingerprint")
        {
            MountQuestionKind::HostIdentity
        } else {
            MountQuestionKind::Other
        };
        let mut lines = message.trim().lines();
        let first = lines.clone().next().unwrap_or_default();
        let detail = if kind != MountQuestionKind::Other
            && first.to_lowercase().contains("verification failed")
        {
            lines.next();
            lines.collect::<Vec<_>>().join("\n").trim().to_owned()
        } else {
            message.trim().to_owned()
        };
        Self {
            kind,
            detail,
            choices: choices.to_vec(),
        }
    }

    /// GIO's `show-processes` prompt while disconnecting a busy mount.
    pub fn busy(message: &str, choices: &[String]) -> Self {
        Self {
            kind: MountQuestionKind::Busy,
            detail: message.trim().to_owned(),
            choices: choices.to_vec(),
        }
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            MountQuestionKind::HostIdentity => "Verify the server's identity",
            MountQuestionKind::Certificate => "Untrusted certificate",
            MountQuestionKind::Busy => "Connection in use",
            MountQuestionKind::Other => "The server needs a decision",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self.kind {
            MountQuestionKind::HostIdentity => {
                "Continue only if you can confirm this key with the server's administrator"
            }
            MountQuestionKind::Certificate => {
                "Continue only if you trust this certificate and expected this warning"
            }
            MountQuestionKind::Busy => {
                "Disconnecting now can lose unsaved changes in other applications"
            }
            MountQuestionKind::Other => "Choose how the connection should continue",
        }
    }

    /// Trust questions put acceptance first; that choice is styled as dangerous
    /// and is never focused or chosen by default.
    pub fn is_risky_choice(&self, index: usize) -> bool {
        self.kind != MountQuestionKind::Other && index == 0 && self.choices.len() > 1
    }
}

/// The mount operation outcome as a user decision rather than a GIO error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountResolution {
    Mounted,
    Cancelled,
    Failed(RemoteFailure),
}

/// `declined` records that the user cancelled a prompt or rejected a trust
/// question during this operation. Some backends report that choice as an
/// ordinary failure ("Login dialog cancelled", "Host key verification failed").
/// `backend_available` distinguishes a missing GVfs backend from a server
/// that rejected a command, which GVfs also reports as "not supported".
pub fn resolve_mount_result(
    result: &Result<(), glib::Error>,
    declined: bool,
    context: RemoteErrorContext,
    backend_available: bool,
) -> MountResolution {
    match result {
        Ok(()) => MountResolution::Mounted,
        Err(error) if error.matches(gio::IOErrorEnum::AlreadyMounted) => MountResolution::Mounted,
        Err(_) if declined => MountResolution::Cancelled,
        Err(error) => match classify_remote_error(error, context) {
            RemoteFailure::Cancelled => MountResolution::Cancelled,
            RemoteFailure::BackendMissing if backend_available => {
                MountResolution::Failed(RemoteFailure::Other)
            }
            failure => MountResolution::Failed(failure),
        },
    }
}

/// Whether this process's GIO VFS can open `scheme` locations.
pub fn scheme_is_supported(scheme: &str) -> bool {
    gio::Vfs::default()
        .supported_uri_schemes()
        .iter()
        .any(|supported| supported.eq_ignore_ascii_case(scheme))
}

/// The plaintext protocol that needs a transport warning before a first
/// connection, or `None` for encrypted and non-remote locations.
pub fn plaintext_destination(location: &Location) -> Option<RemoteDestination> {
    RemoteDestination::for_location(location)
        .filter(|destination| destination.protocol().is_plaintext())
}

pub fn plaintext_warning_text(protocol: RemoteProtocol) -> String {
    format!(
        "{} doesn't encrypt the connection. Your username, password, and the files you open \
         can be read or changed by anyone on the network path to this server. This warning \
         is about transport security only; it doesn't affect certificate checks for \
         encrypted servers.",
        protocol.label()
    )
}

pub fn plaintext_secure_alternative(protocol: RemoteProtocol) -> Option<RemoteProtocol> {
    match protocol {
        RemoteProtocol::Ftp => Some(RemoteProtocol::Ftps),
        RemoteProtocol::Dav => Some(RemoteProtocol::Davs),
        _ => None,
    }
}

/// Network presentation for a GVfs mount root, independent of protocol.
pub fn mount_protocol(root_uri: &str) -> Option<RemoteProtocol> {
    RemoteProtocol::for_uri(root_uri)
}

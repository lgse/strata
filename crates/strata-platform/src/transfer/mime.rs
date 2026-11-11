//! The MIME formats file managers exchange for copied and dragged files: `text/uri-list`
//! (RFC 2483), `x-special/gnome-copied-files` and `application/x-kde-cutselection`, plus the
//! X11 target names that stand for UTF-8 text.
//!
//! The encoders write what GTK writes: `file://` URIs escaped as GLib's `g_filename_to_uri`
//! escapes them, so a payload made here matches one made by a GTK file manager byte for byte.
//! The decoders read what other toolkits write too (Qt escapes less, some apps end lines in LF
//! or add a NUL).
//!
//! Which format wins when several are offered, and what a cut means, is the app's policy.

use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

pub const URI_LIST: &str = "text/uri-list";
/// `copy` or `cut`, then one URI per line, with no trailing newline.
pub const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
/// `1` marks the `text/uri-list` beside it as cut, `0` as copied.
pub const KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";
pub const TEXT_UTF8: &str = "text/plain;charset=utf-8";

/// Bytes a path keeps unescaped in a `file://` URI: RFC 3986's unreserved characters plus the
/// sub-delimiters GLib allows in a path (`G_URI_RESERVED_CHARS_ALLOWED_IN_PATH` without `;`,
/// which `g_filename_to_uri` escapes).
fn keeps(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~!$&'()*+,=:@/".contains(&b)
}

/// The `file://` URI of an absolute path, escaped byte by byte: a path that isn't UTF-8
/// still gets a URI (`/tmp/\xff` is `file:///tmp/%FF`).
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if keeps(b) {
            uri.push(char::from(b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// What a URI in a file list names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileUri {
    /// A `file://` URI on this machine: an empty host or `localhost`.
    Local(PathBuf),
    /// A `file://` URI that names another host.
    Remote { host: String, path: PathBuf },
    /// Any other scheme (`smb://`, `https://`), kept as it is.
    Other(String),
}

/// Why a `file://` URI was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UriError {
    /// `%` not followed by two hex digits.
    BadEscape,
    /// An escaped `/` or NUL, which no path can hold.
    EscapedSeparator,
    /// No scheme, or a `file:` URI without an absolute path.
    NotAUri,
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UriError::BadEscape => "a % escape without two hex digits",
            UriError::EscapedSeparator => "an escaped / or NUL",
            UriError::NotAUri => "not a URI with an absolute path",
        })
    }
}

impl std::error::Error for UriError {}

/// Reads one URI of a file list. Takes `file:///p`, `file://localhost/p` and the short
/// `file:/p`, as GLib does, and decodes escapes to raw bytes, so the path needn't be UTF-8.
pub fn file_path(uri: &str) -> Result<FileUri, UriError> {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return Err(UriError::NotAUri);
    };
    let scheme_ok = scheme
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b));
    if !scheme_ok {
        return Err(UriError::NotAUri);
    }
    if !scheme.eq_ignore_ascii_case("file") {
        return Ok(FileUri::Other(uri.to_owned()));
    }
    let (host, path) = match rest.strip_prefix("//") {
        Some(authority) => match authority.find('/') {
            Some(i) => (&authority[..i], &authority[i..]),
            None => return Err(UriError::NotAUri),
        },
        None if rest.starts_with('/') => ("", rest),
        None => return Err(UriError::NotAUri),
    };
    // A query or fragment isn't part of the path.
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let path = PathBuf::from(OsString::from_vec(unescape(path)?));
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        Ok(FileUri::Local(path))
    } else {
        Ok(FileUri::Remote {
            host: host.to_owned(),
            path,
        })
    }
}

fn unescape(s: &str) -> Result<Vec<u8>, UriError> {
    let hex = |b: u8| char::from(b).to_digit(16);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            ) else {
                return Err(UriError::BadEscape);
            };
            let b = (hi * 16 + lo) as u8;
            if b == b'/' || b == 0 {
                return Err(UriError::EscapedSeparator);
            }
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(out)
}

/// A `text/uri-list` payload: each URI followed by CRLF, the last one too.
pub fn encode_uri_list<S: AsRef<str>>(uris: &[S]) -> Vec<u8> {
    let mut out = Vec::new();
    for uri in uris {
        out.extend_from_slice(uri.as_ref().as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// The URIs of a `text/uri-list`: lines ending in CRLF or LF, with `#` comment lines, blank
/// lines and NUL padding skipped. Lines that aren't UTF-8 are skipped too: a URI is ASCII.
pub fn decode_uri_list(payload: &[u8]) -> Vec<String> {
    lines(payload)
        .filter(|l| !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// Non-empty lines of a payload, without CR, LF or NUL padding.
fn lines(payload: &[u8]) -> impl Iterator<Item = &str> {
    payload
        .split(|b| *b == b'\n')
        .map(|l| {
            let end = l
                .iter()
                .rposition(|b| !matches!(b, b'\r' | b'\0'))
                .map_or(0, |i| i + 1);
            &l[..end]
        })
        .filter_map(|l| std::str::from_utf8(l).ok())
        .map(str::trim)
        .filter(|l| !l.is_empty())
}

/// An `x-special/gnome-copied-files` payload: `copy` or `cut`, then `\n` and a URI for each
/// file, with no trailing newline.
pub fn encode_gnome_copied_files<S: AsRef<str>>(cut: bool, uris: &[S]) -> Vec<u8> {
    let mut out = Vec::from(if cut { &b"cut"[..] } else { &b"copy"[..] });
    for uri in uris {
        out.push(b'\n');
        out.extend_from_slice(uri.as_ref().as_bytes());
    }
    out
}

/// Reads an `x-special/gnome-copied-files` payload: whether it is a cut, and its URIs. None
/// when the first line is neither `copy` nor `cut`, or no URI follows. CRLF, a trailing newline
/// and NUL padding read the same as plain newlines.
pub fn decode_gnome_copied_files(payload: &[u8]) -> Option<(bool, Vec<String>)> {
    let mut lines = lines(payload);
    let cut = match lines.next()? {
        "cut" => true,
        "copy" => false,
        _ => return None,
    };
    let uris: Vec<String> = lines.map(str::to_owned).collect();
    (!uris.is_empty()).then_some((cut, uris))
}

/// An `application/x-kde-cutselection` payload.
pub fn encode_kde_cut(cut: bool) -> Vec<u8> {
    Vec::from(if cut { &b"1"[..] } else { &b"0"[..] })
}

/// Whether an `application/x-kde-cutselection` payload marks a cut: its first byte is `1`.
pub fn decode_kde_cut(payload: &[u8]) -> bool {
    payload.first() == Some(&b'1')
}

/// X11 targets that name no data type but a request about the selection (ICCCM 2.6.2, and
/// the clipboard managers' `SAVE_TARGETS`): an offer lists them, but nothing reads them as
/// data, and a copy can't offer them. The list xfsettingsd's clipboard manager excludes, less
/// `PIXMAP`, which is data.
pub const X11_META_TARGETS: &[&str] = &[
    "TARGETS",
    "TIMESTAMP",
    "MULTIPLE",
    "SAVE_TARGETS",
    "DELETE",
    "INSERT_SELECTION",
    "INSERT_PROPERTY",
];

/// Whether `name` is one of [`X11_META_TARGETS`].
pub fn is_x11_meta(name: &str) -> bool {
    X11_META_TARGETS.contains(&name)
}

/// The X11 target names that stand for the same data as `mime`, for a source that also wants
/// to serve X11 clients that ask by the old names. `text/plain;charset=utf-8` is
/// `UTF8_STRING`. `text/plain` is `STRING` and `TEXT`, but ICCCM makes `STRING` ISO Latin-1
/// (and `TEXT` the owner's choice of encoding): serve the `text/plain` bytes under those names
/// only when they are ASCII, or convert them. Other types have none.
pub fn x11_aliases(mime: &str) -> &'static [&'static str] {
    let lower = mime.to_ascii_lowercase().replace(' ', "");
    match lower.as_str() {
        "text/plain;charset=utf-8" => &["UTF8_STRING"],
        "text/plain" => &["STRING", "TEXT"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests;

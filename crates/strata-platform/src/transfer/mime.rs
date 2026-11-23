// SPDX-License-Identifier: MIT

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

pub fn names(listed: &str, offered: &str) -> bool {
    offered == listed
        || (!listed.contains(';')
            && offered
                .strip_prefix(listed)
                .is_some_and(|rest| rest.trim_start().starts_with(';')))
}

fn keeps(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~!$&'()*+,=:@/".contains(&b)
}

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

pub fn encode_gnome_copied_files<S: AsRef<str>>(cut: bool, uris: &[S]) -> Vec<u8> {
    let mut out = Vec::from(if cut { &b"cut"[..] } else { &b"copy"[..] });
    for uri in uris {
        out.push(b'\n');
        out.extend_from_slice(uri.as_ref().as_bytes());
    }
    out
}

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

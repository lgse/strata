//! `text/uri-list` (RFC 2483) and `file:` URIs (RFC 8089), as drags carry files.

use std::path::{Path, PathBuf};

/// A `file:` URI for `path`, percent-encoding every byte but unreserved characters and `/`.
///
/// An absolute path gives `file:///…`. A relative one (headless scripts drop `a.png`) gives
/// `file:a.png`, which [`uri_to_path`] turns back into the same relative path, so the
/// conversion reads no working directory.
///
/// Used by: the [`Dnd::rune`](super::Dnd::rune) binder, X2, and the gallery hosts of C1e, C6,
/// C7 and C8.
pub fn file_uri(path: &Path) -> String {
    let bytes = path_bytes(path);
    let mut uri = String::with_capacity(bytes.len() + 8);
    uri.push_str(if path.is_absolute() {
        "file://"
    } else {
        "file:"
    });
    for &b in bytes.as_ref() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(char::from(b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// The URIs in a `text/uri-list`: one per line, with `#` comment lines, blank lines and line
/// ends (`\r\n` or `\n`) left out. Bytes that aren't UTF-8 are replaced.
///
/// Used by: [`DropData::uris`](super::DropData::uris), X2.
pub fn parse_uri_list(list: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(list)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// The local path a `file:` URI names, or `None` for another scheme, a remote host, a bad
/// percent escape (anything but `%` and two hex digits) or an escaped NUL (`%00`), which no
/// path holds. Takes `file:///p`, `file://localhost/p`, `file:/p` and the relative `file:p`
/// that [`file_uri`] writes. A query or fragment (`?…`, `#…`) is no part of the path and is
/// left out; a `?` or `#` in a name arrives escaped.
///
/// Used by: [`DropData::file_paths`](super::DropData::file_paths), the
/// [`Dnd::rune`](super::Dnd::rune) binder, X2.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let scheme = uri.get(..5)?;
    if !scheme.eq_ignore_ascii_case("file:") {
        return None;
    }
    let rest = &uri[5..];
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let path = match rest.strip_prefix("//") {
        Some(authority) => {
            let slash = authority.find('/')?;
            let host = &authority[..slash];
            if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
                return None;
            }
            &authority[slash..]
        }
        None => rest,
    };
    if path.is_empty() {
        return None;
    }
    Some(bytes_path(percent_decode(path)?))
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex_digit(*bytes.get(i + 1)?)?;
            let lo = hex_digit(*bytes.get(i + 2)?)?;
            let byte = (hi << 4) | lo;
            if byte == 0 {
                return None;
            }
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

/// The value of one ASCII hex digit.
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> std::borrow::Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    std::borrow::Cow::Borrowed(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> std::borrow::Cow<'_, [u8]> {
    match path.to_string_lossy() {
        std::borrow::Cow::Borrowed(s) => std::borrow::Cow::Borrowed(s.as_bytes()),
        std::borrow::Cow::Owned(s) => std::borrow::Cow::Owned(s.into_bytes()),
    }
}

#[cfg(unix)]
fn bytes_path(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn bytes_path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

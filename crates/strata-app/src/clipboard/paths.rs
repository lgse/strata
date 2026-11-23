// SPDX-License-Identifier: MIT

use std::{collections::HashSet, os::unix::ffi::OsStrExt, path::Path};

use gio::prelude::*;
use strata_core::model::Location;
use strata_fs::adapters::{gio_file_for_location, location_for_file};
use strata_platform::clipboard::Files;

pub(crate) fn is_trash_root(location: &Location) -> bool {
    location.uri_value() == Some("trash:///")
}

pub(crate) fn is_trash(location: &Location) -> bool {
    location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
}

pub(crate) fn is_trash_item(location: &Location) -> bool {
    is_trash(location) && location.parent().as_ref().is_some_and(is_trash_root)
}

/// GVfs can move/delete whole trashed items, but cannot modify their children.
pub(crate) fn can_remove(location: &Location) -> bool {
    !is_trash(location) || is_trash_item(location)
}

/// Location equality that also accepts GIO-level equivalence (`file://` URI vs native path).
pub(crate) fn locations_equal(left: &Location, right: &Location) -> bool {
    left == right || gio_file_for_location(left).equal(&gio_file_for_location(right))
}

pub(crate) fn same_locations(left: &[Location], right: &[Location]) -> bool {
    if left.is_empty() || left.len() != right.len() {
        return false;
    }
    let left_set: HashSet<_> = left.iter().collect();
    let right_set: HashSet<_> = right.iter().collect();
    if left_set.len() == right_set.len() && left_set == right_set {
        return true;
    }
    let mut used = vec![false; right.len()];
    left.iter().all(|location| {
        let found = right
            .iter()
            .enumerate()
            .find(|(i, candidate)| !used[*i] && locations_equal(location, candidate));
        match found {
            Some((index, _)) => {
                used[index] = true;
                true
            }
            None => false,
        }
    })
}

pub(crate) fn retain_untransferred(cut: &mut Vec<Location>, transferred: &[Location]) {
    cut.retain(|location| {
        !transferred
            .iter()
            .any(|moved| locations_equal(location, moved))
    });
}

pub(crate) fn files_for_locations(locations: &[Location], cut: bool) -> Option<(Files, String)> {
    if locations.is_empty() {
        return None;
    }
    let uris: Vec<String> = locations
        .iter()
        .map(|loc| gio_file_for_location(loc).uri().into())
        .collect();
    let text = locations
        .iter()
        .map(Location::display_path)
        .collect::<Vec<_>>()
        .join("\n");
    Some((Files::new(uris, cut), text))
}

pub(crate) fn locations_from_files(files: &Files) -> Vec<Location> {
    files
        .uris
        .iter()
        .filter_map(|uri| location_for_file(&gio::File::for_uri(uri)))
        .collect()
}

pub(crate) fn copy_path_text(location: &Location, is_directory: bool) -> String {
    match location.native_path() {
        Some(path) => {
            let mut text = shell_escape_path(path);
            if is_directory && !text.ends_with(std::path::MAIN_SEPARATOR) {
                text.push(std::path::MAIN_SEPARATOR);
            }
            text
        }
        None => location.display_path(),
    }
}

fn shell_escape_path(path: &Path) -> String {
    let bytes = path.as_os_str().as_bytes();
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes
            .split(|&byte| byte == b'/')
            .map(escape_bytes_segment)
            .collect::<Vec<_>>()
            .join("/");
    };
    if text.contains('\n') {
        return format!("'{}'", text.replace('\'', "'\\''"));
    }
    backslash_escape(text)
}

fn escape_bytes_segment(segment: &[u8]) -> String {
    match std::str::from_utf8(segment) {
        Ok(text) if !text.contains('\n') => backslash_escape(text),
        _ => ansi_c_quote(segment),
    }
}

fn backslash_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if needs_shell_escape(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

fn ansi_c_quote(bytes: &[u8]) -> String {
    let mut quoted = String::from("$'");
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            append_quoted_char(&mut quoted, c);
        }
        push_hex_escapes(&mut quoted, chunk.invalid());
    }
    quoted.push('\'');
    quoted
}

fn append_quoted_char(quoted: &mut String, c: char) {
    match c {
        '\'' | '\\' => {
            quoted.push('\\');
            quoted.push(c);
        }
        c if c.is_control() => push_hex_escapes(quoted, c.encode_utf8(&mut [0; 4]).as_bytes()),
        c => quoted.push(c),
    }
}

fn push_hex_escapes(text: &mut String, bytes: &[u8]) {
    use std::fmt::Write;
    for byte in bytes {
        let _ = write!(text, "\\x{byte:02x}");
    }
}

fn needs_shell_escape(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '"' | '\''
                | '\\'
                | '$'
                | '`'
                | '!'
                | '#'
                | '&'
                | '*'
                | ';'
                | '<'
                | '>'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '('
                | ')'
                | '|'
                | '~'
        )
}

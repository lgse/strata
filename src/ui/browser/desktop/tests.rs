// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::{FileEntry, Location};
use std::path::Path;

#[test]
fn terminal_shortcut_prefers_one_selected_directory() {
    let entry = |name: &str, kind| FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        thumbnail_path: None,
        display_name: name.into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
    };
    let directory = entry("selected", crate::model::EntryKind::Directory);
    let file = entry("notes.txt", crate::model::EntryKind::File);

    assert_eq!(
        selected_terminal_location(std::slice::from_ref(&directory)),
        Some(directory.location.clone())
    );
    assert_eq!(selected_terminal_location(&[directory, file.clone()]), None);
    assert_eq!(selected_terminal_location(&[file]), None);
    assert_eq!(selected_terminal_location(&[]), None);
}

#[cfg(unix)]
#[test]
fn terminal_directory_argument_preserves_native_path_bytes() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let path = Path::new(OsStr::from_bytes(b"/tmp/non-utf8-\xff"));

    assert_eq!(
        terminal_directory_argument(path).as_encoded_bytes(),
        b"--dir=/tmp/non-utf8-\xff"
    );
}

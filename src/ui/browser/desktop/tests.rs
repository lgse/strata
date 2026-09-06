// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::{FileEntry, Location};
use std::path::Path;

#[test]
fn executable_fallback_requires_regular_executable_and_missing_handler()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempfile::tempdir()?;
    let program = fixture.path().join("program");
    std::fs::write(&program, b"#!/bin/sh\n")?;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))?;
    let no_handler = glib::Error::new(gio::IOErrorEnum::NotSupported, "no handler");
    let denied = glib::Error::new(gio::IOErrorEnum::PermissionDenied, "denied");

    assert!(executable_without_handler(Some(&program), &no_handler));
    assert!(!executable_without_handler(Some(&program), &denied));
    assert!(!executable_without_handler(
        Some(fixture.path()),
        &no_handler
    ));
    assert!(!executable_without_handler(None, &no_handler));

    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644))?;
    assert!(!executable_without_handler(Some(&program), &no_handler));
    Ok(())
}

#[test]
fn program_command_runs_from_program_directory() {
    let path = Path::new("/tmp/tools/program");
    let command = program_command(path);

    assert_eq!(command.get_program(), path.as_os_str());
    assert_eq!(command.get_current_dir(), Some(Path::new("/tmp/tools")));
}

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

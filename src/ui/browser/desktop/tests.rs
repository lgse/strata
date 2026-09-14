// SPDX-License-Identifier: MIT

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
fn entry_executable_policy_accepts_regular_files_and_file_links() {
    let entry = |kind, mode| FileEntry {
        location: Location::local("/fixture/program"),
        native_name: "program".into(),
        thumbnail_path: None,
        display_name: "program".into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Known(mode),
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    };

    assert!(entry_is_regular_executable(&entry(
        crate::model::EntryKind::File,
        0o755,
    )));
    assert!(entry_is_regular_executable(&entry(
        crate::model::EntryKind::FileSymbolicLink,
        0o755,
    )));
    assert!(!entry_is_regular_executable(&entry(
        crate::model::EntryKind::File,
        0o644,
    )));
    assert!(!entry_is_regular_executable(&entry(
        crate::model::EntryKind::Directory,
        0o755,
    )));

    let mut remote = entry(crate::model::EntryKind::File, 0o755);
    remote.location = Location::uri("smb://server/program");
    assert!(!entry_is_regular_executable(&remote));

    let mut unknown_mode = entry(crate::model::EntryKind::File, 0o755);
    unknown_mode.mode = crate::model::MetadataValue::Unknown;
    assert!(!entry_is_regular_executable(&unknown_mode));
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
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
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

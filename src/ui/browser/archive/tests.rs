// SPDX-License-Identifier: MIT

use super::*;
use crate::model::Location;
use crate::services::{ArchiveFormat, validate_basename};

#[test]
fn archive_names_strip_only_the_selected_dotted_extension() {
    assert_eq!(
        normalized_archive_name("backup.zip", ArchiveFormat::Zip),
        "backup"
    );
    assert_eq!(
        normalized_archive_name("backupzip", ArchiveFormat::Zip),
        "backupzip"
    );
    assert_eq!(
        normalized_archive_name("backup.tar.gz", ArchiveFormat::TarGz),
        "backup"
    );
    assert_eq!(
        normalized_archive_name("backup.rar", ArchiveFormat::Rar),
        "backup"
    );
    assert!(
        validate_basename(&normalized_archive_name(
            "../outside.zip",
            ArchiveFormat::Zip
        ))
        .is_err()
    );
}

#[test]
fn archive_collisions_use_the_final_name() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let destination = Location::local(root.path());
    assert!(!archive_has_collision(&destination, "archive.zip"));
    std::fs::write(root.path().join("archive.zip"), b"existing")?;
    assert!(archive_has_collision(&destination, "archive.zip"));
    Ok(())
}

#[test]
fn quoted_names_never_drive_the_password_retry() {
    for (message, needs_password, wrong_password) in [
        ("The password may be incorrect.", true, true),
        (
            "A password is required to extract this archive.",
            true,
            false,
        ),
        (
            "The password may be incorrect. Extracted entries remain in `photos`.",
            true,
            true,
        ),
        ("Not an archive: `passwords.zip`", false, false),
        (
            "This file is not a valid archive or is damaged. Extracted entries remain in `encrypted-incorrect`.",
            false,
            false,
        ),
        (
            "Archive member `a`b.txt` uses unsupported encryption",
            true,
            false,
        ),
        (
            "Could not link `.password-store/incorrect` to `encrypted`: Operation not permitted",
            false,
            false,
        ),
    ] {
        assert_eq!(
            extract_error_needs_password(message),
            needs_password,
            "{message}"
        );
        assert_eq!(
            extract_error_reports_wrong_password(message),
            wrong_password,
            "{message}"
        );
    }
}

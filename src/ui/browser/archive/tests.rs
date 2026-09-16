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
fn extraction_subfolders_reserve_fresh_names() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let stem = archive_stem("archive.tar.gz");
    let first = create_extraction_subfolder(root.path(), stem)?;
    assert_eq!(first, Location::local(root.path().join("archive")));
    std::fs::write(root.path().join("archive/keep.txt"), b"original")?;
    std::fs::write(root.path().join("archive (1)"), b"existing file")?;
    std::os::unix::fs::symlink("missing", root.path().join("archive (2)"))?;
    for suffix in [3, 4] {
        let destination = create_extraction_subfolder(root.path(), stem)?;
        let expected = root.path().join(format!("archive ({suffix})"));
        assert_eq!(destination, Location::local(&expected));
        assert!(expected.is_dir());
    }
    assert_eq!(
        std::fs::read(root.path().join("archive/keep.txt"))?,
        b"original"
    );
    assert_eq!(
        std::fs::read(root.path().join("archive (1)"))?,
        b"existing file"
    );
    assert_eq!(
        std::fs::read_link(root.path().join("archive (2)"))?,
        Path::new("missing")
    );
    Ok(())
}

#[test]
fn extraction_subfolder_creation_reports_parent_errors() {
    let root = tempfile::tempdir().expect("temporary extraction parent");
    let error = create_extraction_subfolder(&root.path().join("missing"), "archive")
        .expect_err("missing parent must fail");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(!root.path().join("missing").exists());
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

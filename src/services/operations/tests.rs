// SPDX-License-Identifier: GPL-3.0-or-later

use super::{ArchiveFormat, validate_basename};

#[test]
fn basenames_reject_empty_reserved_nested_absolute_and_nul_names() {
    for name in [
        "",
        ".",
        "..",
        "../escaped",
        "nested/child",
        "/tmp/absolute",
        "nul\0name",
    ] {
        assert!(
            validate_basename(name).is_err(),
            "{name:?} should be rejected"
        );
    }
}

#[test]
fn basenames_accept_single_native_and_unicode_components() {
    for name in ["report.txt", "folder name", ".config", "résumé"] {
        assert!(
            validate_basename(name).is_ok(),
            "{name:?} should be accepted"
        );
    }
}

#[test]
fn archive_formats_are_detected_by_extension() {
    assert_eq!(
        ArchiveFormat::from_extension("photos.zip"),
        Some(ArchiveFormat::Zip)
    );
    assert_eq!(
        ArchiveFormat::from_extension("backup.tar.gz"),
        Some(ArchiveFormat::TarGz)
    );
    assert_eq!(
        ArchiveFormat::from_extension("archive.TGZ"),
        Some(ArchiveFormat::TarGz)
    );
    assert_eq!(
        ArchiveFormat::from_extension("data.tar"),
        Some(ArchiveFormat::Tar)
    );
    assert_eq!(ArchiveFormat::from_extension("document.pdf"), None);
    assert_eq!(ArchiveFormat::from_extension("no_extension"), None);
}

#[test]
fn archive_format_extensions_round_trip() {
    for format in [ArchiveFormat::Zip, ArchiveFormat::TarGz, ArchiveFormat::Tar] {
        let name = format!("test.{}", format.extension());
        assert_eq!(ArchiveFormat::from_extension(&name), Some(format));
    }
}

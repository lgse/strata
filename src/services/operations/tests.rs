// SPDX-License-Identifier: MIT

use super::{ArchiveFormat, RenameCause, RenamedItem, TransferReport, validate_basename};
use crate::{
    model::{EntryKind, Location},
    test_support::operations::entry,
};

#[test]
fn basenames_reject_empty_reserved_nested_absolute_and_nul_names() {
    for name in [
        "",
        "   ",
        "\t\n\r",
        "\u{00a0}\u{2003}",
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
    for name in [
        "report.txt",
        "folder name",
        ".config",
        "résumé",
        " padded ",
        "-draft",
        "a\\b",
    ] {
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
    assert_eq!(
        ArchiveFormat::from_extension("files.7z"),
        Some(ArchiveFormat::SevenZ)
    );
    let rar = cfg!(feature = "rar").then_some(ArchiveFormat::Rar);
    assert_eq!(ArchiveFormat::from_extension("archive.rar"), rar);
    assert_eq!(ArchiveFormat::from_extension("ARCHIVE.RAR"), rar);
    assert_eq!(ArchiveFormat::from_extension("document.pdf"), None);
    assert_eq!(ArchiveFormat::from_extension("no_extension"), None);
}

#[test]
fn extractable_entries_require_a_local_regular_file() {
    let local = Location::local("/fixture/photos.zip");
    let remote = Location::uri("sftp://example.com/photos.zip");
    for (location, kind, expected) in [
        (&local, EntryKind::File, Some(ArchiveFormat::Zip)),
        (
            &local,
            EntryKind::FileSymbolicLink,
            Some(ArchiveFormat::Zip),
        ),
        (&local, EntryKind::Directory, None),
        (&local, EntryKind::DirectorySymbolicLink, None),
        (&local, EntryKind::Other, None),
        (&local, EntryKind::SymbolicLink, None),
        (&remote, EntryKind::File, None),
        (
            &Location::local("/fixture/notes.txt"),
            EntryKind::File,
            None,
        ),
    ] {
        let mut candidate = entry(location.clone());
        candidate.kind = kind;
        assert_eq!(
            ArchiveFormat::for_entry(&candidate),
            expected,
            "{location:?} {kind:?}"
        );
    }
}

#[test]
fn an_item_saved_under_several_names_in_turn_is_reported_once_from_its_first_name() {
    let item = |requested: &str, saved_as: &str, cause| RenamedItem {
        requested: Location::local(format!("/copy/{requested}")),
        saved_as: Location::local(format!("/copy/{saved_as}")),
        cause,
    };
    // A fixed name, which the destination took for another's, numbered as an
    // alias whose number a merge then found taken, reported in any order
    // beside entries of other items.
    let links = [
        item("a:b", "a_b", RenameCause::Invalid),
        item("a_b", "a_b (1)", RenameCause::Alias),
        item("a_b (1)", "a_b (2)", RenameCause::Taken),
    ];
    let others = [
        item("makefile", "makefile (1)", RenameCause::Alias),
        item("c?d", "c_d", RenameCause::Invalid),
    ];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut report = TransferReport {
            renamed: vec![others[0].clone()],
            ..TransferReport::default()
        };
        report
            .renamed
            .extend(order.map(|index| links[index].clone()));
        report.renamed.push(others[1].clone());

        report.follow_renames();

        assert_eq!(
            report.renamed,
            [
                others[0].clone(),
                item("a:b", "a_b (2)", RenameCause::Invalid),
                others[1].clone(),
            ],
            "{order:?}"
        );
    }
}

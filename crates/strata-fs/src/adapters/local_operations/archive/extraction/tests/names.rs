// SPDX-License-Identifier: MIT

use std::{error::Error, ffi::OsStr, fs, os::unix::ffi::OsStrExt, path::Path};

use rustix::io::Errno;

use super::super::{ArchiveError, ArchiveOutcome};
use super::harness::{ARCHIVE, Outcome, assert_same_inode};
use super::members::{
    Entry, Extracted,
    Member::{File, Folder, Hard, Link},
    entries, extract_members, renamed,
};
use crate::{
    adapters::local_operations::tests::fs_quirks::{
        FsQuirks, Op, Preset, Quirks, skips_malformed_name_on_fuse,
    },
    services::RenameCause::{self, Alias, Invalid, Taken, TooLong},
};

type Rename<'a> = (&'a [u8], &'a str, RenameCause);

const STORED_NAMES: &[Entry<'static>] = &[
    (b"10:00", Folder),
    (b"10:00/c?d.txt", File),
    (b"10_00/x.txt", File),
    (b"CON", File),
    (b"aux.c", File),
    (b"aux.c", File),
    (b"Readme.txt", File),
    (b"README.TXT", File),
    (b"dot.", File),
    (b"x\xffy", File),
    (b"x?y", File),
    (b"\xc0\xaf", File),
];

const WINDOWS_LISTING: &[&str] = &[
    "CON_",
    "README (2).TXT",
    "Readme.txt",
    "__",
    "10_00 (2)/",
    "10_00 (2)/x.txt",
    "10_00/",
    "10_00/c_d.txt",
    "aux (2).c",
    "aux.c",
    "dot",
    "x_y",
    "x_y (2)",
];

const WINDOWS_RENAMES: &[Rename<'static>] = &[
    (b"10:00".as_slice(), "10_00", Invalid),
    (b"10_00/c?d.txt", "10_00/c_d.txt", Invalid),
    (b"10_00", "10_00 (2)", Taken),
    (b"CON", "CON_", Invalid),
    (b"README.TXT", "README (2).TXT", Alias),
    (b"dot.", "dot", Invalid),
    (b"x\xffy", "x_y", Invalid),
    (b"x?y", "x_y (2)", Invalid),
    (b"\xc0\xaf", "__", Invalid),
];

const POSIX_LISTING: &[&str] = &[
    "CON",
    "README.TXT",
    "Readme.txt",
    "10:00/",
    "10:00/c?d.txt",
    "10_00/",
    "10_00/x.txt",
    "aux (2).c",
    "aux.c",
    "dot.",
    "x?y",
    "x\u{fffd}y",
    "\u{fffd}\u{fffd}",
];

#[test]
fn members_take_the_names_the_destination_stores_and_distinct_folders_stay_apart()
-> Result<(), Box<dyn Error>> {
    for (preset, listed, reported) in [
        (Quirks::ext4 as Preset, POSIX_LISTING, &[][..]),
        (Quirks::vfat, WINDOWS_LISTING, WINDOWS_RENAMES),
        (Quirks::exfat_fuse, WINDOWS_LISTING, WINDOWS_RENAMES),
    ] {
        let utf8_only = !reported.is_empty();
        let root = tempfile::tempdir()?;
        let skipped = STORED_NAMES
            .iter()
            .any(|(name, _)| skips_malformed_name_on_fuse(root.path(), OsStr::from_bytes(name)));
        if !utf8_only && skipped {
            continue;
        }
        let quirks = FsQuirks::install(root.path(), preset());

        let extracted = extract_members(root.path(), ARCHIVE, STORED_NAMES, Ok(()));

        let mut expected = listed.to_vec();
        expected.sort_unstable();
        assert_eq!(extracted.completed(), "archive");
        assert_eq!(entries(&root.path().join("archive"))?, expected);
        assert_eq!(
            extracted.report.renamed,
            renamed(&root.path().join("archive"), reported)
        );
        assert_eq!(
            quirks
                .calls()
                .iter()
                .all(|call| call.names.iter().all(|name| name.to_str().is_some())),
            utf8_only,
            "malformed names were sent only where they are stored"
        );
    }
    Ok(())
}

#[test]
fn a_colon_after_the_first_component_extracts_as_a_name() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let members: &[Entry<'_>] = &[
        (b"before.txt", File),
        (b"names", Folder),
        (b"names/c:notes", File),
        (b"names/linked", Hard("names/c:notes")),
        (b"after.txt", File),
    ];

    extract_members(root.path(), ARCHIVE, members, Ok(())).outcome?;

    assert_eq!(
        entries(root.path())?,
        [
            "archive/",
            "archive/after.txt",
            "archive/before.txt",
            "archive/names/",
            "archive/names/c:notes",
            "archive/names/linked"
        ]
    );
    assert_eq!(fs::read(root.path().join("archive/names/linked"))?, b"data");
    Ok(())
}

fn refuses_colons(name: &OsStr) -> Option<Errno> {
    name.as_bytes().contains(&b':').then_some(Errno::INVAL)
}

fn refuses_colons_as_bad_messages(name: &OsStr) -> Option<Errno> {
    name.as_bytes().contains(&b':').then_some(Errno::BADMSG)
}

fn refuses_trailing_dots(name: &OsStr) -> Option<Errno> {
    name.as_bytes().ends_with(b".").then_some(Errno::BADMSG)
}

const FIRST_ARCHIVE: &[Entry<'static>] = &[
    (b"10:00", Folder),
    (b"10:00/note:1.txt", File),
    (b"10:00/py:3", Link(b"note:1.txt")),
    (b"10:00/hard:1", Hard("10:00/note:1.txt")),
    (b"day_1.txt", File),
    (b"day:1.txt", File),
];

const LATER_ARCHIVE: &[Entry<'static>] = &[
    (b"dir:1", Folder),
    (b"dir:1/t", File),
    (b"file:1", File),
    (b"dot.", File),
];

fn unknown_daemon() -> Quirks {
    Quirks {
        mount_type: Some(("fuse.strictfs", "rw,user_id=0,group_id=0")),
        invalid_name: Some(refuses_colons_as_bad_messages),
        ..Quirks::default()
    }
}

// Known rules never learn. This one also refuses a hard link's name,
// and the lookup of a name, as sshfs does.
fn strict_ext4() -> Quirks {
    Quirks {
        invalid_name: Some(refuses_colons),
        fail: vec![
            (Op::Link, "hard:1", 0, Errno::INVAL),
            (Op::Stat, "day:1.txt", 0, Errno::BADMSG),
        ],
        ..Quirks::ext4()
    }
}

type Refused<'a> = &'a [(Op, &'a str)];

/// The calls that named a member the mount refuses.
fn refused_calls(quirks: &FsQuirks) -> Vec<(Op, String)> {
    quirks
        .calls()
        .into_iter()
        .filter_map(|call| {
            let name = call
                .names
                .iter()
                .find(|name| name.as_bytes().contains(&b':'))?;
            Some((call.op, name.to_string_lossy().into_owned()))
        })
        .collect()
}

fn owned(calls: Refused<'_>) -> Vec<(Op, String)> {
    calls
        .iter()
        .map(|(op, name)| (*op, (*name).to_owned()))
        .collect()
}

fn first_archive_takes_stricter_names(root: &Path) -> Result<(), Box<dyn Error>> {
    let extracted = extract_members(root, ARCHIVE, FIRST_ARCHIVE, Ok(()));

    let archive = root.join("archive");
    assert_eq!(extracted.completed(), "archive");
    assert_eq!(
        entries(&archive)?,
        [
            "10_00/",
            "10_00/hard_1",
            "10_00/note_1.txt",
            "10_00/py_3",
            "day_1 (2).txt",
            "day_1.txt"
        ]
    );
    assert_eq!(
        extracted.report.renamed,
        renamed(
            &archive,
            &[
                (b"10:00".as_slice(), "10_00", Invalid),
                (b"10_00/note:1.txt", "10_00/note_1.txt", Invalid),
                (b"10_00/py:3", "10_00/py_3", Invalid),
                (b"10_00/hard:1", "10_00/hard_1", Invalid),
                (b"day:1.txt", "day_1 (2).txt", Invalid),
            ]
        )
    );
    assert_links_kept_their_targets(&archive)
}

fn assert_links_kept_their_targets(archive: &Path) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        fs::read_link(archive.join("10_00/py_3"))?,
        Path::new("note:1.txt")
    );
    assert_same_inode(
        &archive.join("10_00/hard_1"),
        &archive.join("10_00/note_1.txt"),
    )
}

fn later_archive_takes_stricter_names(root: &Path) -> Result<(), Box<dyn Error>> {
    let extracted = extract_members(root, "later.zip", LATER_ARCHIVE, Ok(()));

    let archive = root.join("later");
    assert_eq!(extracted.completed(), "later");
    assert_eq!(entries(&archive)?, ["dir_1/", "dir_1/t", "dot.", "file_1"]);
    assert_eq!(
        extracted.report.renamed,
        renamed(
            &archive,
            &[
                (b"dir:1".as_slice(), "dir_1", Invalid),
                (b"file:1", "file_1", Invalid)
            ]
        )
    );
    Ok(())
}

#[test]
fn a_refused_member_name_takes_the_first_stricter_rules_it_passes_and_teaches_only_a_mount_that_learns()
-> Result<(), Box<dyn Error>> {
    let rows: [(Preset, Refused<'_>, Refused<'_>); 2] = [
        (unknown_daemon, &[(Op::Mkdir, "10:00")], &[]),
        (
            strict_ext4,
            &[
                (Op::Mkdir, "10:00"),
                (Op::Stat, "note:1.txt"),
                (Op::Create, "note:1.txt"),
                (Op::Stat, "py:3"),
                (Op::Symlink, "py:3"),
                (Op::Stat, "hard:1"),
                (Op::Link, "hard:1"),
                (Op::Stat, "day:1.txt"),
            ],
            &[
                (Op::Mkdir, "dir:1"),
                (Op::Stat, "file:1"),
                (Op::Create, "file:1"),
            ],
        ),
    ];
    for (quirks, refused_first, refused_later) in rows {
        let root = tempfile::tempdir()?;
        let quirks = FsQuirks::install(root.path(), quirks());

        first_archive_takes_stricter_names(root.path())?;
        assert_eq!(refused_calls(&quirks), owned(refused_first));
        quirks.clear_calls();

        later_archive_takes_stricter_names(root.path())?;
        assert_eq!(refused_calls(&quirks), owned(refused_later));
    }
    Ok(())
}

#[test]
fn a_mount_that_learns_keeps_only_the_rules_its_refused_member_names_needed()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let quirks = FsQuirks::install(
        root.path(),
        Quirks {
            mount_type: Some(("fuse.strictfs", "rw,user_id=0,group_id=0")),
            invalid_name: Some(refuses_trailing_dots),
            ..Quirks::default()
        },
    );
    let refused_creates = || {
        quirks
            .calls()
            .into_iter()
            .filter(|call| call.op == Op::Create)
            .flat_map(|call| call.names)
            .filter(|name| name.as_bytes().ends_with(b"."))
            .map(|name| name.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };

    extract_members(
        root.path(),
        ARCHIVE,
        &[(b"note:1.", File), (b"x", File)],
        Ok(()),
    )
    .outcome?;

    assert_eq!(entries(&root.path().join("archive"))?, ["note:1", "x"]);
    assert_eq!(refused_creates(), ["note:1.", "note_1."]);
    quirks.clear_calls();

    let extracted = extract_members(
        root.path(),
        "later.zip",
        &[(b"text:1", File), (b"dot.", File)],
        Ok(()),
    );

    extracted.outcome?;
    let later = root.path().join("later");
    assert_eq!(entries(&later)?, ["dot", "text:1"]);
    assert_eq!(
        extracted.report.renamed,
        renamed(&later, &[(b"dot.", "dot", Invalid)])
    );
    assert!(refused_creates().is_empty());
    Ok(())
}

type Fails<'a> = &'a [(Op, &'a str, Errno)];

/// Only `py:3` has a stricter name; the destination refuses it too, and
/// the first refusal is the one reported.
fn refused_member_rows() -> [(Entry<'static>, Fails<'static>, String); 10] {
    let invalid = "The name isn’t valid at this location";
    let io_failed = crate::services::io_error_message(&Errno::IO.into());
    [
        (
            (b"bin/python3", Link(b"python")),
            &[(Op::Symlink, "python3", Errno::NOSPC)],
            "Could not create symbolic link “bin/python3”: There is not enough space on the device"
                .to_owned(),
        ),
        (
            (b"bin/python3", Link(b"python")),
            &[(Op::Symlink, "python3", Errno::BADMSG)],
            format!("Could not create symbolic link “bin/python3”: {invalid}"),
        ),
        (
            (b"bin/py:3", Link(b"python")),
            &[
                (Op::Symlink, "py:3", Errno::BADMSG),
                (Op::Symlink, "py_3", Errno::NAMETOOLONG),
            ],
            format!("Could not create symbolic link “bin/py:3”: {invalid}"),
        ),
        (
            (b"bin/lib", Folder),
            &[(Op::Mkdir, "lib", Errno::INVAL)],
            format!("Could not create “bin/lib”: {invalid}"),
        ),
        (
            (b"lib/x", File),
            &[(Op::Mkdir, "lib", Errno::BADMSG)],
            format!("Could not create “lib”: {invalid}"),
        ),
        (
            (b"bin/plain", File),
            &[(Op::Create, "plain", Errno::BADMSG)],
            format!("Could not create “bin/plain”: {invalid}"),
        ),
        (
            (b"bin/plain", File),
            &[(Op::Stat, "plain", Errno::ILSEQ)],
            format!("Could not create “bin/plain”: {invalid}"),
        ),
        (
            (b"bin/plain", File),
            &[(Op::Create, "plain", Errno::NAMETOOLONG)],
            "Could not create “bin/plain”: The name is too long".to_owned(),
        ),
        (
            (b"bin/hard", Hard("bin/python")),
            &[(Op::Link, "hard", Errno::INVAL)],
            format!("Could not link “bin/hard” to “bin/python”: {invalid}"),
        ),
        // The copy that stands in for a hard link the mount cannot hold
        // reports why its target could not be read, not the refused link.
        (
            (b"bin/hard", Hard("bin/python")),
            &[
                (Op::Link, "hard", Errno::PERM),
                (Op::OpenSource, "python", Errno::IO),
            ],
            format!("Could not link “bin/hard” to “bin/python”: {io_failed}"),
        ),
    ]
}

#[test]
fn members_the_destination_will_not_make_fail_the_extraction_naming_them_and_why()
-> Result<(), Box<dyn Error>> {
    for (member, fails, failure) in refused_member_rows() {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(
            root.path(),
            Quirks {
                fail: fails
                    .iter()
                    .map(|(op, name, errno)| (*op, *name, 0, *errno))
                    .collect(),
                ..Quirks::ext4()
            },
        );

        let extracted = extract_members(
            root.path(),
            ARCHIVE,
            &[(b"bin/python", File), member],
            Ok(()),
        );

        let Err(ArchiveError::Failed(message)) = &extracted.outcome else {
            panic!("{fails:?} did not fail: {:?}", extracted.outcome);
        };
        assert_eq!(
            *message,
            format!("{failure}. Extracted entries remain in “archive”.")
        );
        assert!(extracted.report.skipped.is_empty());
    }
    Ok(())
}

struct ReportRow {
    preset: Preset,
    archive: String,
    existing: Option<&'static str>,
    members: &'static [Entry<'static>],
    ending: Result<(), ArchiveError>,
    finished: fn(&Outcome) -> bool,
    renamed_as: Vec<(Vec<u8>, String, RenameCause)>,
    skipped: &'static [&'static str],
}

const NESTED: &[Entry<'static>] = &[(b"10:00", Folder), (b"10:00/c?d", File)];
const TWO_FILES: &[Entry<'static>] = &[(b"a.txt", File), (b"b?.txt", File)];

fn rename(requested: &[u8], saved_as: &str, cause: RenameCause) -> (Vec<u8>, String, RenameCause) {
    (requested.to_vec(), saved_as.to_owned(), cause)
}

fn nested_fixed() -> Vec<(Vec<u8>, String, RenameCause)> {
    vec![
        rename(b"archive/10:00", "archive/10_00", Invalid),
        rename(b"archive/10_00/c?d", "archive/10_00/c_d", Invalid),
    ]
}

fn completed_as(outcome: &Outcome, published: &str) -> bool {
    matches!(outcome, Ok(ArchiveOutcome::Completed(Some(name))) if name == published)
}

/// Renames the report applies to the archive folder's own name.
fn renamed_folder_rows() -> Vec<ReportRow> {
    let long = format!("{}.zip", "a".repeat(256));
    vec![
        ReportRow {
            preset: Quirks::vfat,
            archive: ARCHIVE.to_owned(),
            existing: Some("10_00"),
            members: NESTED,
            ending: Ok(()),
            finished: |outcome| completed_as(outcome, "10_00 (2)"),
            renamed_as: vec![
                rename(b"10:00", "10_00 (2)", Invalid),
                rename(b"10_00 (2)/c?d", "10_00 (2)/c_d", Invalid),
            ],
            skipped: &[],
        },
        ReportRow {
            preset: Quirks::vfat,
            archive: "10:00.zip".to_owned(),
            existing: None,
            members: TWO_FILES,
            ending: Ok(()),
            finished: |outcome| completed_as(outcome, "10_00"),
            renamed_as: vec![
                rename(b"10:00".as_slice(), "10_00", Invalid),
                rename(b"10_00/b?.txt", "10_00/b_.txt", Invalid),
            ],
            skipped: &[],
        },
        ReportRow {
            preset: Quirks::vfat,
            archive: "10:00.zip".to_owned(),
            existing: Some("10_00"),
            members: TWO_FILES,
            ending: Ok(()),
            finished: |outcome| completed_as(outcome, "10_00 (1)"),
            renamed_as: vec![
                rename(b"10:00", "10_00 (1)", Invalid),
                rename(b"10_00 (1)/b?.txt", "10_00 (1)/b_.txt", Invalid),
            ],
            skipped: &[],
        },
        ReportRow {
            preset: Quirks::ext4,
            archive: long.clone(),
            existing: None,
            members: &[(b"a.txt", File), (b"b.txt", File)],
            ending: Ok(()),
            finished: |outcome| matches!(outcome, Ok(ArchiveOutcome::Completed(Some(name))) if name.len() == 255),
            renamed_as: vec![rename(&long.as_bytes()[..256], &"a".repeat(255), TooLong)],
            skipped: &[],
        },
    ]
}

/// What the report keeps when the extraction does not complete.
fn ended_early_rows() -> Vec<ReportRow> {
    let row = |ending, finished, renamed_as| ReportRow {
        preset: Quirks::vfat,
        archive: ARCHIVE.to_owned(),
        existing: None,
        members: NESTED,
        ending,
        finished,
        renamed_as,
        skipped: &[],
    };
    vec![
        row(
            Err(ArchiveError::Failed("Damaged.".to_owned())),
            |outcome| matches!(outcome, Err(ArchiveError::Failed(message)) if message.ends_with("remain in “archive”.")),
            nested_fixed(),
        ),
        row(
            Err(ArchiveError::Cancelled),
            |outcome| matches!(outcome, Ok(ArchiveOutcome::Cancelled { .. })),
            nested_fixed(),
        ),
        row(
            Err(ArchiveError::PasswordRequired(String::new())),
            |outcome| matches!(outcome, Err(ArchiveError::PasswordRequired(_))),
            Vec::new(),
        ),
    ]
}

/// Links a FAT destination leaves out are reported where they would have been.
fn skipped_link_rows() -> Vec<ReportRow> {
    vec![
        ReportRow {
            preset: Quirks::vfat,
            archive: ARCHIVE.to_owned(),
            existing: None,
            members: &[(b"lnk", Link(b"target"))],
            ending: Ok(()),
            finished: |outcome| matches!(outcome, Ok(ArchiveOutcome::Completed(None))),
            renamed_as: Vec::new(),
            skipped: &["lnk"],
        },
        ReportRow {
            preset: Quirks::vfat,
            archive: ARCHIVE.to_owned(),
            existing: None,
            members: &[(b"bin/lnk", Link(b"target"))],
            ending: Ok(()),
            finished: |outcome| completed_as(outcome, "bin"),
            renamed_as: Vec::new(),
            skipped: &["bin/lnk"],
        },
    ]
}

#[test]
fn the_report_names_entries_where_the_extraction_left_them() -> Result<(), Box<dyn Error>> {
    let rows = renamed_folder_rows()
        .into_iter()
        .chain(ended_early_rows())
        .chain(skipped_link_rows());
    for row in rows {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), (row.preset)());
        if let Some(existing) = row.existing {
            fs::create_dir(root.path().join(existing))?;
        }

        let extracted = extract_members(root.path(), &row.archive, row.members, row.ending);

        let Extracted { outcome, report } = &extracted;
        assert!((row.finished)(outcome), "{}: {outcome:?}", row.archive);
        assert_eq!(
            report.renamed,
            renamed(root.path(), &row.renamed_as),
            "{outcome:?}"
        );
        extracted.assert_skipped_links(root.path(), row.skipped);
        if let Ok(ArchiveOutcome::Cancelled { not_attempted, .. }) = outcome {
            let pending = format!("archive/10_00/later_x{}/next.txt", "a".repeat(248));
            assert_eq!(
                *not_attempted,
                [crate::model::Location::local(root.path().join(pending))]
            );
        }
    }
    Ok(())
}

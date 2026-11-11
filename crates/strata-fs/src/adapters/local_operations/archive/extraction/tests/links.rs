// SPDX-License-Identifier: MIT

use std::{error::Error, fs, os::unix::fs::MetadataExt, path::Path};

use rustix::io::Errno;

use super::super::{MemberContent, MemberMetadata};
use super::harness::{ARCHIVE, Harness, assert_same_inode};
use super::members::{
    Entry,
    Member::{File, Folder, Hard, Link},
    entries, extract_members,
};
use crate::adapters::local_operations::tests::fs_quirks::{FsQuirks, Op, Quirks, Support};

#[test]
fn links_consume_no_space_and_hard_links_follow_renamed_targets() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = harness.open_with_available_bytes(Some(12))?;
    session.record_hard_link_targets();
    for contents in [b"first!", b"second"] {
        session.extract_file("data.txt", contents)?;
    }
    session.extract_member(
        "lnk",
        MemberContent::Symlink(b"data.txt"),
        MemberMetadata::NONE,
    )?;
    session.extract_member(
        "hard",
        MemberContent::HardLink(Path::new("data.txt")),
        MemberMetadata::NONE,
    )?;

    let published = session.finish_completed(Ok(()), Vec::new)?;

    assert_eq!(published.as_deref(), Some("archive"));
    assert_eq!(session.progress(), 4);
    let output = session.path().join("archive");
    assert_eq!(fs::read_link(output.join("lnk"))?, Path::new("data.txt"));
    assert_eq!(fs::read(output.join("lnk"))?, b"first!");
    assert_same_inode(&output.join("hard"), &output.join("data (2).txt"))
}

#[test]
fn refused_link_members_leave_no_staging_behind() -> Result<(), Box<dyn Error>> {
    let too_long = [b'a'; 4096];
    for (index, expected) in [
        "is a hard link to “later.txt”, which was not extracted",
        "has an invalid symbolic link target",
        "has an invalid symbolic link target",
        "has an invalid symbolic link target",
    ]
    .into_iter()
    .enumerate()
    {
        let content = match index {
            0 => MemberContent::HardLink(Path::new("later.txt")),
            1 => MemberContent::Symlink(b""),
            2 => MemberContent::Symlink(b"a\0b"),
            _ => MemberContent::Symlink(&too_long),
        };
        let mut harness = Harness::new()?;
        let mut session = harness.open()?;
        session.record_hard_link_targets();

        let result = session.extract_member("link", content, MemberMetadata::NONE);

        assert!(session.path().read_dir()?.next().is_none(), "case {index}");
        let message = session.finish_failed(result);
        assert_eq!(message, format!("Archive member “link” {expected}"));
        session.assert_extracted(0, [])?;
    }
    Ok(())
}

const VENV: &[Entry<'static>] = &[
    (b"bin", Folder),
    (b"bin/python", File),
    (b"bin/python3", Link(b"python")),
    (b"bin/py", Hard("bin/python3")),
    (b"bin/p2", Hard("bin/py")),
    (b"bin/py:3", Link(b"python")),
];

const LEARNED: &[Entry<'static>] = &[
    (b"bin", Folder),
    (b"bin/python", File),
    (b"bin/python3", Link(b"python")),
    (b"bin/pip", Link(b"python")),
];

const PER_LINK: &[Entry<'static>] = &[
    (b"bin", Folder),
    (b"bin/python", File),
    (b"bin/bad", Link(b"far")),
    (b"bin/python3", Link(b"python")),
    (b"bin/10:00", File),
    (b"bin/now", Hard("bin/10:00")),
];

const MALFORMED: &[Entry<'static>] = &[
    (b"bin", Folder),
    (b"bin/python", File),
    (b"bin/bad", Link(b"x\xffy")),
    (b"bin/python3", Link(b"python")),
];

// A stricter name would be `dot`, which a link the mount cannot hold never tries.
const UNHELD: &[Entry<'static>] = &[
    (b"bin", Folder),
    (b"bin/python", File),
    (b"bin/dot.", Link(b"python")),
];

const PER_LINK_LISTED: &[&str] = &["bin/", "bin/10_00", "bin/now", "bin/python", "bin/python3"];

struct LinkRow {
    quirks: Quirks,
    members: &'static [Entry<'static>],
    listed: &'static [&'static str],
    skipped: &'static [&'static str],
    symlink_calls: usize,
}

fn refusing_one_target(errno: Errno) -> Quirks {
    Quirks {
        fail: vec![(Op::Symlink, "bad", 0, errno)],
        ..Quirks::ntfs3()
    }
}

fn link_rows() -> [LinkRow; 7] {
    [
        LinkRow {
            quirks: Quirks::ext4(),
            members: VENV,
            listed: &[
                "bin/",
                "bin/p2",
                "bin/py",
                "bin/py:3",
                "bin/python",
                "bin/python3",
            ],
            skipped: &[],
            symlink_calls: 2,
        },
        LinkRow {
            quirks: Quirks::vfat(),
            members: VENV,
            listed: &["bin/", "bin/python"],
            skipped: &["python3", "py", "p2", "py:3"],
            symlink_calls: 0,
        },
        LinkRow {
            quirks: Quirks::cifs(),
            members: LEARNED,
            listed: &["bin/", "bin/python"],
            skipped: &["python3", "pip"],
            symlink_calls: 1,
        },
        LinkRow {
            quirks: refusing_one_target(Errno::INVAL),
            members: PER_LINK,
            listed: PER_LINK_LISTED,
            skipped: &["bad"],
            symlink_calls: 2,
        },
        LinkRow {
            quirks: refusing_one_target(Errno::ILSEQ),
            members: PER_LINK,
            listed: PER_LINK_LISTED,
            skipped: &["bad"],
            symlink_calls: 2,
        },
        LinkRow {
            quirks: Quirks::cifs_posix(),
            members: MALFORMED,
            listed: &["bin/", "bin/python", "bin/python3"],
            skipped: &["bad"],
            symlink_calls: 1,
        },
        LinkRow {
            quirks: Quirks {
                fail: vec![(Op::Symlink, "dot.", 0, Errno::PERM)],
                ..Quirks::ext4()
            },
            members: UNHELD,
            listed: &["bin/", "bin/python"],
            skipped: &["dot."],
            symlink_calls: 1,
        },
    ]
}

#[test]
fn links_the_destination_cannot_hold_are_left_out_with_hard_links_to_them()
-> Result<(), Box<dyn Error>> {
    for row in link_rows() {
        let root = tempfile::tempdir()?;
        let quirks = FsQuirks::install(root.path(), row.quirks);

        let extracted = extract_members(root.path(), ARCHIVE, row.members, Ok(()));

        assert_eq!(extracted.completed(), "bin");
        assert_eq!(entries(root.path())?, row.listed);
        extracted.assert_skipped_links(&root.path().join("bin"), row.skipped);
        assert_eq!(
            quirks.count(Op::Symlink),
            row.symlink_calls,
            "{:?}",
            row.listed
        );
        if row.listed.contains(&"bin/now") {
            assert_same_inode(&root.path().join("bin/now"), &root.path().join("bin/10_00"))?;
        }
    }
    Ok(())
}

#[test]
fn a_hard_link_the_destination_cannot_hold_is_copied_and_the_rest_still_extracts()
-> Result<(), Box<dyn Error>> {
    let members: &[Entry<'_>] = &[
        (b"d", Folder),
        (b"d/a.txt", File),
        (b"d/b.txt", Hard("d/a.txt")),
        (b"d/c.txt", Hard("d/b.txt")),
        (b"d/e.txt", File),
    ];
    let refusing = |errno| Quirks {
        links: Support::Rejected(errno),
        ..Quirks::ext4()
    };
    for (quirks, shares_inode) in [
        (Quirks::ext4(), true),
        (Quirks::vfat(), false),
        (Quirks::exfat_fuse(), false),
        (refusing(Errno::OPNOTSUPP), false),
    ] {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), quirks);

        let extracted = extract_members(root.path(), ARCHIVE, members, Ok(()));

        assert_eq!(extracted.completed(), "d");
        assert_eq!(
            entries(root.path())?,
            ["d/", "d/a.txt", "d/b.txt", "d/c.txt", "d/e.txt"]
        );
        for name in ["a.txt", "b.txt", "c.txt", "e.txt"] {
            assert_eq!(fs::read(root.path().join("d").join(name))?, b"data");
        }
        assert!(
            extracted.report.skipped.is_empty(),
            "{:?}",
            extracted.report.skipped
        );
        let inode = |name| fs::metadata(root.path().join("d").join(name)).map(|meta| meta.ino());
        assert_eq!(inode("a.txt")? == inode("b.txt")?, shares_inode);
        assert_eq!(inode("a.txt")? == inode("c.txt")?, shares_inode);
    }
    Ok(())
}

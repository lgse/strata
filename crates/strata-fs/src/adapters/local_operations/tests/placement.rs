// SPDX-License-Identifier: MIT

use std::os::{fd::AsFd, unix::fs::MetadataExt};

use rustix::io::Errno;

use super::super::{
    LocalFileIdentity,
    fs_ops::{Entry, fs},
    mount_profile::known_mount,
    placement::{
        Feasibility, PlaceError, Placed, Probed, RenameFailure, Renamed, can_set_aside, links_hold,
        place, probe, rename_in_place, set_aside,
    },
};
use super::fs_quirks::{Op, OverEmptyDirectory, Preset, Racer, Support, hold};
use super::*;
use crate::{
    adapters::volume::{MountCapabilities, mount_capabilities},
    services::{Holder, InodeNumbers},
};

mod case_renames;
mod ladder;
mod placeholders;
mod probe;
mod reserved_names;
mod smb;

fn without_flags(error: Errno) -> Quirks {
    Quirks {
        rename_flags: Support::Rejected(error),
        ..Quirks::default()
    }
}

fn identity(path: &Path) -> LocalFileIdentity {
    LocalFileIdentity::from_stat(&rustix::fs::lstat(path).expect("entry exists"))
}

fn place_in(
    dir: &Path,
    from: &str,
    to: &str,
    expected: Option<LocalFileIdentity>,
) -> Result<Placed, PlaceError> {
    let parent = open_local_parent_directory(dir).expect("open parent");
    place(
        fs(),
        Entry::new(&parent, OsStr::new(from)),
        Entry::new(&parent, OsStr::new(to)),
        expected,
    )
}

fn rename_in(dir: &Path, from: &str, to: &str) -> Result<Renamed, RenameFailure> {
    let parent = open_local_parent_directory(dir).expect("open parent");
    rename_in_place(
        fs(),
        Entry::new(&parent, OsStr::new(from)),
        Entry::new(&parent, OsStr::new(to)),
    )
}

/// Only `name` is left in `root`, and it is the item `original` was.
fn assert_only(root: &Path, name: &str, original: LocalFileIdentity, row: &str) -> io::Result<()> {
    assert_eq!(
        (names(root)?, identity(&root.join(name))),
        (vec![name.to_owned()], original),
        "{row}"
    );
    Ok(())
}

fn case_insensitive(quirks: Quirks) -> Quirks {
    Quirks {
        case_insensitive: true,
        ..quirks
    }
}

fn make(path: &Path, folder: bool) -> io::Result<()> {
    if folder {
        fs::create_dir(path)?;
        fs::write(path.join("inside.txt"), b"inside")
    } else {
        fs::write(path, b"data")
    }
}

/// No `RENAME_NOREPLACE`, and a link to `target` refused for that item alone,
/// as `protected_hardlinks` does for another user's file.
fn links_refused_for_the_item() -> Quirks {
    Quirks {
        fail: vec![(Op::Link, "target", 0, Errno::PERM)],
        ..Quirks::without_rename_flags()
    }
}

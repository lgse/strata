// SPDX-License-Identifier: MIT

//! Puts an entry at a new name without ever replacing whatever is there:
//! `renameat2(NOREPLACE)`, then a hard link for files or a fresh empty
//! placeholder for folders, then a recreated link for symlinks, and last a
//! plain rename over a reserved name where the mount offers none of these.
//! Errors that may mean "unsupported" are only learned from definitive errnos
//! or from a probe on Strata's own names, and nothing is unlinked without proof.

use std::{
    cell::Cell,
    ffi::{OsStr, OsString},
    ops::ControlFlow,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::Path,
    sync::Arc,
};

use rustix::{
    fs::{AtFlags, FileType, Mode, OFlags, RenameFlags, ResolveFlags},
    io::Errno,
};

use super::{
    fs_ops::{Entry, EntryStat, FsOps, OffMain},
    leftovers::{self, Deadline, Refused},
    mount_profile,
    names::fix::units,
    native_fs::{self, LocalFileIdentity, Recorded},
};
use crate::{
    adapters::volume::{
        LONGEST_NAME_NTFS_3G_RENAMES_OVER, MountCapabilities, MountKey, learn_mount_capability,
        mount_capabilities,
    },
    model::hidden_names::HiddenName,
    services::{Attributes, InodeNumbers},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Placed {
    Renamed,
    Linked,
    ReservedDirectory,
    RecreatedSymlink,
    OverReservedFile,
    /// A plain rename of a folder to a name found free.
    AfterCheck,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PlaceError {
    Exists,
    CrossDevice,
    Unsupported,
    InvalidName,
    IntoItself,
    Changed,
    /// The outcome could not be proven, so both names were kept.
    Unconfirmed,
    Io(Errno),
}

impl PlaceError {
    /// Whether the destination refused the name itself. `EINVAL` from a
    /// rename may mean its flags, so only `EINVAL` from a flagless call or
    /// mapped by the probe counts.
    pub(super) fn refuses_name(self) -> bool {
        matches!(
            self,
            Self::InvalidName | Self::Io(Errno::NAMETOOLONG | Errno::ILSEQ | Errno::BADMSG)
        )
    }
}

/// The kernel's own "not supported", which NFS lets reach userspace.
pub(super) const ENOTSUPP: Errno = Errno::from_raw_os_error(524);

const NAME_ATTEMPTS: usize = 8;
/// Bounds "name (n)" numbering, so a fault reported as `Exists` cannot loop forever.
pub(super) const NUMBERED_NAME_ATTEMPTS: u64 = 10_000;

fn is_unsupported(error: Errno) -> bool {
    matches!(error, Errno::NOSYS | Errno::OPNOTSUPP | ENOTSUPP)
}

fn may_have_lost_reply(error: Errno) -> bool {
    matches!(
        error,
        Errno::NOENT | Errno::IO | Errno::INTR | Errno::TIMEDOUT | Errno::STALE
    )
}

fn at<'a>(dir: BorrowedFd<'a>, name: &'a OsStr) -> Entry<'a> {
    Entry { dir, name }
}

mod aside;
mod exchange;
mod mount;
mod rename;
mod rungs;

pub(super) use aside::*;
pub(super) use exchange::*;
use mount::Mount;
pub(super) use mount::{Feasibility, can_place, can_set_aside, links_hold};
#[cfg(test)]
pub(super) use mount::{Probed, probe};
pub(super) use rename::*;

pub(super) fn place(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    expected: Option<LocalFileIdentity>,
) -> Result<Placed, PlaceError> {
    if let Some(expected) = expected
        && fs
            .stat(from, false)
            .is_ok_and(|current| current.identity != expected)
    {
        return Err(PlaceError::Changed);
    }
    verified(fs, to, expected, ladder(fs, from, to, expected))
}

/// Places `from`, inside a private stage, at `to`. Nothing else writes the
/// stage, so an entry another program puts at `to` once a rung placed it
/// there is theirs, and stays.
pub(super) fn place_staged(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    expected: LocalFileIdentity,
) -> Result<Placed, PlaceError> {
    ladder(fs, from, to, Some(expected))
}

/// Unlike ordinary placement reconciliation, a missing private copy stage after
/// a failed rename is unconfirmed: callers cannot infer where its bytes went.
pub(super) fn publish_staged(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    expected: LocalFileIdentity,
) -> Result<Placed, PlaceError> {
    first_placement(Placing { fs, from, to }, Some(expected), true)
}

fn verified(
    fs: &dyn FsOps,
    to: Entry<'_>,
    expected: Option<LocalFileIdentity>,
    placed: Result<Placed, PlaceError>,
) -> Result<Placed, PlaceError> {
    let placed = placed?;
    let Some(expected) = expected else {
        return Ok(placed);
    };
    if placed == Placed::RecreatedSymlink {
        return Ok(placed);
    }
    match fs.stat(to, false) {
        Ok(current) if current.identity == expected => Ok(placed),
        _ => Err(PlaceError::Unconfirmed),
    }
}

fn ladder(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    expected: Option<LocalFileIdentity>,
) -> Result<Placed, PlaceError> {
    first_placement(Placing { fs, from, to }, expected, false)
}

fn stat_source(fs: &dyn FsOps, from: Entry<'_>) -> Result<EntryStat, PlaceError> {
    fs.stat(from, true).map_err(PlaceError::Io)
}

fn exists(fs: &dyn FsOps, entry: Entry<'_>) -> Result<bool, PlaceError> {
    match fs.stat(entry, false) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(PlaceError::Io(error)),
    }
}

/// An `EEXIST` can be the lost reply of a link that did happen; a case alias
/// or an older hard link fails the link-count or exact-name test.
fn link_reply_was_lost(fs: &dyn FsOps, to: Entry<'_>, source: &EntryStat) -> bool {
    fs.stat(to, true)
        .is_ok_and(|target| target.identity == source.identity && target.nlink == source.nlink + 1)
        && fs
            .names(at(to.dir, OsStr::new(".")))
            .is_ok_and(|names| names.iter().any(|name| name == to.name))
}

/// Removes `entry` while it still holds `identity`, and tells whether it is
/// gone: an NFS unlink retried after its reply was lost reports `ENOENT`.
pub(super) fn remove_if_unchanged(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    identity: LocalFileIdentity,
    attributes: Option<Attributes>,
    flags: AtFlags,
) -> bool {
    let recorded = Recorded {
        identity,
        attributes,
    };
    let removable = |current: EntryStat| {
        native_fs::removable(
            recorded,
            &current,
            || mount_profile::identity_at(fs, entry.dir),
            flags.contains(AtFlags::REMOVEDIR),
        )
    };
    fs.stat(entry, true).is_ok_and(removable)
        && matches!(fs.unlink(entry, flags), Ok(()) | Err(Errno::NOENT))
}

fn placeholder_is_ours(fs: &dyn FsOps, to: Entry<'_>, placeholder: LocalFileIdentity) -> bool {
    fs.stat(to, true)
        .is_ok_and(|current| current.identity == placeholder)
        && fs.names(to).is_ok_and(|names| names.is_empty())
}

fn remove_placeholder(fs: &dyn FsOps, to: Entry<'_>, placeholder: LocalFileIdentity) -> bool {
    placeholder_is_ours(fs, to, placeholder) && fs.unlink(to, AtFlags::REMOVEDIR).is_ok()
}

/// Whether `dir` is `ancestor` or inside it, walking `..` (which crosses
/// mount boundaries) up to the root. `None` when the walk fails.
pub(super) fn contains(
    fs: &dyn FsOps,
    ancestor: LocalFileIdentity,
    dir: BorrowedFd<'_>,
) -> Option<bool> {
    let mut current = fs.stat_fd(dir).ok()?;
    if current.identity == ancestor {
        return Some(true);
    }
    let mut handle = fs.open_parent(dir).ok()?;
    for _ in 0..4096 {
        let parent = fs.stat_fd(handle.as_fd()).ok()?;
        if parent.identity == ancestor {
            return Some(true);
        }
        if parent.identity == current.identity {
            return Some(false);
        }
        current = parent;
        handle = fs.open_parent(handle.as_fd()).ok()?;
    }
    None
}

/// The entry at a fresh hidden name that this process's own create, whose
/// reply was lost, left there: nobody else knows the name yet, so an entry
/// as fresh as a create leaves it is that create's.
pub(super) fn left_by_lost_reply(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    fresh: impl FnOnce(&EntryStat) -> bool,
) -> Option<LocalFileIdentity> {
    fs.stat(entry, true)
        .ok()
        .filter(fresh)
        .map(|stat| stat.identity)
}

pub(super) fn reserve_hidden_name<T>(
    hidden: HiddenName,
    mut attempt: impl FnMut(&OsStr) -> Result<T, PlaceError>,
) -> Result<(OsString, T), PlaceError> {
    for name in std::iter::repeat_with(|| hidden.fresh()).take(NAME_ATTEMPTS) {
        match attempt(&name) {
            Err(PlaceError::Exists) => continue,
            result => return result.map(|value| (name, value)),
        }
    }
    Err(PlaceError::Exists)
}

pub(super) fn creation_error(error: Errno) -> PlaceError {
    match error {
        Errno::EXIST => PlaceError::Exists,
        Errno::INVAL => PlaceError::InvalidName,
        error => PlaceError::Io(error),
    }
}

/// For `O_CREAT | O_EXCL` files: SMB with POSIX extensions reports a folder
/// already at the name as `EISDIR`.
pub(super) fn file_creation_error(error: Errno) -> PlaceError {
    match error {
        Errno::ISDIR => PlaceError::Exists,
        error => creation_error(error),
    }
}

/// An entry being placed, and the name it goes to.
#[derive(Clone, Copy)]
struct Placing<'a> {
    fs: &'a dyn FsOps,
    from: Entry<'a>,
    to: Entry<'a>,
}

fn first_placement(
    placing: Placing<'_>,
    expected: Option<LocalFileIdentity>,
    require_private_stage: bool,
) -> Result<Placed, PlaceError> {
    let Placing { fs, from, to } = placing;
    let error = match fs.rename(from, to, RenameFlags::NOREPLACE) {
        Ok(()) => return Ok(Placed::Renamed),
        Err(Errno::EXIST | Errno::NOTEMPTY) => return Err(PlaceError::Exists),
        Err(Errno::XDEV) => return Err(PlaceError::CrossDevice),
        Err(error) => error,
    };
    if require_private_stage {
        match fs.stat(from, false) {
            Ok(current) if Some(current.identity) == expected => {}
            Ok(_) => return Err(PlaceError::Changed),
            Err(Errno::NOENT) => return Err(PlaceError::Unconfirmed),
            Err(error) => return Err(PlaceError::Io(error)),
        }
    }
    rungs::fallback(placing, expected, error)
}

fn resolve(
    placing: &Placing<'_>,
    identity: Option<LocalFileIdentity>,
    error: Errno,
) -> Result<(), PlaceError> {
    let Placing { fs, from, to } = *placing;
    let Some(identity) = identity else {
        return Err(PlaceError::Io(error));
    };
    match (fs.stat(from, true), fs.stat(to, true)) {
        (Err(Errno::NOENT), Ok(target)) if target.identity == identity => Ok(()),
        (Ok(source), Err(Errno::NOENT)) if source.identity == identity => {
            Err(PlaceError::Io(error))
        }
        (Ok(source), Ok(target)) if source.identity == identity && target.identity != identity => {
            Err(PlaceError::Io(error))
        }
        _ => Err(PlaceError::Unconfirmed),
    }
}

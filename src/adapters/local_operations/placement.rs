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

/// Looks the mount key up only when a fallback needs it, so the first rung
/// costs a single syscall.
struct Mount<'a> {
    fs: &'a dyn FsOps,
    dir: BorrowedFd<'a>,
    key: Cell<Option<MountKey>>,
    probed_links: Cell<bool>,
}

impl<'a> Mount<'a> {
    fn new(fs: &'a dyn FsOps, dir: BorrowedFd<'a>) -> Self {
        Self {
            fs,
            dir,
            key: Cell::new(None),
            probed_links: Cell::new(false),
        }
    }

    fn key(&self) -> MountKey {
        if let Some(key) = self.key.get() {
            return key;
        }
        let key = self.fs.mount_key(self.dir);
        self.key.set(Some(key));
        key
    }

    fn capabilities(&self) -> MountCapabilities {
        mount_capabilities(self.key())
    }

    /// Whether an inode number still found at a name proves the entry is
    /// the one it was read from, as the mount's type tells.
    fn numbers_kept(&self) -> bool {
        mount_profile::identity_at(self.fs, self.dir).numbers == InodeNumbers::Kept
    }

    fn learn(&self, learn: impl FnOnce(&mut MountCapabilities)) {
        learn_mount_capability(self.key(), learn);
    }

    fn probe(&self, links: bool, folders: bool) -> Result<Probed, Errno> {
        let renames = self.capabilities().no_replace.is_none();
        self.probed_links.set(self.probed_links.get() || links);
        let probed = probe(self.fs, self.dir, renames, links, folders)?;
        self.learn(|capabilities| {
            if probed.links_keep_numbers {
                capabilities.links_keep_numbers = true;
                capabilities.identity =
                    mount_profile::with_links(capabilities.family, capabilities.identity, true);
            }
            for (field, value) in [
                (&mut capabilities.no_replace, probed.no_replace),
                (&mut capabilities.hard_links, probed.hard_links),
                (
                    &mut capabilities.rename_over_empty_directory,
                    probed.rename_over_empty_directory,
                ),
            ] {
                if value.is_some() {
                    *field = value;
                }
            }
        });
        Ok(probed)
    }

    /// Whether the reserved-name rung may run once the others gave up: only
    /// after a probe on Strata's own names, or a definitive errno, shows that
    /// NOREPLACE is missing and settles whether links work. Files take it where
    /// links are missing or unusable on the mount, or refused for that one file
    /// (`protected_hardlinks`); folders only where links are missing too.
    fn allows_reserved_name(&self, directory: bool) -> bool {
        let settled = |capabilities: MountCapabilities| {
            capabilities.no_replace.is_some() && capabilities.hard_links.is_some()
        };
        if !settled(self.capabilities()) && !self.probed_links.get() {
            let _ = self.probe(true, false);
        }
        let capabilities = self.capabilities();
        capabilities.no_replace == Some(false)
            && match capabilities.hard_links {
                Some(false) => true,
                // The link rung only gives up on a file whose own link was refused.
                Some(true) => !directory,
                None => false,
            }
    }

    fn renames_over(&self, name: &OsStr) -> bool {
        units(name.as_encoded_bytes()) <= LONGEST_NAME_NTFS_3G_RENAMES_OVER
            || !mount_profile::known(self.fs, self.dir, self.key()).cuts_long_names_renamed_over
    }
}

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
    first_placement(fs, from, to, Some(expected), true)
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
    first_placement(fs, from, to, expected, false)
}

fn first_placement(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    expected: Option<LocalFileIdentity>,
    require_private_stage: bool,
) -> Result<Placed, PlaceError> {
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
    fallback(fs, from, to, expected, error)
}

fn fallback(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    expected: Option<LocalFileIdentity>,
    error: Errno,
) -> Result<Placed, PlaceError> {
    let mount = Mount::new(fs, to.dir);
    let mut source = None;
    match error {
        Errno::EXIST | Errno::NOTEMPTY => return Err(PlaceError::Exists),
        Errno::XDEV => return Err(PlaceError::CrossDevice),
        Errno::INVAL => {
            let stat = stat_source(fs, from)?;
            let known = mount.capabilities().no_replace;
            // Once NOREPLACE is known missing, the folder rung's own rename or the
            // caller's copy guard catches a move into itself.
            if stat.is_directory()
                && known != Some(false)
                && contains(fs, stat.identity, to.dir) != Some(false)
            {
                return Err(PlaceError::IntoItself);
            }
            let no_replace = match known {
                None => mount
                    .probe(!stat.is_directory(), false)
                    .ok()
                    .and_then(|_| mount.capabilities().no_replace),
                known => known,
            };
            if no_replace == Some(true) {
                return Err(PlaceError::InvalidName);
            }
            source = Some(stat);
        }
        error if is_unsupported(error) => {
            mount.learn(|capabilities| capabilities.no_replace = Some(false));
        }
        error if may_have_lost_reply(error) => {
            return resolve(fs, from, to, expected, error).map(|()| Placed::Renamed);
        }
        error => return Err(PlaceError::Io(error)),
    }

    let source = match source {
        Some(source) => source,
        None => stat_source(fs, from)?,
    };
    if expected.is_some_and(|expected| expected != source.identity) {
        return Err(PlaceError::Changed);
    }
    let directory = source.is_directory();
    let placed = if directory {
        place_directory(fs, &mount, from, to, &source)
    } else {
        place_non_directory(fs, &mount, from, to, &source)
    };
    match placed {
        Err(PlaceError::Unsupported) if mount.allows_reserved_name(directory) => {
            if directory {
                rename_folder_after_check(fs, from, to, &source)
            } else if mount.renames_over(to.name) {
                rename_over_reserved_file(fs, &mount, from, to, &source)
            } else {
                Err(PlaceError::Unsupported)
            }
        }
        placed => placed,
    }
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

fn resolve(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    identity: Option<LocalFileIdentity>,
    error: Errno,
) -> Result<(), PlaceError> {
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

fn place_non_directory(
    fs: &dyn FsOps,
    mount: &Mount<'_>,
    from: Entry<'_>,
    to: Entry<'_>,
    source: &EntryStat,
) -> Result<Placed, PlaceError> {
    if mount.capabilities().hard_links != Some(false) {
        match fs.link(from, to) {
            Ok(()) => {
                if link_holds_source(fs, mount, from, to, source)? {
                    return finish_link(fs, from, to, source);
                }
            }
            Err(Errno::EXIST) => {
                return if link_reply_was_lost(fs, to, source) {
                    finish_link(fs, from, to, source)
                } else {
                    Err(PlaceError::Exists)
                };
            }
            Err(error @ (Errno::PERM | Errno::ACCESS)) => {
                if exists(fs, to)? {
                    return Err(PlaceError::Exists);
                }
                // Linking can be refused for this item alone (protected_hardlinks) or by
                // an SFTP server without a link extension (EACCES), so only a probe
                // teaches the mount anything; a folder that refuses the probe's own
                // file refused the link too.
                if mount.capabilities().hard_links.is_none()
                    && mount.probe(true, false) == Err(Errno::ACCESS)
                {
                    return Err(PlaceError::Io(error));
                }
            }
            Err(Errno::MLINK) => {}
            Err(Errno::XDEV) => return Err(PlaceError::CrossDevice),
            Err(Errno::INVAL) => return Err(PlaceError::InvalidName),
            Err(error) if is_unsupported(error) => {
                mount.learn(|capabilities| capabilities.hard_links = Some(false));
            }
            Err(error) if may_have_lost_reply(error) => {
                return match fs.stat(to, true) {
                    Ok(target) if target.identity == source.identity => {
                        finish_link(fs, from, to, source)
                    }
                    Ok(_) | Err(Errno::NOENT) => Err(PlaceError::Io(error)),
                    Err(_) => Err(PlaceError::Unconfirmed),
                };
            }
            Err(error) => return Err(PlaceError::Io(error)),
        }
    }
    recreate_symlink(fs, from, to, source)
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

/// Only the source's identity at the new name proves a link; a link count can
/// be stale, or always 1 on sshfs. Another identity there is a racer's item,
/// or a mount where each link reports its own inode number (libfuse without
/// `use_ino`, cifs without server inode numbers). Only a probe on Strata's own
/// names tells them apart; on such a mount a link that matches the source's
/// type, mode, size and time is removed by the identity it reported, and links
/// are not used again.
fn link_holds_source(
    fs: &dyn FsOps,
    mount: &Mount<'_>,
    from: Entry<'_>,
    to: Entry<'_>,
    source: &EntryStat,
) -> Result<bool, PlaceError> {
    let linked = fs.stat(to, true).map_err(|_| PlaceError::Unconfirmed)?;
    if linked.identity == source.identity {
        return Ok(true);
    }
    let current = match fs.stat(from, true) {
        Ok(current) if current.identity == source.identity => current,
        Ok(_) => return Err(PlaceError::Changed),
        Err(_) => return Err(PlaceError::Unconfirmed),
    };
    let alike = (linked.kind, linked.mode, linked.size, linked.mtime)
        == (current.kind, current.mode, current.size, current.mtime);
    let links_unusable = mount
        .probe(true, false)
        .is_ok_and(|probed| probed.hard_links == Some(false));
    if links_unusable
        && alike
        && remove_if_unchanged(
            fs,
            to,
            linked.identity,
            linked.kept_attributes(),
            AtFlags::empty(),
        )
    {
        Ok(false)
    } else {
        Err(PlaceError::Unconfirmed)
    }
}

fn finish_link(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    source: &EntryStat,
) -> Result<Placed, PlaceError> {
    let current = match fs.stat(from, true) {
        Ok(current) => current,
        Err(Errno::NOENT) => return Ok(Placed::Linked),
        Err(_) => return Err(PlaceError::Unconfirmed),
    };
    if current.identity != source.identity {
        return Err(PlaceError::Changed);
    }
    match fs.unlink(from, AtFlags::empty()) {
        Ok(()) => Ok(Placed::Linked),
        Err(Errno::NOENT) => {
            resolve(fs, from, to, Some(source.identity), Errno::NOENT).map(|()| Placed::Linked)
        }
        Err(error) => {
            if fs
                .stat(to, true)
                .is_ok_and(|target| target.identity == source.identity && target.nlink >= 2)
            {
                let _ = fs.unlink(to, AtFlags::empty());
            }
            Err(PlaceError::Io(error))
        }
    }
}

fn recreate_symlink(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    source: &EntryStat,
) -> Result<Placed, PlaceError> {
    if source.kind != FileType::Symlink {
        return Err(PlaceError::Unsupported);
    }
    let link = fs.readlink(from).map_err(PlaceError::Io)?;
    match fs.symlink(&link, to) {
        Ok(()) => {}
        Err(Errno::EXIST) => return Err(PlaceError::Exists),
        Err(Errno::PERM | Errno::ACCESS) if exists(fs, to)? => return Err(PlaceError::Exists),
        Err(Errno::PERM | Errno::ACCESS) => return Err(PlaceError::Unsupported),
        Err(Errno::INVAL) => return Err(PlaceError::InvalidName),
        Err(error) => return Err(PlaceError::Io(error)),
    }
    let created = fs.stat(to, false).map_err(|_| PlaceError::Unconfirmed)?;
    let remove_created = || {
        remove_if_unchanged(
            fs,
            to,
            created.identity,
            created.kept_attributes(),
            AtFlags::empty(),
        )
    };
    match fs.stat(from, true) {
        Ok(current) if current.identity == source.identity => {
            match fs.unlink(from, AtFlags::empty()) {
                Ok(()) | Err(Errno::NOENT) => Ok(Placed::RecreatedSymlink),
                Err(error) => {
                    remove_created();
                    Err(PlaceError::Io(error))
                }
            }
        }
        Err(Errno::NOENT) => Ok(Placed::RecreatedSymlink),
        _ => {
            remove_created();
            Err(PlaceError::Changed)
        }
    }
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

fn place_directory(
    fs: &dyn FsOps,
    mount: &Mount<'_>,
    from: Entry<'_>,
    to: Entry<'_>,
    source: &EntryStat,
) -> Result<Placed, PlaceError> {
    if !mount.renames_over(to.name) {
        return if mount.capabilities().no_replace == Some(false) {
            rename_folder_after_check(fs, from, to, source)
        } else {
            Err(PlaceError::Unsupported)
        };
    }
    if mount.capabilities().rename_over_empty_directory == Some(false) {
        return Err(PlaceError::Unsupported);
    }
    match fs.mkdir(to, Mode::from_raw_mode(0o700)) {
        Ok(()) => {}
        Err(Errno::EXIST) => return Err(PlaceError::Exists),
        Err(Errno::PERM) if exists(fs, to)? => return Err(PlaceError::Exists),
        Err(Errno::INVAL) => return Err(PlaceError::InvalidName),
        // A lost reply leaves an empty folder that cannot be told apart from a racer's.
        Err(error) if may_have_lost_reply(error) => {
            return Err(if exists(fs, to)? {
                PlaceError::Exists
            } else {
                PlaceError::Io(error)
            });
        }
        Err(error) => return Err(PlaceError::Io(error)),
    }
    let placeholder = fs.stat(to, false).map_err(PlaceError::Io)?.identity;
    match fs.rename(from, to, RenameFlags::empty()) {
        Ok(()) => Ok(Placed::ReservedDirectory),
        Err(error @ (Errno::NOTEMPTY | Errno::EXIST | Errno::PERM)) => {
            match fs.stat(to, true) {
                Ok(current) if current.identity == placeholder => match fs.names(to) {
                    Ok(names) if names.is_empty() => {}
                    Ok(_) => return Err(PlaceError::Exists),
                    Err(error) => return Err(PlaceError::Io(error)),
                },
                Ok(_) | Err(Errno::NOENT) => return Err(PlaceError::Exists),
                Err(error) => return Err(PlaceError::Io(error)),
            }
            let _ = fs.unlink(to, AtFlags::REMOVEDIR);
            if error != Errno::PERM {
                mount.learn(|capabilities| capabilities.rename_over_empty_directory = Some(false));
            }
            Err(PlaceError::Unsupported)
        }
        Err(Errno::NOTDIR | Errno::ISDIR) => Err(PlaceError::Exists),
        Err(Errno::INVAL) => {
            remove_placeholder(fs, to, placeholder);
            if contains(fs, source.identity, to.dir) == Some(false) {
                Err(PlaceError::InvalidName)
            } else {
                Err(PlaceError::IntoItself)
            }
        }
        Err(Errno::XDEV) => {
            remove_placeholder(fs, to, placeholder);
            Err(PlaceError::CrossDevice)
        }
        Err(error) if may_have_lost_reply(error) => {
            if mount.numbers_kept() && remove_placeholder(fs, to, placeholder) {
                Err(PlaceError::Io(error))
            } else {
                moved_despite_lost_reply(fs, from, to, source, Placed::ReservedDirectory)
            }
        }
        Err(error) => {
            remove_placeholder(fs, to, placeholder);
            Err(PlaceError::Io(error))
        }
    }
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Probed {
    pub no_replace: Option<bool>,
    pub hard_links: Option<bool>,
    pub rename_over_empty_directory: Option<bool>,
    pub links_keep_numbers: bool,
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

/// Tries each primitive on new names of Strata's own in `dir`, where no
/// errno can be blamed on the user's item. Fields stay `None` when the probe
/// cannot tell.
pub(super) fn probe(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    renames: bool,
    links: bool,
    folders: bool,
) -> Result<Probed, Errno> {
    let mut probed = Probed::default();
    let created = reserve_hidden_name(HiddenName::Probe, |name| {
        drop(
            fs.create_exclusive(at(dir, name), Mode::from_raw_mode(0o600))
                .map_err(file_creation_error)?,
        );
        fs.stat(at(dir, name), false)
            .map(|stat| stat.identity)
            .map_err(PlaceError::Io)
    });
    let (mut current, identity) = match created {
        Ok(created) => created,
        Err(PlaceError::Io(error)) => return Err(error),
        Err(_) => return Ok(probed),
    };

    if renames
        && let Ok((renamed, supported)) = reserve_hidden_name(HiddenName::Probe, |name| {
            match fs.rename(at(dir, &current), at(dir, name), RenameFlags::NOREPLACE) {
                Ok(()) => Ok(Some(true)),
                Err(Errno::EXIST) => Err(PlaceError::Exists),
                Err(error) if error == Errno::INVAL || is_unsupported(error) => Ok(Some(false)),
                Err(_) => Ok(None),
            }
        })
    {
        probed.no_replace = supported;
        if supported == Some(true) {
            current = renamed;
        }
    }

    if links
        && let Ok((linked, supported)) = reserve_hidden_name(HiddenName::Probe, |name| {
            match fs.link(at(dir, &current), at(dir, name)) {
                Ok(()) => Ok(Some(true)),
                Err(Errno::EXIST) => Err(PlaceError::Exists),
                Err(error)
                    if matches!(error, Errno::PERM | Errno::ACCESS) || is_unsupported(error) =>
                {
                    Ok(Some(false))
                }
                Err(_) => Ok(None),
            }
        })
    {
        probed.hard_links = supported;
        if supported == Some(true) {
            probed.hard_links = match fs.stat(at(dir, &linked), true) {
                Ok(stat) => {
                    remove_probe_file(fs, at(dir, &linked), stat.identity);
                    probed.links_keep_numbers = stat.identity == identity && stat.nlink == 2;
                    // A link that reports another inode cannot prove a placement.
                    Some(stat.identity == identity)
                }
                Err(_) => None,
            };
        }
    }
    remove_probe_file(fs, at(dir, &current), identity);

    if folders {
        probed.rename_over_empty_directory = probe_folders(fs, dir);
    }
    Ok(probed)
}

fn probe_folders(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> Option<bool> {
    let mkdir = |name: &OsStr| {
        fs.mkdir(at(dir, name), Mode::from_raw_mode(0o700))
            .map_err(creation_error)?;
        fs.stat(at(dir, name), false)
            .map(|stat| stat.identity)
            .map_err(PlaceError::Io)
    };
    let (moved, moved_identity) = reserve_hidden_name(HiddenName::Probe, mkdir).ok()?;
    let Ok((target, target_identity)) = reserve_hidden_name(HiddenName::Probe, mkdir) else {
        remove_placeholder(fs, at(dir, &moved), moved_identity);
        return None;
    };
    let result = match fs.rename(at(dir, &moved), at(dir, &target), RenameFlags::empty()) {
        Ok(()) => {
            remove_placeholder(fs, at(dir, &target), moved_identity);
            return Some(true);
        }
        Err(Errno::NOTEMPTY | Errno::EXIST)
            if placeholder_is_ours(fs, at(dir, &target), target_identity) =>
        {
            Some(false)
        }
        Err(_) => None,
    };
    remove_placeholder(fs, at(dir, &moved), moved_identity);
    remove_placeholder(fs, at(dir, &target), target_identity);
    result
}

fn remove_probe_file(fs: &dyn FsOps, entry: Entry<'_>, made: LocalFileIdentity) {
    if fs
        .stat(entry, true)
        .is_ok_and(|current| current.identity == made && current.size == 0)
    {
        let _ = fs.unlink(entry, AtFlags::empty());
    }
}

pub(super) struct Originals {
    pub name: OsString,
    handle: OffMain<Arc<OwnedFd>>,
}

impl Originals {
    pub(super) fn create(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> Result<Self, PlaceError> {
        let (name, identity) = reserve_hidden_name(HiddenName::Originals, |name| {
            fs.mkdir(at(dir, name), Mode::from_raw_mode(0o700))
                .map_err(creation_error)?;
            fs.stat(at(dir, name), false)
                .map(|stat| stat.identity)
                .map_err(PlaceError::Io)
        })
        .map_err(|error| match error {
            PlaceError::InvalidName => PlaceError::Unsupported,
            error => error,
        })?;
        let opened = open_beneath(fs, dir, Path::new(&name)).and_then(|handle| {
            match fs.stat_fd(handle.as_fd()) {
                Ok(stat) if stat.identity == identity => Ok(handle),
                Ok(_) => Err(PlaceError::Changed),
                Err(error) => Err(PlaceError::Io(error)),
            }
        });
        match opened {
            Ok(handle) => Ok(Self {
                name,
                handle: OffMain::new(Arc::new(handle)),
            }),
            Err(error) => {
                remove_if_unchanged(fs, at(dir, &name), identity, None, AtFlags::REMOVEDIR);
                Err(error)
            }
        }
    }

    pub(super) fn entry<'a>(&'a self, name: &'a OsStr) -> Entry<'a> {
        Entry::new(&**self.handle, name)
    }

    pub(super) fn handle(&self) -> Arc<OwnedFd> {
        Arc::clone(&self.handle)
    }

    pub(super) fn kept(&self, name: &OsStr) -> OsString {
        Path::new(&self.name).join(name).into_os_string()
    }

    pub(super) fn remove(
        &self,
        fs: &dyn FsOps,
        dir: BorrowedFd<'_>,
        deadline: &Deadline,
    ) -> Result<(), Refused> {
        let folder = at(dir, &self.name);
        leftovers::remove_held_folder(fs, folder, self.handle.as_fd(), deadline, &|| false)
    }

    pub(super) fn refusal(&self, fs: &dyn FsOps, dir: BorrowedFd<'_>, refused: Refused) -> String {
        let folder = at(dir, &self.name);
        match refused {
            Refused::StillOpen(_) => {
                leftovers::still_open(fs, folder).map(crate::services::error_detail)
            }
            Refused::Changed(_) => Some(
                rust_i18n::t!(
                    "%{name} changed while the operation was in progress",
                    name = self.name.to_string_lossy()
                )
                .into_owned(),
            ),
            _ => None,
        }
        .unwrap_or_else(|| native_fs::os_error_detail(refused.errno()))
    }
}

pub(super) struct SetAside {
    pub originals: Originals,
    pub name: OsString,
    pub identity: LocalFileIdentity,
    pub attributes: Option<Attributes>,
}

impl SetAside {
    pub(super) fn entry(&self) -> Entry<'_> {
        self.originals.entry(&self.name)
    }

    pub(super) fn kept(&self) -> OsString {
        self.originals.kept(&self.name)
    }
}

pub(super) fn open_beneath(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    relative: &Path,
) -> Result<OwnedFd, PlaceError> {
    native_fs::retry_local_open(|| {
        fs.open_at(
            at(dir, relative.as_os_str()),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            Some(
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            ),
        )
    })
    .map_err(|error| match error {
        Errno::XDEV => PlaceError::CrossDevice,
        error => PlaceError::Io(error),
    })
}

pub(super) fn move_aside(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    expected: LocalFileIdentity,
    attributes: Option<Attributes>,
) -> Result<(LocalFileIdentity, Option<Attributes>), PlaceError> {
    let current = match fs.stat(from, false) {
        Ok(current)
            if current.identity == expected
                || native_fs::unchanged(
                    mount_profile::identity_at(fs, from.dir),
                    expected,
                    attributes,
                    &current,
                ) =>
        {
            current.identity
        }
        Ok(_) | Err(Errno::NOENT) => return Err(PlaceError::Changed),
        Err(error) => return Err(PlaceError::Io(error)),
    };
    match fs.rename(from, to, RenameFlags::empty()) {
        Ok(()) => verified(fs, to, Some(current), Ok(Placed::Renamed)),
        Err(Errno::XDEV) => Err(PlaceError::CrossDevice),
        Err(error) if may_have_lost_reply(error) => {
            resolve(fs, from, to, Some(current), error).map(|()| Placed::Renamed)
        }
        Err(error) => Err(PlaceError::Io(error)),
    }
    .map(|_| (current, attributes))
}

pub(super) fn set_aside(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    expected: LocalFileIdentity,
    attributes: Option<Attributes>,
) -> Result<SetAside, RenameFailure> {
    let originals = Originals::create(fs, entry.dir).map_err(RenameFailure::Place)?;
    let moved = with_owner_write(fs, entry, expected, attributes, || {
        move_aside(fs, entry, originals.entry(entry.name), expected, attributes)
    });
    match moved {
        Ok((identity, attributes)) => Ok(SetAside {
            originals,
            name: entry.name.to_os_string(),
            identity,
            attributes,
        }),
        Err(error @ PlaceError::Unconfirmed) => Err(RenameFailure::Stranded {
            temporary: originals.kept(entry.name),
            error,
        }),
        Err(error) => {
            let _ = originals.remove(fs, entry.dir, &Deadline::default());
            Err(RenameFailure::Place(error))
        }
    }
}

pub(super) fn put_back(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    aside: &SetAside,
    name: &OsStr,
) -> Result<(), PlaceError> {
    place_back(
        fs,
        aside.entry(),
        at(dir, name),
        aside.identity,
        aside.attributes,
    )?;
    let _ = aside.originals.remove(fs, dir, &Deadline::default());
    Ok(())
}

pub(super) fn place_back(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    identity: LocalFileIdentity,
    attributes: Option<Attributes>,
) -> Result<Placed, PlaceError> {
    let current = match fs.stat(from, false) {
        Ok(current) => current,
        Err(_) => return place(fs, from, to, Some(identity)),
    };
    if !native_fs::unchanged(
        mount_profile::identity_at(fs, from.dir),
        identity,
        attributes,
        &current,
    ) {
        return Err(PlaceError::Changed);
    }
    with_owner_write(fs, from, current.identity, None, || {
        place(fs, from, to, Some(current.identity))
    })
}

fn with_owner_write<T>(
    fs: &dyn FsOps,
    folder: Entry<'_>,
    expected: LocalFileIdentity,
    attributes: Option<Attributes>,
    moved: impl Fn() -> Result<T, PlaceError>,
) -> Result<T, PlaceError> {
    let refused = match moved() {
        Err(refused @ PlaceError::Io(Errno::ACCESS)) => refused,
        result => return result,
    };
    let Ok(opened) = fs.open_dir(folder) else {
        return Err(refused);
    };
    let locked = fs.stat_fd(opened.as_fd()).ok().filter(|stat| {
        stat.mode & 0o200 == 0
            && native_fs::unchanged(
                mount_profile::identity_at(fs, folder.dir),
                expected,
                attributes,
                stat,
            )
    });
    let Some(locked) = locked else {
        return Err(refused);
    };
    if fs.chmod_fd(opened.as_fd(), locked.mode | 0o200).is_err() {
        return Err(refused);
    }
    let result = moved();
    let _ = fs.chmod_fd(opened.as_fd(), locked.mode);
    result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Feasibility {
    Yes,
    No,
}

fn placeable(capabilities: MountCapabilities, directory: bool) -> Option<bool> {
    let fallback = if directory {
        capabilities.rename_over_empty_directory
    } else {
        capabilities.hard_links
    };
    match (capabilities.no_replace, fallback, capabilities.hard_links) {
        (Some(true), _, _) | (_, Some(true), _) => Some(true),
        (Some(false), _, Some(false)) => Some(true),
        (Some(false), Some(false), Some(true)) => Some(false),
        _ => None,
    }
}

pub(super) fn links_hold(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> bool {
    let mount = Mount::new(fs, dir);
    if mount.capabilities().hard_links.is_none() {
        let _ = mount.probe(true, false);
    }
    mount.capabilities().hard_links == Some(true)
}

/// Whether a staged entry could be placed in `dir`; `None` until the mount
/// is known, without probing it.
pub(super) fn can_place(fs: &dyn FsOps, dir: BorrowedFd<'_>, directory: bool) -> Option<bool> {
    placeable(Mount::new(fs, dir).capabilities(), directory)
}

/// Whether an existing entry in `dir` can be moved aside without risking a
/// clobber, probing the mount when nothing is known yet.
pub(super) fn can_set_aside(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    directory: bool,
) -> Result<Feasibility, PlaceError> {
    let mount = Mount::new(fs, dir);
    let known = match placeable(mount.capabilities(), directory) {
        Some(known) => known,
        None => {
            mount.probe(true, directory).map_err(PlaceError::Io)?;
            placeable(mount.capabilities(), directory) == Some(true)
        }
    };
    Ok(if known {
        Feasibility::Yes
    } else {
        Feasibility::No
    })
}

/// `Ok(false)`: the filesystem rejected the flag, which is remembered for
/// the mount.
pub(super) fn exchange(fs: &dyn FsOps, from: Entry<'_>, to: Entry<'_>) -> Result<bool, PlaceError> {
    match fs.rename(from, to, RenameFlags::EXCHANGE) {
        Ok(()) => Ok(true),
        Err(error) if error == Errno::INVAL || is_unsupported(error) => {
            Mount::new(fs, to.dir).learn(|capabilities| capabilities.exchange = Some(false));
            Ok(false)
        }
        Err(Errno::XDEV) => Err(PlaceError::CrossDevice),
        Err(error) => Err(PlaceError::Io(error)),
    }
}

pub(super) enum Exchanged {
    Swapped(Originals),
    Refused,
}

pub(super) fn exchange_through(
    fs: &dyn FsOps,
    stage: Entry<'_>,
    target: Entry<'_>,
    copy: LocalFileIdentity,
    attributes: Option<Attributes>,
) -> Result<Exchanged, PlaceError> {
    let denied = |error| matches!(error, PlaceError::Io(Errno::ACCESS | Errno::PERM));
    let originals = Originals::create(fs, target.dir)?;
    let parked = originals.entry(target.name);
    let copy = match move_aside(fs, stage, parked, copy, attributes) {
        Ok((copy, _)) => copy,
        Err(error) => {
            if error != PlaceError::Unconfirmed {
                let _ = originals.remove(fs, target.dir, &Deadline::default());
            }
            return if denied(error) {
                Ok(Exchanged::Refused)
            } else {
                Err(error)
            };
        }
    };
    let exchanged = exchange(fs, parked, target);
    if let Ok(true) = exchanged {
        return Ok(Exchanged::Swapped(originals));
    }
    place(fs, parked, stage, Some(copy))?;
    let _ = originals.remove(fs, target.dir, &Deadline::default());
    match exchanged {
        Err(error) if !denied(error) => Err(error),
        _ => Ok(Exchanged::Refused),
    }
}

/// Filesystems known to implement `RENAME_EXCHANGE` faithfully. FUSE,
/// network, overlay and the other FAT-family drivers are left out because
/// some silently overwrite instead.
const EXCHANGE_FILESYSTEMS: [u32; 8] = [
    0xEF53,      // ext2/3/4
    0x5846_5342, // XFS
    0x9123_683E, // btrfs
    0xF2F5_2010, // f2fs
    0x0102_1994, // tmpfs
    0xCA45_1A4E, // bcachefs
    0x2FC1_2FC1, // ZFS
    0x4D44,      // vfat
];

pub(super) fn exchange_allowed(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> bool {
    fs.statfs(dir)
        .is_some_and(|stat| EXCHANGE_FILESYSTEMS.contains(&(stat.kind as u32)))
        && Mount::new(fs, dir).capabilities().exchange != Some(false)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Displacement {
    Exchange,
    SetAside,
}

pub(super) fn displacement_without_trash(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    directory: bool,
) -> Result<Option<Displacement>, PlaceError> {
    if exchange_allowed(fs, dir) {
        return Ok(Some(Displacement::Exchange));
    }
    set_aside_if_safe(fs, dir, directory)
}

pub(super) fn set_aside_if_safe(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    directory: bool,
) -> Result<Option<Displacement>, PlaceError> {
    Ok((can_set_aside(fs, dir, directory)? == Feasibility::Yes).then_some(Displacement::SetAside))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Renamed {
    Direct(Placed),
    CaseOnly,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RenameFailure {
    Place(PlaceError),
    Stranded {
        temporary: OsString,
        error: PlaceError,
    },
}

/// Renames `from` to `to` without replacing another item. A name that only
/// differs in case on a case-insensitive mount resolves to the item itself,
/// so it is renamed in two steps through a hidden temporary name.
pub(super) fn rename_in_place(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
) -> Result<Renamed, RenameFailure> {
    if from.name == to.name && same_directory(fs, from.dir, to.dir) {
        return Ok(Renamed::Unchanged);
    }
    match place(fs, from, to, None) {
        Err(PlaceError::Exists) => {}
        result => return result.map(Renamed::Direct).map_err(RenameFailure::Place),
    }
    let Some(identity) = alias_of(fs, from, to) else {
        return Err(RenameFailure::Place(PlaceError::Exists));
    };
    let aside = set_aside(fs, from, identity, None)?;
    // The SMB client keeps answering for the lookup of `to` for about a
    // second; revalidating drops it.
    let _ = fs.stat(to, true);
    let renamed = match place_back(fs, aside.entry(), to, aside.identity, None) {
        Ok(_) => Ok(Renamed::CaseOnly),
        Err(error) => match place_back(fs, aside.entry(), from, aside.identity, None) {
            Ok(_) => Err(RenameFailure::Place(error)),
            Err(_) => {
                return Err(RenameFailure::Stranded {
                    temporary: aside.kept(),
                    error,
                });
            }
        },
    };
    let _ = aside.originals.remove(fs, from.dir, &Deadline::default());
    renamed
}

fn same_directory(fs: &dyn FsOps, left: BorrowedFd<'_>, right: BorrowedFd<'_>) -> bool {
    match (fs.stat_fd(left), fs.stat_fd(right)) {
        (Ok(left), Ok(right)) => left.identity == right.identity,
        _ => false,
    }
}

/// The item's identity when `to` names `from` itself rather than another
/// item: a case alias, never a separate hard link.
pub(super) fn alias_of(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
) -> Option<LocalFileIdentity> {
    let source = fs.stat(from, true).ok()?;
    let target = fs.stat(to, true).ok()?;
    let names = || fs.names(at(to.dir, OsStr::new(".")));
    if source.identity != target.identity {
        // libfuse without `use_ino` numbers each spelling as its own node.
        let same_item = (source.kind, source.size, source.mtime)
            == (target.kind, target.size, target.mtime)
            && same_directory(fs, from.dir, to.dir)
            && names().is_ok_and(|names| {
                let mut spellings = names
                    .iter()
                    .filter(|name| same_ignoring_case(name, to.name));
                spellings.next().is_some_and(|name| name == from.name) && spellings.next().is_none()
            });
        return same_item.then_some(source.identity);
    }
    let single_name = !source.is_directory() && source.nlink == 1;
    let alias = single_name
        || names().is_ok_and(|names| {
            !names.iter().any(|name| name == to.name)
                && names
                    .iter()
                    .filter(|name| same_ignoring_case(name, to.name))
                    .all(|name| name == from.name)
        });
    alias.then_some(source.identity)
}

fn same_ignoring_case(left: &OsStr, right: &OsStr) -> bool {
    match (left.to_str(), right.to_str()) {
        (Some(left), Some(right)) => left.to_lowercase() == right.to_lowercase(),
        _ => left.eq_ignore_ascii_case(right),
    }
}

fn rename_over_reserved_file(
    fs: &dyn FsOps,
    mount: &Mount<'_>,
    from: Entry<'_>,
    to: Entry<'_>,
    source: &EntryStat,
) -> Result<Placed, PlaceError> {
    let reserved = match fs.create_exclusive(to, Mode::from_raw_mode(0o600)) {
        Ok(file) => fs.stat_fd(file.as_fd()).map_err(PlaceError::Io)?.identity,
        Err(Errno::PERM) if exists(fs, to)? => return Err(PlaceError::Exists),
        Err(error) => return Err(file_creation_error(error)),
    };
    let still_reserved = || match fs.stat(to, true) {
        Ok(current) => {
            let empty_file = current.kind == FileType::RegularFile && current.size == 0;
            Ok(current.identity == reserved && empty_file)
        }
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(PlaceError::Io(error)),
    };
    if !still_reserved()? {
        return Err(PlaceError::Exists);
    }
    let error = match fs.rename(from, to, RenameFlags::empty()) {
        Ok(()) => return Ok(Placed::OverReservedFile),
        Err(error) => error,
    };
    if may_have_lost_reply(error) {
        if mount.numbers_kept()
            && still_reserved() == Ok(true)
            && fs.unlink(to, AtFlags::empty()).is_ok()
        {
            return Err(PlaceError::Io(error));
        }
        return moved_despite_lost_reply(fs, from, to, source, Placed::OverReservedFile);
    }
    let removed = still_reserved() == Ok(true) && fs.unlink(to, AtFlags::empty()).is_ok();
    Err(match error {
        Errno::INVAL => PlaceError::InvalidName,
        Errno::XDEV => PlaceError::CrossDevice,
        // SFTP without `posix-rename@openssh.com` refuses to rename over any file.
        Errno::PERM if removed => PlaceError::Unsupported,
        error => PlaceError::Io(error),
    })
}

/// Decides a rename whose reply may have been lost without removing anything:
/// libfuse without `use_ino` keeps a name's inode number then, so a
/// placeholder's number at `to` can belong to the moved item. Where numbers
/// are stable, a placeholder still in place proves the rename never happened,
/// and the rungs remove it before coming here.
fn moved_despite_lost_reply(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    source: &EntryStat,
    placed: Placed,
) -> Result<Placed, PlaceError> {
    if matches!(fs.stat(from, true), Err(Errno::NOENT))
        && fs
            .stat(to, true)
            .is_ok_and(|target| target.identity == source.identity)
    {
        Ok(placed)
    } else {
        Err(PlaceError::Unconfirmed)
    }
}

fn rename_folder_after_check(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    source: &EntryStat,
) -> Result<Placed, PlaceError> {
    if exists(fs, to)? {
        return Err(PlaceError::Exists);
    }
    match fs.rename(from, to, RenameFlags::empty()) {
        Ok(()) => Ok(Placed::AfterCheck),
        Err(Errno::EXIST | Errno::NOTEMPTY | Errno::NOTDIR | Errno::ISDIR) => {
            Err(PlaceError::Exists)
        }
        Err(Errno::PERM) if exists(fs, to)? => Err(PlaceError::Exists),
        Err(Errno::INVAL) => Err(if contains(fs, source.identity, to.dir) == Some(false) {
            PlaceError::InvalidName
        } else {
            PlaceError::IntoItself
        }),
        Err(Errno::XDEV) => Err(PlaceError::CrossDevice),
        Err(error) if may_have_lost_reply(error) => {
            moved_despite_lost_reply(fs, from, to, source, Placed::AfterCheck)
        }
        Err(error) => Err(PlaceError::Io(error)),
    }
}

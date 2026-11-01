// SPDX-License-Identifier: MIT

use super::*;

/// Looks the mount key up only when a fallback needs it, so the first rung
/// costs a single syscall.
pub(super) struct Mount<'a> {
    fs: &'a dyn FsOps,
    dir: BorrowedFd<'a>,
    key: Cell<Option<MountKey>>,
    probed_links: Cell<bool>,
}

impl<'a> Mount<'a> {
    pub(super) fn new(fs: &'a dyn FsOps, dir: BorrowedFd<'a>) -> Self {
        Self {
            fs,
            dir,
            key: Cell::new(None),
            probed_links: Cell::new(false),
        }
    }

    pub(super) fn key(&self) -> MountKey {
        if let Some(key) = self.key.get() {
            return key;
        }
        let key = self.fs.mount_key(self.dir);
        self.key.set(Some(key));
        key
    }

    pub(super) fn capabilities(&self) -> MountCapabilities {
        mount_capabilities(self.key())
    }

    /// Whether an inode number still found at a name proves the entry is
    /// the one it was read from, as the mount's type tells.
    pub(super) fn numbers_kept(&self) -> bool {
        mount_profile::identity_at(self.fs, self.dir).numbers == InodeNumbers::Kept
    }

    pub(super) fn learn(&self, learn: impl FnOnce(&mut MountCapabilities)) {
        learn_mount_capability(self.key(), learn);
    }

    pub(super) fn probe(&self, links: bool, folders: bool) -> Result<Probed, Errno> {
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
    pub(super) fn allows_reserved_name(&self, directory: bool) -> bool {
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

    pub(super) fn renames_over(&self, name: &OsStr) -> bool {
        units(name.as_encoded_bytes()) <= LONGEST_NAME_NTFS_3G_RENAMES_OVER
            || !mount_profile::known(self.fs, self.dir, self.key()).cuts_long_names_renamed_over
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::adapters::local_operations) struct Probed {
    pub no_replace: Option<bool>,
    pub hard_links: Option<bool>,
    pub rename_over_empty_directory: Option<bool>,
    pub links_keep_numbers: bool,
}

/// Tries each primitive on new names of Strata's own in `dir`, where no
/// errno can be blamed on the user's item. Fields stay `None` when the probe
/// cannot tell.
pub(in crate::adapters::local_operations) fn probe(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    renames: bool,
    links: bool,
    folders: bool,
) -> Result<Probed, Errno> {
    let mut probed = Probed::default();
    let (mut current, identity) = match create_probe_file(fs, dir) {
        Ok(created) => created,
        Err(PlaceError::Io(error)) => return Err(error),
        Err(_) => return Ok(probed),
    };
    if renames && let Ok((renamed, supported)) = probe_rename(fs, dir, &current) {
        probed.no_replace = supported;
        if supported == Some(true) {
            current = renamed;
        }
    }
    if links {
        probe_links(fs, (dir, &current), identity, &mut probed);
    }
    remove_probe_file(fs, at(dir, &current), identity);
    if folders {
        probed.rename_over_empty_directory = probe_folders(fs, dir);
    }
    Ok(probed)
}

fn create_probe_file(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
) -> Result<(OsString, LocalFileIdentity), PlaceError> {
    reserve_hidden_name(HiddenName::Probe, |name| {
        drop(
            fs.create_exclusive(at(dir, name), Mode::from_raw_mode(0o600))
                .map_err(file_creation_error)?,
        );
        fs.stat(at(dir, name), false)
            .map(|stat| stat.identity)
            .map_err(PlaceError::Io)
    })
}

fn probe_rename(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    current: &OsStr,
) -> Result<(OsString, Option<bool>), PlaceError> {
    reserve_hidden_name(HiddenName::Probe, |name| {
        match fs.rename(at(dir, current), at(dir, name), RenameFlags::NOREPLACE) {
            Ok(()) => Ok(Some(true)),
            Err(Errno::EXIST) => Err(PlaceError::Exists),
            Err(error) if error == Errno::INVAL || is_unsupported(error) => Ok(Some(false)),
            Err(_) => Ok(None),
        }
    })
}

fn probe_links(
    fs: &dyn FsOps,
    (dir, current): (BorrowedFd<'_>, &OsStr),
    identity: LocalFileIdentity,
    probed: &mut Probed,
) {
    let linked = reserve_hidden_name(HiddenName::Probe, |name| {
        match fs.link(at(dir, current), at(dir, name)) {
            Ok(()) => Ok(Some(true)),
            Err(Errno::EXIST) => Err(PlaceError::Exists),
            Err(error) if matches!(error, Errno::PERM | Errno::ACCESS) || is_unsupported(error) => {
                Ok(Some(false))
            }
            Err(_) => Ok(None),
        }
    });
    let Ok((linked, supported)) = linked else {
        return;
    };
    probed.hard_links = supported;
    if supported != Some(true) {
        return;
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::adapters::local_operations) enum Feasibility {
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

pub(in crate::adapters::local_operations) fn links_hold(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
) -> bool {
    let mount = Mount::new(fs, dir);
    if mount.capabilities().hard_links.is_none() {
        let _ = mount.probe(true, false);
    }
    mount.capabilities().hard_links == Some(true)
}

/// Whether a staged entry could be placed in `dir`; `None` until the mount
/// is known, without probing it.
pub(in crate::adapters::local_operations) fn can_place(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    directory: bool,
) -> Option<bool> {
    placeable(Mount::new(fs, dir).capabilities(), directory)
}

/// Whether an existing entry in `dir` can be moved aside without risking a
/// clobber, probing the mount when nothing is known yet.
pub(in crate::adapters::local_operations) fn can_set_aside(
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

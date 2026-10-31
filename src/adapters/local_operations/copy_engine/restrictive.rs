// SPDX-License-Identifier: MIT

//! Folders whose mode would lock their owner out of the copy. They are
//! recorded by path beneath the copy's root, so no descriptor is held for
//! each, and get their exact mode last.

use std::{
    cmp::Reverse,
    ffi::OsStr,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
};

use rustix::io::Errno;

use super::{Mount, best_effort, is_fatal};
use crate::{
    adapters::{
        local_operations::{
            fs_ops::{Entry, FsOps},
            native_fs::{LocalFileIdentity, compare},
        },
        volume::Family,
    },
    services::{IdentityTraits, Proof},
};

#[derive(Clone, Debug)]
pub(in crate::adapters::local_operations) struct Restrictive {
    pub(super) path: PathBuf,
    pub(super) identity: LocalFileIdentity,
    pub(super) mode: u32,
}

impl Restrictive {
    fn depth(&self) -> usize {
        self.path.components().count()
    }
}

pub(super) fn reopen(
    fs: &dyn FsOps,
    root: BorrowedFd<'_>,
    path: &Path,
    identity: LocalFileIdentity,
    traits: IdentityTraits,
) -> Result<Option<OwnedFd>, Errno> {
    let opened = fs
        .open_dir(Entry {
            dir: root,
            name: beneath(path),
        })
        .and_then(|fd| fs.stat_fd(fd.as_fd()).map(|stat| (fd, stat)));
    match opened {
        Ok((fd, current)) => {
            Ok((compare(traits, identity, None, &current) != Proof::Different).then_some(fd))
        }
        Err(error) if is_fatal(error) => Err(error),
        Err(_) => Ok(None),
    }
}

pub(super) fn beneath(path: &Path) -> &OsStr {
    if path.as_os_str().is_empty() {
        OsStr::new(".")
    } else {
        path.as_os_str()
    }
}

/// Gives each folder its exact mode, deepest first, so its ancestors stay
/// open to the owner until then.
pub(super) fn tighten(
    fs: &dyn FsOps,
    root: BorrowedFd<'_>,
    folders: &mut [Restrictive],
    traits: IdentityTraits,
) -> Result<(), Errno> {
    folders.sort_by_key(|folder| Reverse(folder.depth()));
    for folder in folders.iter() {
        if let Some(fd) = reopen(fs, root, &folder.path, folder.identity, traits)? {
            best_effort(fs.chmod_fd(fd.as_fd(), folder.mode))?;
        }
    }
    Ok(())
}

/// Gives the owner access back, shallowest first, so the copy can be removed.
pub(super) fn loosen(fs: &dyn FsOps, root: BorrowedFd<'_>, folders: &[Restrictive], mount: &Mount) {
    let mut folders = folders.iter().collect::<Vec<_>>();
    folders.sort_by_key(|folder| folder.depth());
    for folder in folders {
        let at = Entry {
            dir: root,
            name: beneath(&folder.path),
        };
        if let Ok(Some(fd)) = reopen(fs, root, &folder.path, folder.identity, mount.identity) {
            open_to_owner(fs, at, fd.as_fd(), folder.mode, mount.family);
        }
    }
}

pub(super) fn open_to_owner(
    fs: &dyn FsOps,
    at: Entry<'_>,
    fd: BorrowedFd<'_>,
    mode: u32,
    family: Family,
) -> bool {
    // Without POSIX extensions, the SMB client clears the read-only
    // attribute it set only once it has seen it set.
    if family == Family::Smb {
        let _ = fs.stat(at, true);
    }
    fs.chmod_fd(fd, mode | 0o700).is_ok()
}

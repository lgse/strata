// SPDX-License-Identifier: MIT

//! A source's owner, xattrs, mode and times, applied best effort in that
//! order: `chown` clears `security.capability` and the special bits, and
//! xattrs go in before a read-only mode would refuse them.

use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    os::fd::BorrowedFd,
    sync::Mutex,
};

use rustix::io::Errno;

use super::{Mount, best_effort, is_fatal, is_unsupported, lock};
use crate::adapters::{
    local_operations::fs_ops::{Entry, EntryStat, FsOps},
    volume::{Family, XattrNamespace},
};

/// Never copied: the skip entries of libattr's `xattr.conf`, and ntfs3's
/// names that would set owner and mode behind `fchown` and `fchmod`.
const SKIPPED_XATTRS: [&str; 9] = [
    "security.evm",
    "trusted.SGI_*",
    "xfsroot.*",
    "afs.*",
    "user.Beagle.*",
    "$LXUID",
    "$LXGID",
    "$LXMOD",
    "$LXDEV",
];

/// ACLs, the permissions entries of `xattr.conf`. They can take away the
/// owner's write access, which setting the other xattrs needs.
const ACL_XATTRS: [&str; 4] = [
    "system.nfs4_acl",
    "system.nfs4acl",
    "system.posix_acl_access",
    "system.posix_acl_default",
];

fn skipped(name: &OsStr) -> bool {
    let name = name.as_encoded_bytes();
    SKIPPED_XATTRS
        .iter()
        .any(|pattern| match pattern.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix.as_bytes()),
            None => name == pattern.as_bytes(),
        })
}

/// Within two seconds, the coarsest granularity a mount stores.
fn same_time(left: (i64, u32), right: (i64, u32)) -> bool {
    let nanoseconds = |(seconds, nanoseconds): (i64, u32)| {
        i128::from(seconds) * 1_000_000_000 + i128::from(nanoseconds)
    };
    (nanoseconds(left) - nanoseconds(right)).abs() < 2_000_000_000
}

pub(super) struct Snapshot {
    mode: u32,
    uid: u32,
    gid: u32,
    atime: (i64, u32),
    mtime: (i64, u32),
    xattrs: Vec<(OsString, Vec<u8>)>,
}

impl Snapshot {
    pub(super) fn of(stat: &EntryStat) -> Self {
        Self {
            mode: stat.mode,
            uid: stat.uid,
            gid: stat.gid,
            atime: stat.atime,
            mtime: stat.mtime,
            xattrs: Vec::new(),
        }
    }

    pub(super) fn read(
        fs: &dyn FsOps,
        fd: BorrowedFd<'_>,
        stat: &EntryStat,
    ) -> Result<Self, Errno> {
        let names = match fs.xattr_names(fd) {
            Ok(names) => names,
            Err(error) if is_fatal(error) => return Err(error),
            Err(_) => Vec::new(),
        };
        let mut xattrs = Vec::new();
        for name in names.into_iter().filter(|name| !skipped(name)) {
            match fs.get_xattr(fd, &name) {
                Ok(value) => xattrs.push((name, value)),
                Err(error) if is_fatal(error) => return Err(error),
                Err(_) => {}
            }
        }
        xattrs.sort_by_key(|(name, _)| ACL_XATTRS.iter().any(|acl| name == *acl));
        Ok(Self {
            xattrs,
            ..Self::of(stat)
        })
    }

    /// Inside a setgid folder, `created` keeps the folder's group.
    pub(super) fn apply(
        &self,
        fs: &dyn FsOps,
        fd: BorrowedFd<'_>,
        created: &EntryStat,
        parent: &EntryStat,
        mount: &Mount,
        refused_names: &Mutex<HashSet<OsString>>,
    ) -> Result<(), Errno> {
        let mode = self.owner_and_xattrs(fs, fd, created, parent, mount, refused_names)?;
        best_effort(fs.chmod_fd(fd, mode))?;
        // A write delegation resets them at the close, after which they are
        // set again anyway.
        if mount.family == Family::Nfs4 {
            return Ok(());
        }
        self.apply_times(fs, fd)
    }

    /// As [`Self::apply`] for a named pipe, whose times nothing written
    /// resets.
    pub(super) fn apply_to_pipe(
        &self,
        fs: &dyn FsOps,
        fd: BorrowedFd<'_>,
        created: &EntryStat,
        parent: &EntryStat,
        mount: &Mount,
        refused_names: &Mutex<HashSet<OsString>>,
    ) -> Result<(), Errno> {
        let mode = self.owner_and_xattrs(fs, fd, created, parent, mount, refused_names)?;
        best_effort(fs.chmod_fd(fd, mode))?;
        self.apply_times(fs, fd)
    }

    /// As [`Self::apply`], and a setgid parent hands its bit on. A mode that
    /// would lock the owner out is returned instead of set, and the owner
    /// keeps access until the copy is done.
    pub(super) fn apply_to_folder(
        &self,
        fs: &dyn FsOps,
        fd: BorrowedFd<'_>,
        created: &EntryStat,
        parent: &EntryStat,
        mount: &Mount,
        refused_names: &Mutex<HashSet<OsString>>,
    ) -> Result<Option<u32>, Errno> {
        let mut mode = self.owner_and_xattrs(fs, fd, created, parent, mount, refused_names)?;
        if parent.mode & 0o2000 != 0 {
            mode |= 0o2000;
        }
        let restrictive = mode & 0o700 != 0o700;
        best_effort(fs.chmod_fd(fd, if restrictive { mode | 0o700 } else { mode }))?;
        self.apply_times(fs, fd)?;
        Ok(restrictive.then_some(mode))
    }

    fn owner_and_xattrs(
        &self,
        fs: &dyn FsOps,
        fd: BorrowedFd<'_>,
        created: &EntryStat,
        parent: &EntryStat,
        mount: &Mount,
        refused_names: &Mutex<HashSet<OsString>>,
    ) -> Result<u32, Errno> {
        let shared_group = parent.mode & 0o2000 != 0;
        let other_group = self.gid != created.gid;
        let uid = (self.uid != created.uid).then_some(self.uid);
        let gid = (other_group && !shared_group).then_some(self.gid);
        let mut owner_kept = !(other_group && shared_group);
        if uid.is_some() || gid.is_some() {
            match fs.chown_fd(fd, uid, gid) {
                Ok(()) => {}
                Err(error) if is_fatal(error) => return Err(error),
                Err(_) => owner_kept = false,
            }
        }
        let mut refused = mount.capabilities().refused_xattrs;
        for (name, value) in &self.xattrs {
            let namespace = XattrNamespace::of(name);
            if refused.contains(namespace) || lock(refused_names).contains(name) {
                continue;
            }
            match fs.set_xattr(fd, name, value) {
                Ok(()) => {}
                Err(error) if is_fatal(error) => return Err(error),
                Err(error) if is_unsupported(error) => {
                    refused.insert(namespace);
                    mount.learn(|capabilities| capabilities.refused_xattrs.insert(namespace));
                }
                Err(Errno::PERM | Errno::ACCESS | Errno::INVAL) => {
                    lock(refused_names).insert(name.clone());
                }
                Err(_) => {}
            }
        }
        // Special bits would grant another owner's or group's rights.
        Ok(if owner_kept {
            self.mode
        } else {
            self.mode & !0o6000
        })
    }

    pub(super) fn apply_times(&self, fs: &dyn FsOps, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        best_effort(fs.set_times_fd(fd, self.atime, self.mtime))
    }

    pub(super) fn apply_times_at(&self, fs: &dyn FsOps, entry: Entry<'_>) -> Result<(), Errno> {
        best_effort(fs.set_link_times(entry, self.atime, self.mtime))
    }

    pub(super) fn times_match(&self, current: &EntryStat) -> bool {
        same_time(self.mtime, current.mtime) && same_time(self.atime, current.atime)
    }
}

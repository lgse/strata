// SPDX-License-Identifier: MIT

//! Puts what a copy wrote on its device before anything only the copy holds
//! is deleted, the way the destination's filesystem family reaches it.

use std::{
    ffi::OsStr,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use gtk::glib;
use rustix::{fs::Mode, io::Errno};

use super::{Mount, data::kernel, stage};
use crate::adapters::{
    local_operations::{
        fs_ops::{Entry, FsOps, OffMain},
        io_error,
        native_fs::{open_local_parent_directory, os_error_detail},
    },
    volume::{Family, MountKey},
};

#[derive(Clone, Copy, Default)]
struct Plan {
    each_file: bool,
    sync_fs: bool,
    sync_dir: bool,
}

impl Plan {
    fn of(mount: &Mount) -> Self {
        let mut plan = match mount.family {
            Family::Fat | Family::Ntfs3 => Self {
                sync_fs: true,
                sync_dir: true,
                ..Self::default()
            },
            // Without FUSE_SYNCFS, `syncfs` writes back the device's cache
            // but flushes nothing. ntfs-3g and exfat-fuse sync and flush their
            // whole device for a folder's `fsync`, which reaches every file.
            Family::Fuse if mount.fuse_format.is_some() => Self {
                sync_fs: true,
                sync_dir: true,
                ..Self::default()
            },
            // Other daemons are reached by a file's `fsync`, and by a
            // folder's for the names published after the files were synced.
            Family::Fuse => Self {
                each_file: true,
                sync_dir: true,
                ..Self::default()
            },
            // Only a file's `fsync` reaches the server, as an SMB FLUSH.
            Family::Smb => Self {
                each_file: true,
                ..Self::default()
            },
            Family::Other => Self {
                each_file: true,
                sync_fs: true,
                ..Self::default()
            },
            _ => Self {
                sync_fs: true,
                ..Self::default()
            },
        };
        // Before Linux 5.8, `syncfs` reports no writeback errors.
        if kernel() < (5, 8, 0) {
            plan.each_file = true;
        }
        plan
    }
}

pub(super) fn each_file(mount: &Mount) -> bool {
    Plan::of(mount).each_file
}

static OPENED: AtomicU64 = AtomicU64::new(0);

pub(in crate::adapters::local_operations) struct Barrier {
    dir: OffMain<OwnedFd>,
    pub path: PathBuf,
    pub key: MountKey,
    plan: Plan,
    opened: u64,
}

impl Barrier {
    /// `syncfs` reports only the writeback errors that follow the open of
    /// its descriptor, so this must come before the copy writes. A folder
    /// others may only write into cannot be opened for reading, but any
    /// folder above it on the same mount serves, and so does an unnamed file
    /// made in it where only the filesystem is flushed.
    pub(super) fn open(fs: &dyn FsOps, path: &Path) -> Result<Self, glib::Error> {
        let could_not_open = |error: Errno| {
            io_error(rust_i18n::t!(
                "Could not safely open %{path}: %{error}",
                path = path.display(),
                error = os_error_detail(error)
            ))
        };
        let mut first: Option<(MountKey, Mount, OwnedFd)> = None;
        for folder in path.ancestors() {
            let pinned = open_local_parent_directory(folder).map_err(io_error)?;
            let here = fs.mount_key(pinned.as_fd());
            let mount = match &first {
                Some((key, _, _)) if *key != here => break,
                Some((_, mount, _)) => *mount,
                None => Mount::new(fs, pinned.as_fd()),
            };
            let plan = Plan::of(&mount);
            // Only flushing a folder or its filesystem reads it.
            if !plan.sync_fs && !plan.sync_dir {
                return Ok(Self::opened(fs, pinned, path.to_path_buf()));
            }
            let opened = fs.open_dir(Entry {
                dir: pinned.as_fd(),
                name: OsStr::new("."),
            });
            match opened {
                Ok(dir) => return Ok(Self::opened(fs, dir, path.to_path_buf())),
                Err(Errno::ACCESS) => {}
                Err(error) => return Err(could_not_open(error)),
            }
            if first.is_none() {
                first = Some((here, mount, pinned));
            }
        }
        // Where no folder of the mount can be read, the destination still
        // takes an unnamed file, which `syncfs` accepts.
        if let Some((_, mount, pinned)) = &first
            && !Plan::of(mount).sync_dir
            && stage::tmpfile_allowed(mount.family)
            && let Ok(file) = fs.open_tmpfile(pinned.as_fd(), Mode::from_raw_mode(0o600))
        {
            return Ok(Self::opened(fs, file, path.to_path_buf()));
        }
        Err(could_not_open(Errno::ACCESS))
    }

    pub(super) fn at(fs: &dyn FsOps, dir: BorrowedFd<'_>, path: PathBuf) -> Result<Self, Errno> {
        let dir = fs.open_dir(Entry {
            dir,
            name: OsStr::new("."),
        })?;
        Ok(Self::opened(fs, dir, path))
    }

    fn opened(fs: &dyn FsOps, dir: OwnedFd, path: PathBuf) -> Self {
        let mount = Mount::new(fs, dir.as_fd());
        Self {
            dir: OffMain::new(dir),
            path,
            key: mount.key,
            plan: Plan::of(&mount),
            opened: OPENED.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub(in crate::adapters::local_operations) fn syncs_each_file(&self) -> bool {
        self.plan.each_file
    }

    pub(in crate::adapters::local_operations) fn flushes(&self) -> bool {
        self.plan.sync_fs || self.plan.sync_dir
    }

    /// Whether this was opened first, so it reports every error `other` would.
    pub(in crate::adapters::local_operations) fn precedes(&self, other: &Self) -> bool {
        self.opened < other.opened
    }

    pub(super) fn reach(&self, fs: &dyn FsOps, removable: bool) -> Result<(), Errno> {
        // FAT and ntfs3 report lost renames only from the folder's fsync, and their
        // sync_fs leaves the device cache unflushed, so the folder is synced around it.
        if self.plan.sync_dir && self.plan.sync_fs {
            fs.sync_dir(self.dir.as_fd())?;
        }
        if self.plan.sync_fs {
            fs.sync_fs(self.dir.as_fd())?;
        }
        if self.plan.sync_dir || removable {
            fs.sync_dir(self.dir.as_fd())?;
        }
        Ok(())
    }
}

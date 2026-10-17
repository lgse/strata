// SPDX-License-Identifier: MIT

use super::fs_ops::{Entry, EntryStat, assert_off_main_thread, fs};
use crate::services::{Attributes, Evidence, IdentityTraits, Proof, TrashedOriginal};
use std::{
    ffi::{OsStr, OsString},
    io,
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
};

/// Many filesystems hand a freed inode number to the next item created, so
/// the type and, where reported, the birth time tell the two apart.
#[derive(Clone, Copy, Debug)]
pub(super) struct LocalFileIdentity {
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) kind: rustix::fs::FileType,
    pub(super) birth: Option<(i64, u32)>,
}

impl PartialEq for LocalFileIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.kind == other.kind
            && match (self.birth, other.birth) {
                (Some(left), Some(right)) => left == right,
                _ => true,
            }
    }
}

impl LocalFileIdentity {
    pub(super) fn evidence(self, attributes: Option<Attributes>) -> Evidence {
        Evidence {
            device: self.device,
            inode: self.inode,
            kind: self.kind,
            birth: self.birth,
            attributes,
        }
    }

    pub(super) fn from_stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
            kind: rustix::fs::FileType::from_raw_mode(stat.st_mode),
            birth: None,
        }
    }
}

pub(super) fn compare(
    traits: IdentityTraits,
    recorded: LocalFileIdentity,
    attributes: Option<Attributes>,
    current: &EntryStat,
) -> Proof {
    let now = current.identity.evidence(Some(current.attributes()));
    recorded.evidence(attributes).compare(traits, &now)
}

impl From<LocalFileIdentity> for TrashedOriginal {
    fn from(identity: LocalFileIdentity) -> Self {
        Self {
            device: identity.device,
            inode: identity.inode,
        }
    }
}

pub(super) fn retry_local_open(
    mut open: impl FnMut() -> rustix::io::Result<OwnedFd>,
) -> rustix::io::Result<OwnedFd> {
    for _ in 0..15 {
        match open() {
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => {}
            result => return result,
        }
    }
    open()
}

pub(in crate::adapters) fn open_local_child_directory<Fd: AsFd>(
    parent: &Fd,
    name: &OsStr,
) -> Result<OwnedFd, String> {
    // Reject symlinks swapped in after inspection, including intermediate components.
    retry_local_open(|| {
        fs().open_at(
            Entry::new(parent, name),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            Some(
                rustix::fs::ResolveFlags::BENEATH
                    | rustix::fs::ResolveFlags::NO_SYMLINKS
                    | rustix::fs::ResolveFlags::NO_MAGICLINKS,
            ),
        )
    })
    .map_err(|error| {
        rust_i18n::t!(
            "%{name} changed while it was being read: %{error}",
            name = name.to_string_lossy(),
            error = os_error_detail(error)
        )
        .into_owned()
    })
}

pub(super) fn local_directory_children<Fd: AsFd>(handle: &Fd) -> Result<Vec<OsString>, String> {
    fs().list(handle.as_fd())
        .map(|entries| entries.into_iter().map(|(name, _)| name).collect())
        .map_err(os_error_text)
}

/// Resolve ordinary parent aliases from `/`; descendants keep their separate no-follow policy.
pub(super) fn open_local_parent_directory(parent_path: &Path) -> Result<OwnedFd, String> {
    if !parent_path.is_absolute() {
        return Err(crate::i18n::tr(
            "A local operation target must use an absolute path",
        ));
    }
    let root = fs()
        .open_path(
            OsStr::new("/"),
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| {
            rust_i18n::t!(
                "Could not open the filesystem root: %{error}",
                error = os_error_detail(error)
            )
            .into_owned()
        })?;
    let relative = parent_path
        .strip_prefix(Path::new("/"))
        .map_err(|_| crate::i18n::tr("A local operation target must use an absolute path"))?;
    if relative.as_os_str().is_empty() {
        return Ok(root);
    }
    retry_local_open(|| {
        fs().open_at(
            Entry::new(&root, relative.as_os_str()),
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            Some(rustix::fs::ResolveFlags::IN_ROOT | rustix::fs::ResolveFlags::NO_MAGICLINKS),
        )
    })
    .map_err(|error| {
        rust_i18n::t!(
            "Could not safely open %{path}: %{error}",
            path = parent_path.display(),
            error = os_error_detail(error)
        )
        .into_owned()
    })
}

pub(super) fn open_local_parent_beneath(
    parent_path: &Path,
    allowed_root: &Path,
) -> Result<OwnedFd, String> {
    if !parent_path.is_absolute() || !allowed_root.is_absolute() {
        return Err(crate::i18n::tr(
            "A restore destination must use an absolute path",
        ));
    }
    let root = fs()
        .open_path(
            allowed_root.as_os_str(),
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| {
            rust_i18n::t!(
                "Could not open the restore area: %{error}",
                error = os_error_detail(error)
            )
            .into_owned()
        })?;
    if parent_path == allowed_root {
        return Ok(root);
    }
    let relative = parent_path
        .strip_prefix(allowed_root)
        .map_err(|_| crate::i18n::tr("The restore destination is outside the trash volume"))?;
    retry_local_open(|| {
        fs().open_at(
            Entry::new(&root, relative.as_os_str()),
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            Some(
                rustix::fs::ResolveFlags::BENEATH
                    | rustix::fs::ResolveFlags::NO_MAGICLINKS
                    | rustix::fs::ResolveFlags::NO_XDEV,
            ),
        )
    })
    .map_err(|error| {
        rust_i18n::t!(
            "Could not safely open restore destination %{path}: %{error}",
            path = parent_path.display(),
            error = os_error_detail(error)
        )
        .into_owned()
    })
}

pub(super) fn opened_path(fd: BorrowedFd<'_>) -> io::Result<PathBuf> {
    assert_off_main_thread();
    std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))
}

pub(super) fn os_error_text(error: impl Into<io::Error>) -> String {
    crate::services::io_error_message(&error.into())
}

pub(super) fn os_error_detail(error: impl Into<io::Error>) -> String {
    crate::services::io_error_detail(&error.into())
}

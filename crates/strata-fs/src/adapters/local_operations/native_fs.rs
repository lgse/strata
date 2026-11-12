// SPDX-License-Identifier: MIT

use super::fs_ops::{Entry, EntryStat, FsOps, OffMain, assert_off_main_thread, fs};
use crate::services::{Attributes, Evidence, IdentityTraits, InodeNumbers, ItemIdentity, Proof};
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
    pub(super) handle: Option<u64>,
}

impl PartialEq for LocalFileIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.kind == other.kind
            && self
                .birth
                .zip(other.birth)
                .is_none_or(|(left, right)| left == right)
            && self
                .handle
                .zip(other.handle)
                .is_none_or(|(left, right)| left == right)
    }
}

impl LocalFileIdentity {
    pub(super) fn evidence(self, attributes: Option<Attributes>) -> Evidence {
        Evidence {
            device: self.device,
            inode: self.inode,
            kind: self.kind,
            birth: self.birth,
            handle: self.handle,
            attributes,
        }
    }

    pub(super) fn from_stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
            kind: rustix::fs::FileType::from_raw_mode(stat.st_mode),
            birth: None,
            handle: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Recorded {
    pub(super) identity: LocalFileIdentity,
    pub(super) attributes: Option<Attributes>,
}

impl Recorded {
    pub(super) fn of(stat: &EntryStat) -> Self {
        Self {
            identity: stat.identity,
            attributes: stat.kept_attributes(),
        }
    }
}

/// Without a birth time, only the exact inode proves an entry an operation made or removes.
pub(super) fn beside_birth(
    recorded: LocalFileIdentity,
    attributes: Option<Attributes>,
) -> Option<Attributes> {
    attributes.filter(|_| recorded.birth.is_some_and(|birth| birth != (0, 0)))
}

/// Once `serverino` is gone, cifs keeps a replaced entry's cached number and birth.
pub(super) fn removable(
    recorded: Recorded,
    current: &EntryStat,
    traits: impl FnOnce() -> IdentityTraits,
    folder_reset: bool,
) -> bool {
    let attributes = beside_birth(recorded.identity, recorded.attributes);
    let exact = current.identity == recorded.identity;
    if exact && attributes.is_none() {
        return true;
    }
    let traits = traits();
    (exact && !traits.made_up_births)
        || unchanged(traits, recorded.identity, attributes, current)
        || (folder_reset && birth_reset_folder(traits, recorded.identity, current))
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

pub(super) fn unchanged(
    traits: IdentityTraits,
    recorded: LocalFileIdentity,
    attributes: Option<Attributes>,
    current: &EntryStat,
) -> bool {
    match compare(traits, recorded, attributes, current) {
        Proof::Same => true,
        Proof::Different => false,
        Proof::Unproven => recorded == current.identity,
    }
}

#[derive(Default)]
pub(super) struct Pinned {
    pin: Option<OffMain<OwnedFd>>,
}

impl Pinned {
    pub(super) fn at(
        fs: &dyn FsOps,
        entry: Entry<'_>,
        stat: &EntryStat,
        traits: IdentityTraits,
    ) -> Self {
        let unproven = stat.identity.birth.is_none_or(|birth| birth == (0, 0))
            || (stat.is_directory() && !traits.folder_births);
        if traits.numbers == InodeNumbers::Renewed && unproven {
            Self::open(fs, entry, stat)
        } else {
            Self { pin: None }
        }
    }

    pub(super) fn open(fs: &dyn FsOps, entry: Entry<'_>, stat: &EntryStat) -> Self {
        match Self::hold(fs, entry) {
            Some((held, pin)) if held.identity == stat.identity => pin,
            _ => Self { pin: None },
        }
    }

    pub(super) fn hold(fs: &dyn FsOps, entry: Entry<'_>) -> Option<(EntryStat, Self)> {
        let pin = OffMain::new(
            fs.open_at(
                entry,
                rustix::fs::OFlags::PATH
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
                None,
            )
            .ok()?,
        );
        let held = fs.stat_fd(pin.as_fd()).ok()?;
        Some((held, Self { pin: Some(pin) }))
    }

    pub(super) fn location(&self) -> Option<PathBuf> {
        opened_path(self.pin.as_ref()?.as_fd()).ok()
    }

    pub(super) fn holds(&self, fs: &dyn FsOps, current: &EntryStat) -> Option<bool> {
        let pin = self.pin.as_ref()?;
        Some(
            fs.stat_fd(pin.as_fd())
                .is_ok_and(|pinned| pinned.identity == current.identity),
        )
    }

    pub(super) fn unchanged(
        &self,
        fs: &dyn FsOps,
        traits: IdentityTraits,
        recorded: &EntryStat,
        current: &EntryStat,
    ) -> bool {
        self.holds(fs, current) != Some(false)
            && unchanged(
                traits,
                recorded.identity,
                recorded.file_attributes(),
                current,
            )
    }
}

pub(super) fn birth_reset_folder(
    traits: IdentityTraits,
    recorded: LocalFileIdentity,
    current: &EntryStat,
) -> bool {
    let reported = |birth: Option<(i64, u32)>| birth.is_some_and(|birth| birth != (0, 0));
    !traits.folder_births
        && recorded.kind == rustix::fs::FileType::Directory
        && current.identity.kind == recorded.kind
        && current.identity.device == recorded.device
        && reported(recorded.birth)
        && reported(current.identity.birth)
}

pub(super) fn trashed_identity(stat: &EntryStat, traits: IdentityTraits) -> ItemIdentity {
    ItemIdentity {
        evidence: stat.identity.evidence(stat.kept_attributes()),
        traits,
    }
}

pub(super) fn identity_at_path(path: &Path) -> Option<ItemIdentity> {
    let (parent, name) = (path.parent()?, path.file_name()?);
    let parent = open_local_parent_directory(parent).ok()?;
    let stat = fs().stat(Entry::new(&parent, name), false).ok()?;
    Some(trashed_identity(
        &stat,
        super::mount_profile::identity_at(fs(), parent.as_fd()),
    ))
}

#[expect(unsafe_code, reason = "rustix has no name_to_handle_at")]
pub(super) fn file_handle(dir: BorrowedFd<'_>, name: &OsStr) -> Result<u64, rustix::io::Errno> {
    use rustix::io::Errno;
    use std::hash::{Hash, Hasher};
    const MAX_HANDLE_SZ: usize = 128;
    #[repr(C)]
    struct FileHandle {
        handle_bytes: libc::c_uint,
        handle_type: libc::c_int,
        f_handle: [u8; MAX_HANDLE_SZ],
    }
    let path = std::ffi::CString::new(name.as_encoded_bytes()).map_err(|_| Errno::INVAL)?;
    let flags = if name.is_empty() {
        libc::AT_EMPTY_PATH
    } else {
        0
    };
    let mut handle = FileHandle {
        handle_bytes: libc::c_uint::try_from(MAX_HANDLE_SZ).map_err(|_| Errno::INVAL)?,
        handle_type: 0,
        f_handle: [0; MAX_HANDLE_SZ],
    };
    let mut mount_id: libc::c_int = 0;
    // SAFETY: `handle` is a `struct file_handle` with room for the `handle_bytes` it states,
    // `path` is NUL-terminated, and every pointer outlives the call, which keeps none of them.
    let result = unsafe {
        libc::name_to_handle_at(
            dir.as_raw_fd(),
            path.as_ptr(),
            std::ptr::from_mut(&mut handle).cast::<libc::file_handle>(),
            &raw mut mount_id,
            flags,
        )
    };
    if result != 0 {
        return Err(Errno::from_io_error(&io::Error::last_os_error()).unwrap_or(Errno::IO));
    }
    let bytes = usize::try_from(handle.handle_bytes)
        .ok()
        .and_then(|length| handle.f_handle.get(..length))
        .ok_or(Errno::OVERFLOW)?;
    let mut digest = std::hash::DefaultHasher::new();
    handle.handle_type.hash(&mut digest);
    bytes.hash(&mut digest);
    Ok(digest.finish())
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

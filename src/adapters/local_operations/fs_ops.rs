// SPDX-License-Identifier: MIT

//! The directory-relative syscalls that placement uses, behind one seam so
//! tests can emulate filesystems that lack a primitive.

use std::{
    ffi::{OsStr, OsString},
    mem::MaybeUninit,
    ops::ControlFlow,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, IntoRawFd, OwnedFd},
        unix::ffi::OsStringExt,
    },
    path::Path,
    time::Duration,
};

use gtk::gio;

use rustix::{
    fs::{
        Access, AtFlags, FileType, FsWord, Mode, OFlags, RenameFlags, ResolveFlags, SeekFrom,
        StatxFlags,
    },
    io::Errno,
};

use super::native_fs::{LocalFileIdentity, retry_local_open};
use crate::{
    adapters::volume::MountKey,
    services::{Attributes, IdentityTraits},
};

/// `STATX_MNT_ID_UNIQUE` (Linux 6.8), which rustix does not name yet.
const STATX_MNT_ID_UNIQUE: u32 = 0x4000;

const LISTING_CHUNK_BYTES: usize = 32 * 1024;

pub(in crate::adapters) type Listed = Vec<(OsString, FileType)>;

#[derive(Clone, Copy, Debug)]
pub(in crate::adapters) struct FolderStamp {
    pub kind: FileType,
    pub mode: u32,
    pub size: u64,
    pub modified: (i64, u32),
    pub changed: (i64, u32),
    pub device: u64,
    pub inode: u64,
    pub birth: Option<(i64, u32)>,
    pub mount_id: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FsStat {
    pub kind: FsWord,
    /// `f_namelen`, which only byte-counting filesystems mean.
    pub name_max: u32,
}

#[derive(Clone, Copy)]
pub(super) struct Entry<'a> {
    pub dir: BorrowedFd<'a>,
    pub name: &'a OsStr,
}

impl<'a> Entry<'a> {
    pub(super) fn new(dir: &'a impl AsFd, name: &'a OsStr) -> Self {
        Self {
            dir: dir.as_fd(),
            name,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct EntryStat {
    pub identity: LocalFileIdentity,
    pub kind: FileType,
    /// Permission bits only.
    pub mode: u32,
    pub nlink: u64,
    pub size: u64,
    /// 512-byte blocks allocated.
    pub blocks: u64,
    pub atime: (i64, u32),
    pub mtime: (i64, u32),
    pub ctime: (i64, u32),
    pub uid: u32,
    pub gid: u32,
}

impl EntryStat {
    pub(super) fn is_directory(&self) -> bool {
        self.kind == FileType::Directory
    }

    pub(super) fn attributes(&self) -> Attributes {
        Attributes {
            size: self.size,
            mtime: self.mtime,
            ctime: Some(self.ctime),
        }
    }

    pub(super) fn file_attributes(&self) -> Option<Attributes> {
        matches!(self.kind, FileType::RegularFile | FileType::Symlink).then(|| self.attributes())
    }

    pub(super) fn kept_attributes(&self) -> Option<Attributes> {
        self.file_attributes().map(|attributes| Attributes {
            ctime: None,
            ..attributes
        })
    }
}

pub(super) trait FsOps: super::environment::FsEnvironment {
    // Not asserted off the main thread: restore opens descriptors before dispatching to a worker.
    fn open_at(
        &self,
        at: Entry<'_>,
        flags: OFlags,
        mode: Mode,
        resolve: Option<ResolveFlags>,
    ) -> Result<OwnedFd, Errno>;
    fn open_path(&self, path: &OsStr, flags: OFlags, mode: Mode) -> Result<OwnedFd, Errno>;
    fn dup(&self, fd: BorrowedFd<'_>) -> Result<OwnedFd, Errno>;
    fn stat_vfs(&self, fd: BorrowedFd<'_>) -> Result<rustix::fs::StatVfs, Errno>;
    fn rename(&self, from: Entry<'_>, to: Entry<'_>, flags: RenameFlags) -> Result<(), Errno>;
    fn link(&self, from: Entry<'_>, to: Entry<'_>) -> Result<(), Errno>;
    fn unlink(&self, at: Entry<'_>, flags: AtFlags) -> Result<(), Errno>;
    fn mkdir(&self, at: Entry<'_>, mode: Mode) -> Result<(), Errno>;
    fn symlink(&self, target: &OsStr, at: Entry<'_>) -> Result<(), Errno>;
    fn readlink(&self, at: Entry<'_>) -> Result<OsString, Errno>;
    /// Never follows `at`; `force_sync` asks a network filesystem to revalidate.
    fn stat(&self, at: Entry<'_>, force_sync: bool) -> Result<EntryStat, Errno>;
    /// The open file or folder itself, from cached attributes.
    fn stat_fd(&self, fd: BorrowedFd<'_>) -> Result<EntryStat, Errno>;
    fn open_parent(&self, dir: BorrowedFd<'_>) -> Result<OwnedFd, Errno>;
    fn create_exclusive(&self, at: Entry<'_>, mode: Mode) -> Result<OwnedFd, Errno>;
    fn names(&self, dir: Entry<'_>) -> Result<Vec<OsString>, Errno>;
    fn mount_key(&self, dir: BorrowedFd<'_>) -> MountKey;
    fn statfs(&self, dir: BorrowedFd<'_>) -> Option<FsStat>;
    /// The regular file at `at`, opened for reading without following it,
    /// or `None` when something else is there. `listed` skips looking at it
    /// by name first, where the caller just saw a regular file there.
    fn open_source(
        &self,
        at: Entry<'_>,
        listed: bool,
    ) -> Result<Option<(OwnedFd, EntryStat)>, Errno>;
    /// An unnamed file in `dir`, readable so a failed link can copy it out.
    fn open_tmpfile(&self, dir: BorrowedFd<'_>, mode: Mode) -> Result<OwnedFd, Errno>;
    /// Gives an unnamed file a name, never replacing one; `through_proc`
    /// links `/proc/self/fd/N` where `AT_EMPTY_PATH` needs privileges.
    fn link_fd(&self, fd: BorrowedFd<'_>, to: Entry<'_>, through_proc: bool) -> Result<(), Errno>;
    fn reflink(&self, target: BorrowedFd<'_>, source: BorrowedFd<'_>) -> Result<(), Errno>;
    fn copy_range(
        &self,
        source: BorrowedFd<'_>,
        source_offset: u64,
        target: BorrowedFd<'_>,
        target_offset: u64,
        len: usize,
    ) -> Result<usize, Errno>;
    fn read_at(&self, fd: BorrowedFd<'_>, buffer: &mut [u8], offset: u64) -> Result<usize, Errno>;
    fn write_at(&self, fd: BorrowedFd<'_>, buffer: &[u8], offset: u64) -> Result<usize, Errno>;
    fn set_len(&self, fd: BorrowedFd<'_>, len: u64) -> Result<(), Errno>;
    fn seek_data(&self, fd: BorrowedFd<'_>, offset: u64) -> Result<u64, Errno>;
    /// The end of the file counts as a hole.
    fn seek_hole(&self, fd: BorrowedFd<'_>, offset: u64) -> Result<u64, Errno>;
    fn sync_data(&self, fd: BorrowedFd<'_>) -> Result<(), Errno>;
    /// `syncfs`, which reports the writeback errors since `fd` was opened.
    fn sync_fs(&self, fd: BorrowedFd<'_>) -> Result<(), Errno>;
    fn sync_dir(&self, fd: BorrowedFd<'_>) -> Result<(), Errno>;
    /// Reports what `close` returns, where NFS, SMB and FUSE fail late writes.
    fn close(&self, fd: OwnedFd) -> Result<(), Errno>;
    fn chown_fd(&self, fd: BorrowedFd<'_>, uid: Option<u32>, gid: Option<u32>)
    -> Result<(), Errno>;
    fn chmod_fd(&self, fd: BorrowedFd<'_>, mode: u32) -> Result<(), Errno>;
    fn set_times_fd(
        &self,
        fd: BorrowedFd<'_>,
        atime: (i64, u32),
        mtime: (i64, u32),
    ) -> Result<(), Errno>;
    fn xattr_names(&self, fd: BorrowedFd<'_>) -> Result<Vec<OsString>, Errno>;
    fn get_xattr(&self, fd: BorrowedFd<'_>, name: &OsStr) -> Result<Vec<u8>, Errno>;
    fn set_xattr(&self, fd: BorrowedFd<'_>, name: &OsStr, value: &[u8]) -> Result<(), Errno>;
    /// The folder at `at`, which may name several components, opened
    /// without following a symlink anywhere on the way.
    fn open_dir(&self, at: Entry<'_>) -> Result<OwnedFd, Errno>;
    fn list(&self, dir: BorrowedFd<'_>) -> Result<Vec<(OsString, FileType)>, Errno>;
    fn list_chunks(
        &self,
        dir: BorrowedFd<'_>,
        each: &mut dyn FnMut(Listed) -> ControlFlow<()>,
    ) -> Result<(), Errno>;
    fn folder_stamp(&self, path: &Path, force_sync: bool) -> Result<FolderStamp, Errno>;
    fn access(&self, path: &Path, access: Access) -> Result<(), Errno>;
    fn create_node(&self, at: Entry<'_>, kind: FileType, mode: Mode) -> Result<(), Errno>;
    /// Opens the named pipe at `at` for its metadata, which never waits for
    /// a writer.
    fn open_fifo(&self, at: Entry<'_>) -> Result<OwnedFd, Errno>;
    /// Creates a file exclusively without opening it for writing, so its
    /// close flushes nothing (vfat mounted with `flush` waits there).
    fn create_readonly(&self, at: Entry<'_>, mode: Mode) -> Result<OwnedFd, Errno>;
    fn open_write(&self, at: Entry<'_>) -> Result<OwnedFd, Errno>;
    fn file_flags(&self, fd: BorrowedFd<'_>) -> Result<u32, Errno>;
    fn set_link_times(
        &self,
        at: Entry<'_>,
        atime: (i64, u32),
        mtime: (i64, u32),
    ) -> Result<(), Errno>;
}

pub(super) struct System;

pub(super) fn fs() -> &'static dyn FsOps {
    #[cfg(test)]
    {
        &super::tests::fs_quirks::SCOPED
    }
    #[cfg(not(test))]
    {
        &System
    }
}

pub(in crate::adapters) fn folder_stamp(
    path: &Path,
    force_sync: bool,
) -> Result<FolderStamp, Errno> {
    fs().folder_stamp(path, force_sync)
}

pub(in crate::adapters) struct StampMount {
    pub id: u64,
    pub identity: IdentityTraits,
    pub outside_writers: bool,
    pub coarse_window: Option<Duration>,
    pub daemon_cache: Option<Duration>,
}

pub(in crate::adapters) fn stamp_mount(path: &Path, stamp: &FolderStamp) -> Option<StampMount> {
    let (id, mount_type) = fs().folder_mount(path, stamp.mount_id)?;
    Some(StampMount {
        id,
        identity: mount_type.identity(),
        outside_writers: mount_type.outside_writers(),
        coarse_window: mount_type.coarse_times(None).map(|coarse| coarse.window),
        daemon_cache: mount_type.daemon_cache(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Watched {
    Folder,
    File,
}

pub(in crate::adapters) fn watch(
    file: &gio::File,
    watched: Watched,
    flags: gio::FileMonitorFlags,
) -> Result<gio::FileMonitor, gtk::glib::Error> {
    fs().watch(file, watched, flags)
}

pub(in crate::adapters) fn unwatch(file: &gio::File, monitor: gio::FileMonitor) {
    fs().unwatch(file, monitor);
}

pub(in crate::adapters) fn list_folder(
    path: &Path,
    each: &mut dyn FnMut(Listed) -> ControlFlow<()>,
) -> Result<(), Errno> {
    let folder = fs().open_path(
        path.as_os_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    fs().list_chunks(folder.as_fd(), each)
}

pub(super) fn assert_off_main_thread() {
    crate::util::ui_thread::assert_off();
}

pub(super) fn drop_off_main_thread<T: Send + 'static>(held: T) {
    if crate::util::ui_thread::is_current() {
        drop(gio::spawn_blocking(move || drop(held)));
    }
}

pub(super) struct OffMain<T: Send + 'static>(Option<T>);

impl<T: Send + 'static> OffMain<T> {
    pub(super) fn new(held: T) -> Self {
        Self(Some(held))
    }

    pub(super) fn into_inner(mut self) -> T {
        self.0.take().expect("held until dropped")
    }
}

impl<T: Send + 'static> std::ops::Deref for OffMain<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.as_ref().expect("held until dropped")
    }
}

impl<T: Send + 'static> Drop for OffMain<T> {
    fn drop(&mut self) {
        if let Some(held) = self.0.take() {
            drop_off_main_thread(held);
        }
    }
}

fn entry_stat(stat: &rustix::fs::Statx) -> EntryStat {
    let kind = FileType::from_raw_mode(u32::from(stat.stx_mode));
    let birth = StatxFlags::from_bits_retain(stat.stx_mask)
        .contains(StatxFlags::BTIME)
        .then_some((stat.stx_btime.tv_sec, stat.stx_btime.tv_nsec));
    EntryStat {
        identity: LocalFileIdentity {
            device: rustix::fs::makedev(stat.stx_dev_major, stat.stx_dev_minor),
            inode: stat.stx_ino,
            kind,
            birth,
            handle: None,
        },
        kind,
        mode: u32::from(stat.stx_mode) & 0o7777,
        nlink: u64::from(stat.stx_nlink),
        size: stat.stx_size,
        blocks: stat.stx_blocks,
        atime: (stat.stx_atime.tv_sec, stat.stx_atime.tv_nsec),
        mtime: (stat.stx_mtime.tv_sec, stat.stx_mtime.tv_nsec),
        ctime: (stat.stx_ctime.tv_sec, stat.stx_ctime.tv_nsec),
        uid: stat.stx_uid,
        gid: stat.stx_gid,
    }
}

/// `st_nlink` is `u64` on x86_64 but `u32` on aarch64; widening through a
/// generic keeps the conversion valid on both without a same-type cast.
fn link_count<N: Into<u64>>(nlink: N) -> u64 {
    nlink.into()
}

fn legacy_stat(stat: &rustix::fs::Stat) -> EntryStat {
    EntryStat {
        identity: LocalFileIdentity::from_stat(stat),
        kind: FileType::from_raw_mode(stat.st_mode),
        mode: stat.st_mode & 0o7777,
        nlink: link_count(stat.st_nlink),
        size: u64::try_from(stat.st_size).unwrap_or(u64::MAX),
        blocks: u64::try_from(stat.st_blocks).unwrap_or_default(),
        atime: (
            stat.st_atime,
            u32::try_from(stat.st_atime_nsec).unwrap_or_default(),
        ),
        mtime: (
            stat.st_mtime,
            u32::try_from(stat.st_mtime_nsec).unwrap_or_default(),
        ),
        ctime: (
            stat.st_ctime,
            u32::try_from(stat.st_ctime_nsec).unwrap_or_default(),
        ),
        uid: stat.st_uid,
        gid: stat.st_gid,
    }
}

fn statx_at(dir: BorrowedFd<'_>, name: &OsStr, flags: AtFlags) -> Result<EntryStat, Errno> {
    let mask = StatxFlags::TYPE
        | StatxFlags::MODE
        | StatxFlags::INO
        | StatxFlags::NLINK
        | StatxFlags::SIZE
        | StatxFlags::BLOCKS
        | StatxFlags::ATIME
        | StatxFlags::MTIME
        | StatxFlags::CTIME
        | StatxFlags::BTIME
        | StatxFlags::UID
        | StatxFlags::GID
        | StatxFlags::MNT_ID;
    match rustix::fs::statx(dir, name, flags | AtFlags::SYMLINK_NOFOLLOW, mask) {
        Ok(stat) => {
            let mut entry = entry_stat(&stat);
            let mount_id = StatxFlags::from_bits_retain(stat.stx_mask)
                .contains(StatxFlags::MNT_ID)
                .then_some(stat.stx_mnt_id);
            let (mounts, birth) = fs().file_handle_inputs(Entry { dir, name }, &entry);
            if mounts.wants_file_handle(mount_id, birth) {
                entry.identity.handle = fs().file_handle(Entry { dir, name }).ok();
            }
            Ok(entry)
        }
        Err(Errno::NOSYS) => {
            let at_flags = (flags & AtFlags::EMPTY_PATH) | AtFlags::SYMLINK_NOFOLLOW;
            rustix::fs::statat(dir, name, at_flags).map(|stat| legacy_stat(&stat))
        }
        Err(error) => Err(error),
    }
}

fn timespec((seconds, nanoseconds): (i64, u32)) -> rustix::fs::Timespec {
    rustix::fs::Timespec {
        tv_sec: seconds,
        tv_nsec: nanoseconds.into(),
    }
}

fn read_sized(mut read: impl FnMut(&mut [u8]) -> Result<usize, Errno>) -> Result<Vec<u8>, Errno> {
    let mut attempts = 0;
    loop {
        let size = read(&mut [])?;
        if size == 0 {
            return Ok(Vec::new());
        }
        let mut buffer = vec![0; size];
        match read(&mut buffer) {
            Ok(length) => {
                buffer.truncate(length);
                return Ok(buffer);
            }
            Err(Errno::RANGE) if attempts < 3 => attempts += 1,
            Err(error) => return Err(error),
        }
    }
}

impl FsOps for System {
    fn open_at(
        &self,
        at: Entry<'_>,
        flags: OFlags,
        mode: Mode,
        resolve: Option<ResolveFlags>,
    ) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match resolve {
            Some(resolve) => rustix::fs::openat2(at.dir, at.name, flags, mode, resolve),
            None => rustix::fs::openat(at.dir, at.name, flags, mode),
        }
    }

    fn open_path(&self, path: &OsStr, flags: OFlags, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        rustix::fs::open(path, flags, mode)
    }

    fn dup(&self, fd: BorrowedFd<'_>) -> Result<OwnedFd, Errno> {
        rustix::io::dup(fd)
    }

    fn stat_vfs(&self, fd: BorrowedFd<'_>) -> Result<rustix::fs::StatVfs, Errno> {
        assert_off_main_thread();
        rustix::fs::fstatvfs(fd)
    }
    fn rename(&self, from: Entry<'_>, to: Entry<'_>, flags: RenameFlags) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::renameat_with(from.dir, from.name, to.dir, to.name, flags)
    }

    fn link(&self, from: Entry<'_>, to: Entry<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::linkat(from.dir, from.name, to.dir, to.name, AtFlags::empty())
    }

    fn unlink(&self, at: Entry<'_>, flags: AtFlags) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::unlinkat(at.dir, at.name, flags)
    }

    fn mkdir(&self, at: Entry<'_>, mode: Mode) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::mkdirat(at.dir, at.name, mode)
    }

    fn symlink(&self, target: &OsStr, at: Entry<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::symlinkat(target, at.dir, at.name)
    }

    fn readlink(&self, at: Entry<'_>) -> Result<OsString, Errno> {
        assert_off_main_thread();
        rustix::fs::readlinkat(at.dir, at.name, Vec::new())
            .map(|link| OsString::from_vec(link.into_bytes()))
    }

    fn stat(&self, at: Entry<'_>, force_sync: bool) -> Result<EntryStat, Errno> {
        assert_off_main_thread();
        let flags = if force_sync {
            AtFlags::STATX_FORCE_SYNC
        } else {
            AtFlags::empty()
        };
        statx_at(at.dir, at.name, flags)
    }

    fn stat_fd(&self, fd: BorrowedFd<'_>) -> Result<EntryStat, Errno> {
        assert_off_main_thread();
        statx_at(
            fd,
            OsStr::new(""),
            AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC,
        )
    }

    fn open_parent(&self, dir: BorrowedFd<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        rustix::fs::openat(
            dir,
            "..",
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
    }

    fn create_exclusive(&self, at: Entry<'_>, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        rustix::fs::openat(
            at.dir,
            at.name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            mode,
        )
    }

    fn names(&self, dir: Entry<'_>) -> Result<Vec<OsString>, Errno> {
        assert_off_main_thread();
        let handle = rustix::fs::openat(
            dir.dir,
            dir.name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mut names = Vec::new();
        for entry in rustix::fs::Dir::read_from(&handle)? {
            let entry = entry?;
            let name = entry.file_name();
            if name != c"." && name != c".." {
                names.push(OsString::from_vec(name.to_bytes().to_vec()));
            }
        }
        Ok(names)
    }

    fn mount_key(&self, dir: BorrowedFd<'_>) -> MountKey {
        assert_off_main_thread();
        let mask = StatxFlags::from_bits_retain(STATX_MNT_ID_UNIQUE) | StatxFlags::MNT_ID;
        match rustix::fs::statx(
            dir,
            "",
            AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC,
            mask,
        ) {
            Ok(stat) => {
                let device = rustix::fs::makedev(stat.stx_dev_major, stat.stx_dev_minor);
                if stat.stx_mask & STATX_MNT_ID_UNIQUE != 0 {
                    MountKey::Unique(stat.stx_mnt_id)
                } else if StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::MNT_ID) {
                    MountKey::Mount {
                        id: stat.stx_mnt_id,
                        device,
                    }
                } else {
                    MountKey::Device(device)
                }
            }
            Err(_) => rustix::fs::fstat(dir).map_or(MountKey::Device(u64::MAX), |stat| {
                MountKey::Device(stat.st_dev)
            }),
        }
    }

    fn statfs(&self, dir: BorrowedFd<'_>) -> Option<FsStat> {
        assert_off_main_thread();
        rustix::fs::fstatfs(dir).ok().map(|stat| FsStat {
            kind: stat.f_type,
            name_max: u32::try_from(stat.f_namelen).unwrap_or(0),
        })
    }

    fn open_source(
        &self,
        at: Entry<'_>,
        listed: bool,
    ) -> Result<Option<(OwnedFd, EntryStat)>, Errno> {
        assert_off_main_thread();
        if !listed && statx_at(at.dir, at.name, AtFlags::empty())?.kind != FileType::RegularFile {
            return Ok(None);
        }
        // NONBLOCK, so an entry swapped for a FIFO meanwhile cannot block the open.
        let fd = retry_local_open(|| {
            rustix::fs::openat2(
                at.dir,
                at.name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
            )
        })?;
        let stat = statx_at(fd.as_fd(), OsStr::new(""), AtFlags::EMPTY_PATH)?;
        if stat.kind != FileType::RegularFile {
            return Ok(None);
        }
        // Some FUSE daemons pass the flag on to their reads.
        rustix::fs::fcntl_setfl(&fd, OFlags::empty())?;
        Ok(Some((fd, stat)))
    }

    fn open_tmpfile(&self, dir: BorrowedFd<'_>, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        rustix::fs::openat(
            dir,
            ".",
            OFlags::TMPFILE | OFlags::RDWR | OFlags::CLOEXEC,
            mode,
        )
    }

    fn link_fd(&self, fd: BorrowedFd<'_>, to: Entry<'_>, through_proc: bool) -> Result<(), Errno> {
        assert_off_main_thread();
        if through_proc {
            rustix::fs::linkat(
                rustix::fs::CWD,
                format!("/proc/self/fd/{}", fd.as_raw_fd()),
                to.dir,
                to.name,
                AtFlags::SYMLINK_FOLLOW,
            )
        } else {
            rustix::fs::linkat(fd, "", to.dir, to.name, AtFlags::EMPTY_PATH)
        }
    }

    fn reflink(&self, target: BorrowedFd<'_>, source: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::ioctl_ficlone(target, source)
    }

    fn copy_range(
        &self,
        source: BorrowedFd<'_>,
        mut source_offset: u64,
        target: BorrowedFd<'_>,
        mut target_offset: u64,
        len: usize,
    ) -> Result<usize, Errno> {
        assert_off_main_thread();
        rustix::fs::copy_file_range(
            source,
            Some(&mut source_offset),
            target,
            Some(&mut target_offset),
            len,
        )
    }

    fn read_at(&self, fd: BorrowedFd<'_>, buffer: &mut [u8], offset: u64) -> Result<usize, Errno> {
        assert_off_main_thread();
        rustix::io::pread(fd, buffer, offset)
    }

    fn write_at(&self, fd: BorrowedFd<'_>, buffer: &[u8], offset: u64) -> Result<usize, Errno> {
        assert_off_main_thread();
        rustix::io::pwrite(fd, buffer, offset)
    }

    fn set_len(&self, fd: BorrowedFd<'_>, len: u64) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::ftruncate(fd, len)
    }

    fn seek_data(&self, fd: BorrowedFd<'_>, offset: u64) -> Result<u64, Errno> {
        assert_off_main_thread();
        rustix::fs::seek(fd, SeekFrom::Data(offset))
    }

    fn seek_hole(&self, fd: BorrowedFd<'_>, offset: u64) -> Result<u64, Errno> {
        assert_off_main_thread();
        rustix::fs::seek(fd, SeekFrom::Hole(offset))
    }

    fn sync_data(&self, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::fdatasync(fd)
    }

    fn sync_fs(&self, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::syncfs(fd)
    }

    fn sync_dir(&self, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::fsync(fd)
    }

    #[expect(
        unsafe_code,
        reason = "OwnedFd and rustix discard close errors, which report failed network writes"
    )]
    fn close(&self, fd: OwnedFd) -> Result<(), Errno> {
        assert_off_main_thread();
        let raw = fd.into_raw_fd();
        // SAFETY: `raw` came from an owned descriptor and is closed exactly once.
        if unsafe { libc::close(raw) } == 0 {
            return Ok(());
        }
        match Errno::from_io_error(&std::io::Error::last_os_error()) {
            // Linux always releases the descriptor, as `g_close` assumes.
            Some(Errno::INTR) | None => Ok(()),
            Some(error) => Err(error),
        }
    }

    fn chown_fd(
        &self,
        fd: BorrowedFd<'_>,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        // -1 means "unchanged" to fchown, and is never a real id.
        let uid = uid
            .filter(|uid| *uid != u32::MAX)
            .map(rustix::process::Uid::from_raw);
        let gid = gid
            .filter(|gid| *gid != u32::MAX)
            .map(rustix::process::Gid::from_raw);
        rustix::fs::fchown(fd, uid, gid)
    }

    fn chmod_fd(&self, fd: BorrowedFd<'_>, mode: u32) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::fchmod(fd, Mode::from_raw_mode(mode))
    }

    fn set_times_fd(
        &self,
        fd: BorrowedFd<'_>,
        atime: (i64, u32),
        mtime: (i64, u32),
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::futimens(
            fd,
            &rustix::fs::Timestamps {
                last_access: timespec(atime),
                last_modification: timespec(mtime),
            },
        )
    }

    fn xattr_names(&self, fd: BorrowedFd<'_>) -> Result<Vec<OsString>, Errno> {
        assert_off_main_thread();
        let list = read_sized(|buffer| rustix::fs::flistxattr(fd, buffer))?;
        Ok(list
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            .map(|name| OsString::from_vec(name.to_vec()))
            .collect())
    }

    fn get_xattr(&self, fd: BorrowedFd<'_>, name: &OsStr) -> Result<Vec<u8>, Errno> {
        assert_off_main_thread();
        read_sized(|buffer| rustix::fs::fgetxattr(fd, name, buffer))
    }

    fn set_xattr(&self, fd: BorrowedFd<'_>, name: &OsStr, value: &[u8]) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::fsetxattr(fd, name, value, rustix::fs::XattrFlags::empty())
    }

    fn open_dir(&self, at: Entry<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        retry_local_open(|| {
            rustix::fs::openat2(
                at.dir,
                at.name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
            )
        })
    }

    fn list(&self, dir: BorrowedFd<'_>) -> Result<Vec<(OsString, FileType)>, Errno> {
        assert_off_main_thread();
        let mut entries = Vec::new();
        for entry in rustix::fs::Dir::read_from(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if name != c"." && name != c".." {
                entries.push((
                    OsString::from_vec(name.to_bytes().to_vec()),
                    entry.file_type(),
                ));
            }
        }
        Ok(entries)
    }

    fn list_chunks(
        &self,
        dir: BorrowedFd<'_>,
        each: &mut dyn FnMut(Listed) -> ControlFlow<()>,
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        let mut buffer = vec![MaybeUninit::uninit(); LISTING_CHUNK_BYTES];
        let mut entries = rustix::fs::RawDir::new(dir, &mut buffer);
        let mut chunk = Vec::new();
        while let Some(entry) = entries.next() {
            let entry = entry?;
            let name = entry.file_name();
            if name != c"." && name != c".." {
                let kind = match entry.file_type() {
                    FileType::Unknown => {
                        match rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
                            Ok(stat) => Some(FileType::from_raw_mode(stat.st_mode)),
                            Err(Errno::NOENT) => None,
                            Err(error) => return Err(error),
                        }
                    }
                    kind => Some(kind),
                };
                if let Some(kind) = kind {
                    chunk.push((OsString::from_vec(name.to_bytes().to_vec()), kind));
                }
            }
            if entries.is_buffer_empty()
                && !chunk.is_empty()
                && each(std::mem::take(&mut chunk)).is_break()
            {
                return Ok(());
            }
        }
        Ok(())
    }

    fn folder_stamp(&self, path: &Path, force_sync: bool) -> Result<FolderStamp, Errno> {
        assert_off_main_thread();
        let sync = if force_sync {
            AtFlags::STATX_FORCE_SYNC
        } else {
            AtFlags::empty()
        };
        let mask = StatxFlags::TYPE
            | StatxFlags::MODE
            | StatxFlags::SIZE
            | StatxFlags::INO
            | StatxFlags::MTIME
            | StatxFlags::CTIME
            | StatxFlags::BTIME
            | StatxFlags::MNT_ID;
        let stat = rustix::fs::statx(rustix::fs::CWD, path, sync, mask)?;
        let reported = StatxFlags::from_bits_retain(stat.stx_mask);
        Ok(FolderStamp {
            kind: FileType::from_raw_mode(u32::from(stat.stx_mode)),
            mode: u32::from(stat.stx_mode),
            size: stat.stx_size,
            modified: (stat.stx_mtime.tv_sec, stat.stx_mtime.tv_nsec),
            changed: (stat.stx_ctime.tv_sec, stat.stx_ctime.tv_nsec),
            device: rustix::fs::makedev(stat.stx_dev_major, stat.stx_dev_minor),
            inode: stat.stx_ino,
            birth: reported
                .contains(StatxFlags::BTIME)
                .then_some((stat.stx_btime.tv_sec, stat.stx_btime.tv_nsec)),
            mount_id: reported
                .contains(StatxFlags::MNT_ID)
                .then_some(stat.stx_mnt_id),
        })
    }

    fn access(&self, path: &Path, access: Access) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::access(path, access)
    }

    fn create_node(&self, at: Entry<'_>, kind: FileType, mode: Mode) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::mknodat(at.dir, at.name, kind, mode, 0)
    }

    fn open_fifo(&self, at: Entry<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        rustix::fs::openat(
            at.dir,
            at.name,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
    }

    fn create_readonly(&self, at: Entry<'_>, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        rustix::fs::openat(
            at.dir,
            at.name,
            OFlags::RDONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            mode,
        )
    }

    fn open_write(&self, at: Entry<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        // NONBLOCK, so a pipe swapped in for the claim fails at once instead
        // of waiting for a reader.
        let fd = rustix::fs::openat(
            at.dir,
            at.name,
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )?;
        rustix::fs::fcntl_setfl(&fd, OFlags::empty())?;
        Ok(fd)
    }

    fn file_flags(&self, fd: BorrowedFd<'_>) -> Result<u32, Errno> {
        assert_off_main_thread();
        rustix::fs::ioctl_getflags(fd).map(|flags| flags.bits())
    }

    fn set_link_times(
        &self,
        at: Entry<'_>,
        atime: (i64, u32),
        mtime: (i64, u32),
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        rustix::fs::utimensat(
            at.dir,
            at.name,
            &rustix::fs::Timestamps {
                last_access: timespec(atime),
                last_modification: timespec(mtime),
            },
            AtFlags::SYMLINK_NOFOLLOW,
        )
    }
}

#[cfg(test)]
mod tests;

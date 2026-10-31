// SPDX-License-Identifier: MIT

//! Emulates filesystems that lack a primitive. A scope rooted at a temporary
//! directory applies its quirks to every seam call whose directory lies
//! inside it; other calls reach the system unchanged. Separate scopes act as
//! separate mounts.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    ffi::{OsStr, OsString},
    ops::ControlFlow,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd},
        unix::ffi::OsStringExt,
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use gtk::{gio, glib, prelude::FileExt};
use rustix::{
    fs::{Access, AtFlags, FileType, FsWord, Mode, OFlags, RenameFlags, ResolveFlags, StatxFlags},
    io::Errno,
};

use super::super::copy_engine::clock_for_test;
use super::super::environment::FsEnvironment;
use super::super::fs_ops::{
    Entry, EntryStat, FolderStamp, FsOps, FsStat, Listed, System, Watched, assert_off_main_thread,
};
use super::super::holders;
use super::super::native_fs::LocalFileIdentity;
use crate::{
    adapters::volume::{
        Family, MountKey, MountSite, MountTable, MountType, family, forget_test_mount_capabilities,
    },
    services::Holder,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Op {
    OpenAt,
    OpenPath,
    Dup,
    StatVfs,
    Rename,
    Exchange,
    Link,
    Unlink,
    Mkdir,
    Symlink,
    Readlink,
    Stat,
    StatFd,
    OpenParent,
    Create,
    Names,
    MountKey,
    Statfs,
    CanTrash,
    Trash,
    OpenSource,
    OpenTmpfile,
    LinkFd,
    Clone,
    CopyRange,
    Read,
    Write,
    SetLen,
    SeekData,
    SeekHole,
    SyncData,
    SyncFs,
    SyncDir,
    Close,
    ChownFd,
    ChmodFd,
    SetTimes,
    XattrNames,
    GetXattr,
    SetXattr,
    MountType,
    CurrentMountType,
    DeviceFormat,
    OpenDir,
    List,
    ListChunks,
    FolderStamp,
    Access,
    Watch,
    Unwatch,
    CreateNode,
    OpenFifo,
    CreateReadonly,
    OpenWrite,
    FileFlags,
    SetLinkTimes,
    Holders,
}

impl Op {
    fn changes(self) -> bool {
        matches!(
            self,
            Self::Rename
                | Self::Exchange
                | Self::Link
                | Self::Unlink
                | Self::Mkdir
                | Self::Symlink
                | Self::Create
                | Self::Trash
                | Self::LinkFd
                | Self::Clone
                | Self::CopyRange
                | Self::Write
                | Self::SetLen
                | Self::ChownFd
                | Self::ChmodFd
                | Self::SetTimes
                | Self::SetXattr
                | Self::CreateNode
                | Self::CreateReadonly
                | Self::SetLinkTimes
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum Support {
    #[default]
    Supported,
    Rejected(Errno),
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum Exchange {
    #[default]
    Supported,
    Rejected,
    /// Overwrites the target instead of swapping, as some FUSE daemons do.
    IgnoredOverwrites,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum OverEmptyDirectory {
    #[default]
    Supported,
    NotEmpty,
    Eperm,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum Trash {
    #[default]
    Supported,
    Unsupported,
    /// GLib #4060: FALSE without an error, and nothing moved.
    FalseWithoutError,
    /// TRUE, but nothing moved.
    SucceedsWithoutMoving,
    /// A FUSE backend refuses a step of the trash protocol.
    ProtocolRefused,
    /// Offered, but moving an item there is not supported.
    Refused,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum TmpfileLink {
    #[default]
    Any,
    /// `AT_EMPTY_PATH` needs privileges, as before Linux 6.10.
    ProcOnly,
    /// Neither form works, as before 6.10 without `/proc`.
    Unavailable,
}

/// Opening a name of a file fails with `EINVAL` where another name of it
/// is in the way, as the SMB client's one lease per file makes a second
/// open fail, or a Samba server defer it forever.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum NameOpens {
    #[default]
    Any,
    /// Once another name was opened at all, which pins that a copy never
    /// opens a later name.
    FirstOnly,
    /// While another name is open.
    OneAtATime,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Aside {
    Nfs,
    Smb,
    Fuse,
    Unlisted(Errno),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Silly {
    pub aside: Aside,
    pub lingers: Duration,
}

impl Silly {
    pub(crate) fn nfs() -> Self {
        Self {
            aside: Aside::Nfs,
            lingers: Duration::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CachedTimes {
    pub folders: Duration,
    pub files: Duration,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CachedListing {
    pub kept_for: Duration,
    pub renewed_by_use: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Births {
    #[default]
    Reported,
    MadeUp,
    /// A birth time per file, kept across renumbering, on a base that reports none (as FAT).
    Recorded,
}

pub(crate) type Racer = Box<dyn FnMut() + Send>;
pub(crate) type Preset = fn() -> Quirks;

/// Patterns match a name exactly, or by prefix when they end in `*`.
#[derive(Default)]
pub(crate) struct Quirks {
    pub rename_flags: Support,
    pub exchange: Exchange,
    pub links: Support,
    pub collisions_as_eperm: bool,
    pub rename_over_empty_dir: OverEmptyDirectory,
    /// A rename over an existing entry links it to `<name>.ntfs-3g-0000000001` first, a name
    /// stored cut to its UTF-16 length modulo 256 that stays: a cut to nothing fails with
    /// `EINVAL`, as ntfs-3g does.
    pub cut_rename_over_names: bool,
    pub case_insensitive: bool,
    /// Keeps answering for a name found through a case alias until a forced
    /// revalidation, as the SMB client's lookup cache does for about a second.
    pub stale_aliases: bool,
    /// A case alias reports its own inode number, as libfuse without `use_ino` does.
    pub alias_inodes: bool,
    /// Each name a hard link adds reports its own inode number, which a rename
    /// keeps, as libfuse without `use_ino` (sshfs) and cifs without server
    /// inode numbers do.
    pub link_inodes: bool,
    /// Every entry reports one link, as sshfs does, whose SFTP attributes have none.
    pub single_link_count: bool,
    /// A rename whose reply is lost leaves its target name reporting the inode
    /// number it had, as libfuse without `use_ino` moves a node only on success.
    pub lost_renames_keep_inodes: bool,
    /// Names rejected with the returned errno by rename and every create,
    /// and with `EPERM` by link, as vfat refuses them.
    pub invalid_name: Option<fn(&OsStr) -> Option<Errno>>,
    /// The name a create or rename stores instead, which lookups of the
    /// requested name then find, as vfat and exFAT drop trailing dots.
    pub stored_as: Option<fn(&OsStr) -> Option<OsString>>,
    /// Creating or renaming to a name longer than this many bytes fails
    /// with the errno. Every root's own filesystem already stops at 255.
    pub name_limit: Option<(usize, Errno)>,
    /// Creating a folder whose name is longer than this many bytes fails
    /// with the errno, as Samba makes it under a longer temporary name.
    pub folder_name_limit: Option<(usize, Errno)>,
    pub symlinks: Support,
    pub fs_type: Option<FsWord>,
    pub name_max: Option<u32>,
    pub trash: Trash,
    /// Reports the first inode number seen at each matching name for whatever
    /// is there later, as ext4, NFS and CIFS hand a freed number straight out,
    /// with a birth time that changes only when [`recreated`] marks a new item.
    pub reused_inodes: Option<&'static str>,
    /// As on NFSv3, ntfs-3g and exfat-fuse.
    pub no_birth_time: bool,
    pub births: Births,
    /// The change time reports the modification time, as on FAT and exFAT,
    /// which store none.
    pub ctime_is_mtime: bool,
    /// A rename into or within a folder sets the folder's birth time to the
    /// rename's, as the kernel's exFAT driver does.
    pub renames_reset_folder_births: bool,
    /// Performs the call, then reports this errno once.
    pub lost_reply: Vec<(Op, &'static str, Errno)>,
    /// Fails the matching call after skipping the given number of matches, once.
    pub fail: Vec<(Op, &'static str, usize, Errno)>,
    /// Runs once, before the matching call after skipping the given number of matches.
    pub before: Vec<(Op, &'static str, usize, Racer)>,
    /// Runs once, after the matching call after skipping the given number of matches, whether
    /// or not the call succeeded.
    pub after: Vec<(Op, &'static str, usize, Racer)>,
    pub unplugged_after: Option<(Op, &'static str, usize)>,
    pub mount_type: Option<(&'static str, &'static str)>,
    pub mount_root: Option<&'static str>,
    pub device_format: Option<&'static str>,
    pub rotational: Option<bool>,
    pub tmpfile: Support,
    pub tmpfile_link: TmpfileLink,
    /// The most bytes one read or write moves.
    pub short_io: Option<usize>,
    /// Closing a written file sets its times to the close time, as NFSv4.2
    /// write delegations and FUSE writeback caches do.
    pub times_reset_at_close: bool,
    /// A file opened for writing reports the modification time it had then through any stat
    /// that does not force revalidation, until a forced stat or a time set revalidates it, as
    /// the SMB client leaves a written file's times to the server.
    pub stale_written_times: bool,
    pub chown: Support,
    /// Entries report a group the user is not in.
    pub foreign_group: bool,
    /// Refuses setting xattrs whose names start with the prefix.
    pub xattrs_refused: Vec<(&'static str, Errno)>,
    /// Listed and read on every file, besides its real xattrs.
    pub extra_xattrs: Vec<(&'static str, &'static [u8])>,
    /// `mknodat` of a regular file, which FUSE daemons without `mknod` refuse.
    pub node_create: Support,
    /// `mknodat` of a named pipe, which FAT, exFAT and SMB refuse.
    pub fifos: Support,
    pub silly_rename: Option<Silly>,
    pub holder: Option<Holder>,
    pub folder_renames_refused_below: Option<Duration>,
    /// Files report a size of 0 whatever they hold, so `copy_file_range`
    /// copies nothing from them, as virtual files on FUSE (lxcfs) do.
    pub virtual_sizes: bool,
    /// `FICLONE`, which btrfs and XFS hosts would otherwise perform.
    pub reflink: Support,
    /// `copy_file_range` copies nothing past this offset, as when a source
    /// shrinks after it was opened.
    pub copy_range_end: Option<u64>,
    /// Entries matching the name report this mode, as another user's file
    /// readable through its group or others does to the user copying it.
    pub reported_mode: Option<(&'static str, u32)>,
    pub name_opens: NameOpens,
    /// Opening a matching name fails with `EINVAL` until the test clock
    /// reaches the time, as the SMB client refuses another name of a file
    /// this machine wrote while it keeps the closed handle (`closetimeo`).
    pub deferred_close: Option<(&'static str, SystemTime)>,
    /// Giving a folder write access back right after taking it does
    /// nothing until a stat that revalidates comes between, as the SMB
    /// client without POSIX extensions decides from attributes it cached.
    pub stale_read_only: bool,
    /// Lists `system.*` names, such as ACLs, before the others, as tmpfs
    /// and NFSv4 do whatever order the host's filesystem keeps.
    pub acls_listed_first: bool,
    /// Entries report a device of their own, to the seam and to GIO, as a
    /// filesystem mounted inside another does; scopes otherwise share the
    /// device of the folder they are made in, as bind mounts do.
    pub own_device: bool,
    pub no_mount_ids: bool,
    /// A stat that does not force revalidation reports the change time the
    /// last stat of the entry saw, until this long after by the test clock,
    /// as the SMB client answers from its attribute cache (`actimeo`).
    pub cached_change_times: Option<Duration>,
    pub cached_times: Option<CachedTimes>,
    pub daemon_cache: Option<Duration>,
    pub cached_listing: Option<CachedListing>,
    /// Times stamped at or after this instant read as it, as one tick of the coarse clock
    /// stamps them where the kernel has no multigrain timestamps. The tick lasts as long as
    /// the scope.
    pub coarse_clock: Option<SystemTime>,
    pub listing_chunk: Option<usize>,
}

impl Quirks {
    pub(crate) fn without_rename_flags() -> Self {
        Self {
            rename_flags: Support::Rejected(Errno::INVAL),
            ..Self::default()
        }
    }

    /// No `RENAME_NOREPLACE`, no hard links, and no rename over an empty folder.
    pub(crate) fn vboxsf() -> Self {
        Self {
            mount_type: Some(("vboxsf", VBOXSF_OPTIONS)),
            fs_type: Some(0x786f_4256),
            rename_flags: Support::Rejected(Errno::INVAL),
            links: Support::Rejected(Errno::PERM),
            rename_over_empty_dir: OverEmptyDirectory::NotEmpty,
            no_birth_time: true,
            ..Self::default()
        }
    }

    /// Hard links work, so folders never take the reserved-name rung, and
    /// nothing else places a folder without `RENAME_NOREPLACE`.
    pub(crate) fn without_folder_placement() -> Self {
        Self {
            rename_over_empty_dir: OverEmptyDirectory::NotEmpty,
            ..Self::without_rename_flags()
        }
    }

    /// No rung that cannot replace: no `RENAME_NOREPLACE` or rename over an
    /// empty folder, and links fail with an errno that says nothing about the
    /// mount, so the reserved-name rung stays unconfirmed.
    pub(crate) fn without_placement() -> Self {
        Self {
            links: Support::Rejected(Errno::MLINK),
            ..Self::without_folder_placement()
        }
    }

    pub(crate) fn ext4() -> Self {
        Self::default().on_ext4()
    }

    pub(crate) fn on_ext4(self) -> Self {
        Self {
            mount_type: Some(EXT4),
            fs_type: Some(EXT4_SUPER_MAGIC),
            ..self
        }
    }

    /// NOREPLACE fails with `EINVAL`, and a write delegation sets the
    /// times of a written file when it is closed.
    pub(crate) fn nfs42() -> Self {
        Self {
            mount_type: Some(("nfs4", "rw,vers=4.2")),
            fs_type: Some(0x6969),
            rename_flags: Support::Rejected(Errno::INVAL),
            times_reset_at_close: true,
            silly_rename: Some(Silly {
                lingers: Duration::from_millis(2),
                ..Silly::nfs()
            }),
            cached_times: Some(CachedTimes {
                folders: Duration::from_secs(30),
                files: Duration::from_secs(3),
            }),
            ..Self::default()
        }
    }

    pub(crate) fn nfs3() -> Self {
        Self {
            mount_type: Some(("nfs", "rw,vers=3")),
            times_reset_at_close: false,
            xattrs_refused: vec![("", Errno::OPNOTSUPP)],
            no_birth_time: true,
            silly_rename: Some(Silly::nfs()),
            ..Self::nfs42()
        }
    }

    /// No hard links, symbolic links, named pipes or xattrs, and names
    /// compared without case. This and the presets after it carry the
    /// mountinfo line of a lab mount, and refuse and store names as it did.
    pub(crate) fn vfat() -> Self {
        Self {
            mount_type: Some(("vfat", VFAT_OPTIONS)),
            fs_type: Some(0x4d44),
            name_max: Some(1530),
            invalid_name: Some(vfat_refuses),
            stored_as: Some(without_trailing_dots),
            links: Support::Rejected(Errno::PERM),
            symlinks: Support::Rejected(Errno::PERM),
            fifos: Support::Rejected(Errno::PERM),
            case_insensitive: true,
            xattrs_refused: vec![("", Errno::OPNOTSUPP)],
            births: Births::Recorded,
            ..Self::default()
        }
    }

    pub(crate) fn exfat() -> Self {
        Self {
            mount_type: Some(("exfat", EXFAT_OPTIONS)),
            fs_type: Some(0x2011_bab0),
            invalid_name: Some(exfat_refuses),
            renames_reset_folder_births: true,
            ..Self::vfat()
        }
    }

    pub(crate) fn exfat_fuse() -> Self {
        Self {
            mount_type: Some(("fuseblk", FUSEBLK_OPTIONS)),
            device_format: Some("exfat"),
            fs_type: Some(FUSE_SUPER_MAGIC),
            name_max: Some(255),
            invalid_name: Some(exfat_fuse_refuses),
            links: Support::Rejected(Errno::NOSYS),
            symlinks: Support::Rejected(Errno::NOSYS),
            case_insensitive: true,
            no_birth_time: true,
            silly_rename: Some(FUSEBLK_SILLY),
            ..Self::default()
        }
    }

    pub(crate) fn ntfs3() -> Self {
        Self {
            mount_type: Some(("ntfs3", NTFS3_OPTIONS)),
            fs_type: Some(0x7366_746e),
            name_max: Some(255),
            invalid_name: Some(ntfs3_refuses),
            ..Self::default()
        }
    }

    pub(crate) fn ntfs3_windows_names() -> Self {
        Self {
            mount_type: Some(("ntfs3", NTFS3_WINDOWS_NAMES_OPTIONS)),
            invalid_name: Some(windows_names_refuses),
            ..Self::ntfs3()
        }
    }

    /// udisks passes `windows_names`, which mountinfo does not show.
    pub(crate) fn ntfs_3g() -> Self {
        Self {
            mount_type: Some(("fuseblk", FUSEBLK_OPTIONS)),
            device_format: Some("ntfs"),
            fs_type: Some(FUSE_SUPER_MAGIC),
            name_max: Some(255),
            invalid_name: Some(ntfs_3g_refuses),
            no_birth_time: true,
            cut_rename_over_names: true,
            silly_rename: Some(FUSEBLK_SILLY),
            ..Self::default()
        }
    }

    /// Samba with the client's default `mapposix`, which stores characters
    /// Windows refuses as private-use ones.
    pub(crate) fn cifs() -> Self {
        Self {
            mount_type: Some(("cifs", CIFS_OPTIONS)),
            fs_type: Some(0xfe53_4d42),
            name_max: Some(255),
            invalid_name: Some(cifs_refuses),
            stored_as: Some(smb_listed),
            folder_name_limit: Some((217, Errno::NOENT)),
            symlinks: Support::Rejected(Errno::OPNOTSUPP),
            case_insensitive: true,
            cached_change_times: Some(Duration::from_secs(1)),
            stale_written_times: true,
            silly_rename: Some(Silly {
                aside: Aside::Smb,
                lingers: Duration::from_millis(4),
            }),
            cached_times: Some(CachedTimes {
                folders: Duration::from_secs(1),
                files: Duration::from_secs(1),
            }),
            cached_listing: Some(CachedListing {
                kept_for: Duration::from_secs(46),
                renewed_by_use: true,
            }),
            ..Self::default()
        }
    }

    pub(crate) fn cifs_noserverino() -> Self {
        Self {
            mount_type: Some(("cifs", CIFS_NOSERVERINO_OPTIONS)),
            ..Self::cifs()
        }
    }

    pub(crate) fn cifs_nocase() -> Self {
        Self {
            mount_type: Some(("cifs", CIFS_NOCASE_OPTIONS)),
            invalid_name: Some(cifs_nocase_refuses),
            ..Self::cifs()
        }
    }

    pub(crate) fn cifs_posix() -> Self {
        Self {
            mount_type: Some(("cifs", CIFS_POSIX_OPTIONS)),
            invalid_name: None,
            stored_as: Some(question_marks),
            symlinks: Support::Supported,
            case_insensitive: false,
            silly_rename: Some(Silly {
                aside: Aside::Unlisted(Errno::ACCESS),
                lingers: Duration::from_millis(1),
            }),
            ..Self::cifs()
        }
    }

    pub(crate) fn sshfs() -> Self {
        Self {
            mount_type: Some(("fuse.sshfs", "rw,user_id=0,group_id=0,allow_other")),
            fs_type: Some(FUSE_SUPER_MAGIC),
            name_max: Some(255),
            name_limit: Some((255, Errno::BADMSG)),
            xattrs_refused: vec![("", Errno::OPNOTSUPP)],
            rename_flags: Support::Rejected(Errno::INVAL),
            collisions_as_eperm: true,
            link_inodes: true,
            single_link_count: true,
            lost_renames_keep_inodes: true,
            no_birth_time: true,
            silly_rename: Some(Silly {
                aside: Aside::Fuse,
                lingers: Duration::from_millis(70),
            }),
            daemon_cache: Some(Duration::from_secs(21)),
            cached_listing: Some(CachedListing {
                kept_for: Duration::from_secs(21),
                renewed_by_use: false,
            }),
            ..Self::default()
        }
    }
}

const FUSE_SUPER_MAGIC: FsWord = 0x6573_5546;
const FUSEBLK_SILLY: Silly = Silly {
    aside: Aside::Fuse,
    lingers: Duration::ZERO,
};
const EXT4_SUPER_MAGIC: FsWord = 0xEF53;
const EXT4: (&str, &str) = ("ext4", "rw");
const VBOXSF_OPTIONS: &str = "rw,nodev,relatime,iocharset=utf8,uid=1000,gid=1000";
const VFAT_OPTIONS: &str = concat!(
    "rw,uid=1000,gid=1000,fmask=0022,dmask=0022,codepage=437,iocharset=ascii,",
    "shortname=mixed,showexec,utf8,flush,errors=remount-ro"
);
const EXFAT_OPTIONS: &str =
    "rw,uid=1000,gid=1000,fmask=0022,dmask=0022,iocharset=utf8,errors=remount-ro";
const FUSEBLK_OPTIONS: &str =
    "rw,user_id=0,group_id=0,default_permissions,allow_other,blksize=4096";
const NTFS3_OPTIONS: &str = "rw,uid=1000,gid=1000,acl,iocharset=utf8,prealloc";
const NTFS3_WINDOWS_NAMES_OPTIONS: &str =
    "rw,uid=1000,gid=1000,windows_names,acl,iocharset=utf8,prealloc";
const CIFS_OPTIONS: &str = concat!(
    "rw,vers=3.1.1,cache=strict,uid=1000,forceuid,gid=1000,forcegid,soft,nounix,serverino,",
    "mapposix,reparse=nfs,nativesocket,symlink=native,actimeo=1,closetimeo=1"
);
const CIFS_NOSERVERINO_OPTIONS: &str = concat!(
    "rw,vers=3.1.1,cache=strict,uid=1000,forceuid,gid=1000,forcegid,soft,nounix,mapposix,",
    "reparse=nfs,nativesocket,symlink=native,actimeo=1,closetimeo=1"
);
const CIFS_NOCASE_OPTIONS: &str = concat!(
    "rw,vers=3.1.1,cache=strict,uid=1000,forceuid,gid=1000,forcegid,nocase,soft,nounix,",
    "serverino,mapposix,reparse=nfs,nativesocket,symlink=native,actimeo=1,closetimeo=1"
);
const CIFS_POSIX_OPTIONS: &str = concat!(
    "rw,vers=3.1.1,cache=strict,uid=1000,forceuid,gid=1000,forcegid,soft,posix,posixpaths,",
    "serverino,reparse=nfs,nonativesocket,symlink=native,actimeo=1,closetimeo=1"
);

/// How a name breaks UTF-8, worst last. A decoder that only checks the
/// shape of each sequence takes overlong forms and encoded surrogates.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Utf8Fault {
    None,
    Surrogate,
    Overlong,
    Shape,
}

fn utf8_fault(name: &OsStr) -> Utf8Fault {
    let bytes = name.as_encoded_bytes();
    let mut worst = Utf8Fault::None;
    let mut index = 0;
    while index < bytes.len() {
        let len = match bytes[index] {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => return Utf8Fault::Shape,
        };
        let Some(sequence) = bytes
            .get(index..index + len)
            .filter(|sequence| sequence[1..].iter().all(|byte| byte & 0xc0 == 0x80))
        else {
            return Utf8Fault::Shape;
        };
        let fault = match std::str::from_utf8(sequence) {
            Ok(_) => Utf8Fault::None,
            Err(_) if sequence[0] == 0xed => Utf8Fault::Surrogate,
            Err(_) => Utf8Fault::Overlong,
        };
        worst = worst.max(fault);
        index += len;
    }
    worst
}

fn has_windows_forbidden(name: &OsStr) -> bool {
    name.as_encoded_bytes()
        .iter()
        .any(|byte| b"\\:*?\"<>|".contains(byte) || (0x01..0x20).contains(byte))
}

fn trimmed_dots(name: &OsStr) -> &[u8] {
    let bytes = name.as_encoded_bytes();
    let end = bytes
        .iter()
        .rposition(|byte| *byte != b'.')
        .map_or(0, |last| last + 1);
    &bytes[..end]
}

fn ends_with_dot_or_space(name: &OsStr) -> bool {
    matches!(name.as_encoded_bytes().last(), Some(b'.' | b' '))
}

/// The device names both `windows_names` drivers check: `CON`, `PRN`,
/// `AUX`, `NUL`, `COM1`-`COM9` and `LPT1`-`LPT9`, alone or before a dot.
fn device_name(name: &OsStr) -> bool {
    let bytes = name.as_encoded_bytes();
    let ends_at = |len: usize| bytes.len() == len || bytes.get(len) == Some(&b'.');
    let base = |len: usize| bytes.get(..len).map(<[u8]>::to_ascii_uppercase);
    let three = base(3).is_some_and(|base| {
        [&b"CON"[..], b"PRN", b"AUX", b"NUL"].contains(&base.as_slice()) && ends_at(3)
    });
    let numbered = base(3).is_some_and(|base| [&b"COM"[..], b"LPT"].contains(&base.as_slice()))
        && bytes
            .get(3)
            .is_some_and(|digit| (b'1'..=b'9').contains(digit))
        && ends_at(4);
    three || numbered
}

fn vfat_refuses(name: &OsStr) -> Option<Errno> {
    if utf8_fault(name) != Utf8Fault::None || has_windows_forbidden(name) {
        return Some(Errno::INVAL);
    }
    match trimmed_dots(name) {
        [] => Some(Errno::NOENT),
        [.., b' '] => Some(Errno::INVAL),
        _ => None,
    }
}

fn exfat_refuses(name: &OsStr) -> Option<Errno> {
    if utf8_fault(name) != Utf8Fault::None {
        Some(Errno::ILSEQ)
    } else if has_windows_forbidden(name) {
        Some(Errno::INVAL)
    } else {
        trimmed_dots(name).is_empty().then_some(Errno::NOENT)
    }
}

/// The daemon stores overlong forms and encoded surrogates, writing an
/// overlong `/` as a real one and aborting at the next listing. Nothing may
/// send them, so they fail with an errno no driver gives.
fn exfat_fuse_refuses(name: &OsStr) -> Option<Errno> {
    match utf8_fault(name) {
        Utf8Fault::Shape => Some(Errno::ILSEQ),
        Utf8Fault::Overlong | Utf8Fault::Surrogate => Some(Errno::UCLEAN),
        Utf8Fault::None => has_windows_forbidden(name).then_some(Errno::NOENT),
    }
}

fn ntfs3_refuses(name: &OsStr) -> Option<Errno> {
    (utf8_fault(name) != Utf8Fault::None).then_some(Errno::INVAL)
}

fn windows_names_refuses(name: &OsStr) -> Option<Errno> {
    (utf8_fault(name) != Utf8Fault::None
        || has_windows_forbidden(name)
        || ends_with_dot_or_space(name)
        || device_name(name))
    .then_some(Errno::INVAL)
}

/// Encoded surrogates pass.
fn ntfs_3g_refuses(name: &OsStr) -> Option<Errno> {
    if utf8_fault(name) >= Utf8Fault::Overlong {
        Some(Errno::ILSEQ)
    } else {
        (has_windows_forbidden(name) || ends_with_dot_or_space(name) || device_name(name))
            .then_some(Errno::INVAL)
    }
}

/// Only `\` is refused; `mapposix` maps the other characters.
fn cifs_refuses(name: &OsStr) -> Option<Errno> {
    if utf8_fault(name) != Utf8Fault::None {
        Some(Errno::NOENT)
    } else {
        name.as_encoded_bytes()
            .contains(&b'\\')
            .then_some(Errno::INVAL)
    }
}

/// `nocase` also refuses every character above U+FFFF.
fn cifs_nocase_refuses(name: &OsStr) -> Option<Errno> {
    let astral = name
        .to_str()
        .is_some_and(|name| name.chars().any(|character| u32::from(character) > 0xffff));
    (utf8_fault(name) != Utf8Fault::None || astral || cifs_refuses(name).is_some())
        .then_some(Errno::INVAL)
}

/// What the SMB client lists: the character each `mapposix` code point
/// stands for, and an 8.3 alias for a name Samba takes for a device.
fn smb_listed(name: &OsStr) -> Option<OsString> {
    let name = name.to_str()?;
    let (base, extension) = name.split_once('.').unwrap_or((name, ""));
    let reserved = [
        "AUX", "CLOCK$", "COM1", "COM2", "COM3", "COM4", "CON", "LPT1", "LPT2", "LPT3", "NUL",
        "PRN",
    ];
    if reserved
        .iter()
        .any(|device| base.eq_ignore_ascii_case(device))
    {
        let extension = extension.rsplit('.').next().unwrap_or_default();
        let extension = extension.get(..3).unwrap_or(extension).to_ascii_uppercase();
        let dot = if extension.is_empty() { "" } else { "." };
        let first = base[..1].to_ascii_uppercase();
        return Some(OsString::from(format!("{first}MANGL~F{dot}{extension}")));
    }
    let listed = name
        .chars()
        .map(|character| match character {
            '\u{f001}'..='\u{f01f}' => {
                char::from_u32(u32::from(character) - 0xf000).unwrap_or(character)
            }
            '\u{f020}' => '"',
            '\u{f021}' => '*',
            '\u{f022}' => ':',
            '\u{f023}' => '<',
            '\u{f024}' => '>',
            '\u{f025}' => '?',
            '\u{f027}' => '|',
            '\u{f028}' => ' ',
            '\u{f029}' => '.',
            character => character,
        })
        .collect::<String>();
    (listed != name).then(|| OsString::from(listed))
}

fn without_trailing_dots(name: &OsStr) -> Option<OsString> {
    let trimmed = trimmed_dots(name);
    (trimmed.len() < name.len()).then(|| OsString::from_vec(trimmed.to_vec()))
}

/// SMB carries UTF-16, so the client sends `?` for every byte that is not UTF-8.
fn question_marks(name: &OsStr) -> Option<OsString> {
    let bytes = name.as_encoded_bytes();
    if std::str::from_utf8(bytes).is_ok() {
        return None;
    }
    let mut stored = Vec::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        stored.extend_from_slice(chunk.valid().as_bytes());
        stored.extend(chunk.invalid().iter().map(|_| b'?'));
    }
    Some(OsString::from_vec(stored))
}

#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub op: Op,
    pub names: Vec<OsString>,
    /// A stat that forced revalidation.
    pub forced: bool,
}

#[derive(Clone, Copy)]
struct WrittenTime {
    birth: Option<(i64, u32)>,
    modified: (i64, u32),
}

#[derive(Clone, Copy)]
struct Seen {
    modified: (i64, u32),
    changed: (i64, u32),
    fetch: Fetch,
}

#[derive(Clone, Copy)]
struct Fetch {
    at: SystemTime,
    own_changes: u64,
}

impl Fetch {
    fn kept(&self, kept_for: Duration, now: Fetch) -> bool {
        self.own_changes == now.own_changes
            && now
                .at
                .duration_since(self.at)
                .is_ok_and(|age| age < kept_for)
    }
}

struct Scope {
    id: u64,
    root: PathBuf,
    mount_id: Option<u64>,
    host: Option<MountType>,
    own_device: bool,
    state: Mutex<State>,
    /// Makes looking a name up and creating it one step, as a filesystem that
    /// compares names itself does.
    naming: Mutex<()>,
}

struct State {
    quirks: Quirks,
    calls: Vec<Call>,
    inodes: HashMap<PathBuf, u64>,
    births: HashMap<PathBuf, i64>,
    /// The birth time of each folder a rename reset, by device and inode.
    reset_births: HashMap<(u64, u64), (i64, u32)>,
    /// Real and reported inode number per name a hard link added, or a lost
    /// rename reply left reporting its old number.
    link_names: HashMap<PathBuf, (u64, u64)>,
    links_made: u64,
    stale: HashMap<PathBuf, EntryStat>,
    written: HashSet<(u64, u64)>,
    written_times: HashMap<(u64, u64), WrittenTime>,
    renumbered: HashSet<PathBuf>,
    /// Writebacks lost so far, and how many each open folder has reported.
    writebacks_lost: u64,
    reported_losses: HashMap<RawFd, u64>,
    /// Each name a file was opened by, by device and inode, with the
    /// descriptor that opened it.
    opened_names: Vec<((u64, u64), PathBuf, RawFd)>,
    /// Folders made read-only that no revalidating stat has seen since.
    read_only_unseen: HashSet<PathBuf>,
    /// The change time each entry's last stat saw, and when, by device and inode.
    seen_change_times: HashMap<(u64, u64), ((i64, u32), SystemTime)>,
    seen_times: HashMap<(u64, u64), Seen>,
    listings: HashMap<PathBuf, (Listed, Fetch)>,
    own_changes: u64,
    unplugged: bool,
    serverino_disabled: bool,
    cached: HashMap<PathBuf, (u64, Option<(i64, u32)>)>,
    created: HashSet<(u64, u64)>,
    made_up: HashMap<(u64, u64), (i64, u32)>,
    recorded_births: HashMap<(u64, u64), SystemTime>,
    latest_birth: SystemTime,
    holds: HashMap<(u64, u64), usize>,
    held_paths: HashMap<(u64, u64), PathBuf>,
    closed_holds: Vec<(PathBuf, SystemTime)>,
    leftovers: Vec<SillyLeftover>,
    silly_renames: u64,
}

struct SillyLeftover {
    folder: PathBuf,
    folder_key: (u64, u64),
    name: Option<OsString>,
    key: (u64, u64),
    unheld_since: Option<SystemTime>,
}

impl State {
    fn fetch(&self) -> Fetch {
        Fetch {
            at: clock_for_test(),
            own_changes: self.own_changes,
        }
    }

    fn recorded_birth(&mut self, key: (u64, u64)) -> Option<(i64, u32)> {
        let latest = &mut self.latest_birth;
        let born = *self.recorded_births.entry(key).or_insert_with(|| {
            *latest = SystemTime::now().max(*latest + Duration::from_nanos(1));
            *latest
        });
        timespec(born)
    }
}

static SCOPES: Mutex<Vec<Arc<Scope>>> = Mutex::new(Vec::new());
static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);
static FIRST_FOLDER_BIRTHS: Mutex<BTreeMap<(u64, u64), (i64, u32)>> = Mutex::new(BTreeMap::new());

fn scopes() -> MutexGuard<'static, Vec<Arc<Scope>>> {
    SCOPES.lock().unwrap_or_else(|error| error.into_inner())
}

fn first_folder_births() -> MutexGuard<'static, BTreeMap<(u64, u64), (i64, u32)>> {
    FIRST_FOLDER_BIRTHS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub(crate) struct FsQuirks {
    scope: Arc<Scope>,
}

impl FsQuirks {
    pub(crate) fn install(root: &Path, quirks: Quirks) -> Self {
        let mount_id =
            rustix::fs::statx(rustix::fs::CWD, root, AtFlags::empty(), StatxFlags::MNT_ID)
                .ok()
                .filter(|stat| {
                    StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::MNT_ID)
                })
                .map(|stat| stat.stx_mnt_id);
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let scope = Arc::new(Scope {
            id: NEXT_SCOPE.fetch_add(1, Ordering::Relaxed),
            host: host_mount_type(mount_id, &root),
            root,
            mount_id,
            own_device: quirks.own_device,
            naming: Mutex::new(()),
            state: Mutex::new(State {
                quirks,
                calls: Vec::new(),
                inodes: HashMap::new(),
                births: HashMap::new(),
                reset_births: HashMap::new(),
                link_names: HashMap::new(),
                links_made: 0,
                stale: HashMap::new(),
                written: HashSet::new(),
                written_times: HashMap::new(),
                renumbered: HashSet::new(),
                writebacks_lost: 0,
                reported_losses: HashMap::new(),
                opened_names: Vec::new(),
                read_only_unseen: HashSet::new(),
                seen_change_times: HashMap::new(),
                seen_times: HashMap::new(),
                listings: HashMap::new(),
                own_changes: 0,
                unplugged: false,
                serverino_disabled: false,
                holds: HashMap::new(),
                held_paths: HashMap::new(),
                closed_holds: Vec::new(),
                leftovers: Vec::new(),
                silly_renames: 0,
                cached: HashMap::new(),
                created: HashSet::new(),
                made_up: HashMap::new(),
                recorded_births: HashMap::new(),
                latest_birth: SystemTime::UNIX_EPOCH,
            }),
        });
        scopes().push(scope.clone());
        Self { scope }
    }

    pub(crate) fn key(&self) -> MountKey {
        MountKey::Test(self.scope.id)
    }

    /// Loses a writeback in this scope, from a racer: as errseq does,
    /// `syncfs` reports it once through each folder opened before, and
    /// never through one opened after.
    pub(crate) fn writeback_loser(&self) -> impl Fn() + Send + Sync + 'static {
        let scope = self.scope.clone();
        move || scope.state().writebacks_lost += 1
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.scope.state().calls.clone()
    }

    pub(crate) fn reader(&self) -> impl Fn() -> Vec<Call> + Send + Sync + 'static {
        let scope = self.scope.clone();
        move || scope.state().calls.clone()
    }

    pub(crate) fn count(&self, op: Op) -> usize {
        self.scope
            .state()
            .calls
            .iter()
            .filter(|call| call.op == op)
            .count()
    }

    pub(crate) fn clear_calls(&self) {
        self.scope.state().calls.clear();
    }

    pub(crate) fn replug(&self) {
        self.scope.state().unplugged = false;
    }

    pub(crate) fn used_name(&self, pattern: &str) -> bool {
        self.scope.state().calls.iter().any(|call| {
            let names: Vec<&OsStr> = call.names.iter().map(OsString::as_os_str).collect();
            matches(pattern, &names)
        })
    }
}

impl Drop for FsQuirks {
    fn drop(&mut self) {
        scopes().retain(|scope| !Arc::ptr_eq(scope, &self.scope));
        forget_test_mount_capabilities(self.scope.id);
    }
}

pub(crate) struct HoldGuard {
    scope: Arc<Scope>,
    key: (u64, u64),
}

impl Drop for HoldGuard {
    fn drop(&mut self) {
        let mut state = self.scope.state();
        let Some(count) = state.holds.get_mut(&self.key) else {
            return;
        };
        *count -= 1;
        if *count > 0 {
            return;
        }
        state.holds.remove(&self.key);
        let now = clock_for_test();
        if let Some(path) = state.held_paths.remove(&self.key) {
            state.closed_holds.push((path, now));
        }
        for leftover in &mut state.leftovers {
            if leftover.key == self.key {
                leftover.unheld_since.get_or_insert(now);
            }
        }
    }
}

fn take_racer(
    racers: &mut Vec<(Op, &'static str, usize, Racer)>,
    op: Op,
    names: &[&OsStr],
) -> Option<Racer> {
    let index = racers
        .iter()
        .position(|(racer_op, pattern, _, _)| *racer_op == op && matches(pattern, names))?;
    if racers[index].2 == 0 {
        return Some(racers.remove(index).3);
    }
    racers[index].2 -= 1;
    None
}

fn still_open(fd: RawFd, name: &Path, key: (u64, u64)) -> bool {
    use std::os::unix::fs::MetadataExt;
    let link = format!("/proc/self/fd/{fd}");
    std::fs::read_link(&link).is_ok_and(|opened| opened == name)
        && std::fs::metadata(&link).is_ok_and(|file| (file.dev(), file.ino()) == key)
}

/// A bare `*` also matches calls that name nothing.
fn matches(pattern: &str, names: &[&OsStr]) -> bool {
    pattern == "*"
        || names.iter().any(|name| match pattern.strip_suffix('*') {
            Some(prefix) => name.as_encoded_bytes().starts_with(prefix.as_bytes()),
            None => name.as_encoded_bytes() == pattern.as_bytes(),
        })
}

impl Scope {
    fn record_open(&self, opened: OwnedFd) -> OwnedFd {
        let mut state = self.state();
        let lost = state.writebacks_lost;
        state.reported_losses.insert(opened.as_raw_fd(), lost);
        opened
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn naming(&self) -> Option<MutexGuard<'_, ()>> {
        let compares = self.quirk(|quirks| quirks.case_insensitive || quirks.stored_as.is_some());
        compares.then(|| {
            self.naming
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        })
    }

    fn run<T>(
        &self,
        op: Op,
        names: &[&OsStr],
        perform: impl FnOnce(&Self) -> Result<T, Errno>,
    ) -> Result<T, Errno> {
        self.run_logged(op, names, false, perform)
    }

    fn run_logged<T>(
        &self,
        op: Op,
        names: &[&OsStr],
        forced: bool,
        perform: impl FnOnce(&Self) -> Result<T, Errno>,
    ) -> Result<T, Errno> {
        let racer = {
            let mut state = self.state();
            state.calls.push(Call {
                op,
                names: names.iter().map(|name| name.to_os_string()).collect(),
                forced,
            });
            if state.unplugged {
                return Err(Errno::IO);
            }
            take_racer(&mut state.quirks.before, op, names)
        };
        if let Some(mut racer) = racer {
            racer();
        }
        let fault = {
            let mut state = self.state();
            let mut fault = None;
            if let Some(index) = state
                .quirks
                .fail
                .iter()
                .position(|(fail_op, pattern, _, _)| *fail_op == op && matches(pattern, names))
            {
                let (_, _, skip, errno) = &mut state.quirks.fail[index];
                if *skip == 0 {
                    fault = Some(*errno);
                    state.quirks.fail.remove(index);
                } else {
                    *skip -= 1;
                }
            }
            fault
        };
        if let Some(errno) = fault {
            return Err(errno);
        }
        self.reap();
        let result = perform(self);
        {
            let mut state = self.state();
            if result.is_ok() && op.changes() {
                state.own_changes += 1;
            }
            if let Some((unplug_op, pattern, skip)) = state.quirks.unplugged_after.as_mut()
                && *unplug_op == op
                && matches(pattern, names)
            {
                if *skip == 0 {
                    state.quirks.unplugged_after = None;
                    state.unplugged = true;
                } else {
                    *skip -= 1;
                }
            }
        }
        let racer = take_racer(&mut self.state().quirks.after, op, names);
        if let Some(mut racer) = racer {
            racer();
        }
        if result.is_ok() {
            let mut state = self.state();
            if let Some(index) = state
                .quirks
                .lost_reply
                .iter()
                .position(|(lost_op, pattern, _)| *lost_op == op && matches(pattern, names))
            {
                return Err(state.quirks.lost_reply.remove(index).2);
            }
        }
        result
    }

    fn cached(&self, mut stat: EntryStat, forced: bool) -> EntryStat {
        let mut state = self.state();
        let Some(kept_for) = state.quirks.cached_change_times else {
            return stat;
        };
        let now = clock_for_test();
        let seen = state
            .seen_change_times
            .entry((stat.identity.device, stat.identity.inode))
            .or_insert((stat.ctime, now));
        if forced
            || now
                .duration_since(seen.1)
                .map_or(true, |age| age >= kept_for)
        {
            *seen = (stat.ctime, now);
        }
        stat.ctime = seen.0;
        stat
    }

    fn cached_stamp(&self, mut stamp: FolderStamp, forced: bool) -> FolderStamp {
        let mut state = self.state();
        let cached_times = state.quirks.cached_times.map(|times| {
            if stamp.kind == FileType::Directory {
                times.folders
            } else {
                times.files
            }
        });
        let (kept_for, ignores_force) = match (state.quirks.daemon_cache, cached_times) {
            (Some(daemon), _) => (daemon, true),
            (None, Some(kept_for)) => (kept_for, false),
            (None, None) => return stamp,
        };
        let now = state.fetch();
        let fetched = Seen {
            modified: stamp.modified,
            changed: stamp.changed,
            fetch: now,
        };
        let seen = state
            .seen_times
            .entry((stamp.device, stamp.inode))
            .or_insert(fetched);
        if (forced && !ignores_force) || !seen.fetch.kept(kept_for, now) {
            *seen = fetched;
        }
        (stamp.modified, stamp.changed) = (seen.modified, seen.changed);
        stamp
    }

    fn listed(&self, folder: &Path, listing: Listed) -> Listed {
        let mut state = self.state();
        let Some(cached) = state.quirks.cached_listing else {
            return listing;
        };
        let now = state.fetch();
        let entry = state
            .listings
            .entry(folder.to_path_buf())
            .or_insert_with(|| (listing.clone(), now));
        if !entry.1.kept(cached.kept_for, now) {
            *entry = (listing, now);
        } else if cached.renewed_by_use {
            entry.1.at = now.at;
        }
        entry.0.clone()
    }

    fn used(&self, folder: &Path) {
        let mut state = self.state();
        if state
            .quirks
            .cached_listing
            .is_some_and(|cached| cached.renewed_by_use)
            && let Some((_, seen)) = state.listings.get_mut(folder)
        {
            seen.at = clock_for_test();
        }
    }

    fn reported(&self, path: Option<PathBuf>, mut stat: EntryStat) -> EntryStat {
        let key = (stat.identity.device, stat.identity.inode);
        if let Some(device) = path.as_deref().and_then(own_device) {
            stat.identity.device = device;
        }
        if let Some(birth) = stat.identity.birth.filter(|_| stat.is_directory())
            && self.keeps_folder_births_its_base_restamps()
        {
            stat.identity.birth = Some(*first_folder_births().entry(key).or_insert(birth));
        }
        let mut state = self.state();
        let cached = path
            .as_ref()
            .and_then(|path| state.cached.get(path).copied());
        if state.quirks.births == Births::Recorded
            && stat.identity.birth.is_none_or(|birth| birth == (0, 0))
        {
            stat.identity.birth = state.recorded_birth(key);
        }
        if let Some(&birth) = state.reset_births.get(&key).filter(|_| stat.is_directory()) {
            stat.identity.birth = Some(birth);
        }
        if let Some(&(inode, reported)) = path.as_ref().and_then(|path| state.link_names.get(path))
            && inode == stat.identity.inode
        {
            stat.identity.inode = reported;
        }
        if path
            .as_ref()
            .is_some_and(|path| state.renumbered.contains(path))
        {
            stat.identity.inode = !stat.identity.inode;
        }
        if state.quirks.single_link_count {
            stat.nlink = 1;
        }
        if let (Some((pattern, mode)), Some(name)) = (
            state.quirks.reported_mode,
            path.as_deref().and_then(Path::file_name),
        ) && matches(pattern, &[name])
        {
            stat.mode = mode;
        }
        if let (Some(pattern), Some(path)) = (state.quirks.reused_inodes, path)
            && path
                .file_name()
                .is_some_and(|name| matches(pattern, &[name]))
        {
            stat.identity.inode = *state
                .inodes
                .entry(path.clone())
                .or_insert(stat.identity.inode);
            stat.identity.birth = Some((1 + *state.births.entry(path).or_default(), 0));
        }
        if let Some(tick) = state.quirks.coarse_clock.and_then(timespec) {
            stat.mtime = stat.mtime.min(tick);
            stat.ctime = stat.ctime.min(tick);
            stat.identity.birth = stat.identity.birth.map(|birth| birth.min(tick));
        }
        if state.quirks.births == Births::MadeUp && !state.created.contains(&key) {
            let earliest = stat.atime.min(stat.mtime).min(stat.ctime);
            stat.identity.birth = Some(*state.made_up.entry(key).or_insert(earliest));
        }
        if state.quirks.no_birth_time {
            stat.identity.birth = None;
        }
        if state.quirks.ctime_is_mtime {
            stat.ctime = stat.mtime;
        }
        if state.quirks.foreign_group {
            stat.gid = stat.gid.wrapping_add(1);
        }
        if state.quirks.virtual_sizes && stat.kind == FileType::RegularFile {
            stat.size = 0;
        }
        if let Some((inode, birth)) = cached {
            stat.identity.inode = inode;
            stat.identity.birth = birth;
        }
        stat
    }

    fn reported_stamp(&self, path: &Path, stamp: FolderStamp) -> FolderStamp {
        let real = path.canonicalize().ok();
        let stat = EntryStat {
            identity: LocalFileIdentity {
                device: stamp.device,
                inode: stamp.inode,
                kind: stamp.kind,
                birth: stamp.birth,
                handle: None,
            },
            kind: stamp.kind,
            mode: stamp.mode & 0o7777,
            nlink: 1,
            size: stamp.size,
            blocks: 0,
            atime: stamp.modified,
            mtime: stamp.modified,
            ctime: stamp.changed,
            uid: 0,
            gid: 0,
        };
        if let Some(real) = real.as_deref().filter(|_| stat.is_directory()) {
            self.used(real);
        }
        let stat = self.reported(real, stat);
        FolderStamp {
            modified: stat.mtime,
            changed: stat.ctime,
            device: stat.identity.device,
            inode: stat.identity.inode,
            birth: stat.identity.birth,
            mount_id: (!self.quirk(|quirks| quirks.no_mount_ids)).then_some(u64::MAX - self.id),
            ..stamp
        }
    }

    fn mount(&self) -> Option<(MountType, MountSite)> {
        self.run(Op::MountType, &[], |scope| {
            let root = scope.quirk(|quirks| quirks.mount_root).unwrap_or("/");
            let site = MountSite {
                point: scope.root.clone(),
                root: PathBuf::from(root),
            };
            Ok(scope.mount_type().map(|mount_type| (mount_type, site)))
        })
        .ok()
        .flatten()
    }

    fn mark_written(&self, fd: BorrowedFd<'_>) {
        if self.quirk(|quirks| quirks.times_reset_at_close)
            && let Ok(stat) = System.stat_fd(fd)
        {
            self.state()
                .written
                .insert((stat.identity.device, stat.identity.inode));
        }
    }

    fn opened_for_writing(&self, fd: BorrowedFd<'_>) {
        if self.quirk(|quirks| quirks.stale_written_times)
            && let Ok(stat) = System.stat_fd(fd)
        {
            self.state().written_times.insert(
                (stat.identity.device, stat.identity.inode),
                WrittenTime {
                    birth: stat.identity.birth,
                    modified: stat.mtime,
                },
            );
        }
    }

    fn with_written_time(&self, mut stat: EntryStat) -> EntryStat {
        let key = (stat.identity.device, stat.identity.inode);
        if let Some(written) = self.state().written_times.get(&key)
            && written.birth == stat.identity.birth
        {
            stat.mtime = written.modified;
        }
        stat
    }

    fn revalidate_written_time(&self, stat: &EntryStat) {
        let key = (stat.identity.device, stat.identity.inode);
        if let Some(written) = self.state().written_times.get_mut(&key)
            && written.birth == stat.identity.birth
        {
            written.modified = stat.mtime;
        }
    }

    fn closing(&self, fd: BorrowedFd<'_>) {
        let written = System.stat_fd(fd).is_ok_and(|stat| {
            let key = (stat.identity.device, stat.identity.inode);
            let mut state = self.state();
            state.written_times.remove(&key);
            state.written.remove(&key)
        });
        if written {
            let now = rustix::fs::Timespec {
                tv_sec: 0,
                tv_nsec: rustix::fs::UTIME_NOW,
            };
            let _ = rustix::fs::futimens(
                fd,
                &rustix::fs::Timestamps {
                    last_access: now,
                    last_modification: now,
                },
            );
        }
    }

    fn quirk<T>(&self, read: impl FnOnce(&Quirks) -> T) -> T {
        read(&self.state().quirks)
    }

    fn mount_type(&self) -> Option<MountType> {
        self.emulated_or(|| self.host.clone())
    }

    fn keeps_folder_births_its_base_restamps(&self) -> bool {
        self.host
            .as_ref()
            .is_some_and(|base| !base.identity().folder_births)
            && self
                .mount_type()
                .is_some_and(|mount| mount.identity().folder_births)
    }

    fn current_mount_type(&self) -> Option<MountType> {
        self.emulated_or(|| host_mount_type(self.mount_id, &self.root))
    }

    fn emulated_or(&self, host: impl FnOnce() -> Option<MountType>) -> Option<MountType> {
        let serverino_disabled = self.state().serverino_disabled;
        let mut mount = match self.quirk(|quirks| quirks.mount_type) {
            Some((fs_type, super_options)) => MountType {
                fs_type: fs_type.to_owned(),
                super_options: super_options.to_owned(),
            },
            None => host()?,
        };
        if serverino_disabled {
            mount.super_options = mount
                .super_options
                .split(',')
                .filter(|option| *option != "serverino")
                .collect::<Vec<_>>()
                .join(",");
        }
        Some(mount)
    }

    fn mounts(&self) -> MountTable {
        MountTable::parse(
            self.mount_id
                .zip(self.mount_type())
                .map(|(id, mount)| {
                    let (fs_type, options) = (mount.fs_type, mount.super_options);
                    format!("{id} 0 0:0 / / rw - {fs_type} test {options}\n")
                })
                .unwrap_or_default(),
        )
    }

    fn server_numbers(&self) -> bool {
        self.mount_type().is_some_and(|mount| {
            mount.server_numbers()
                || (mount.fs_type == "cifs"
                    && mount
                        .super_options
                        .split(',')
                        .any(|option| option == "posix"))
        })
    }

    fn invalid(&self, name: &OsStr) -> Option<Errno> {
        self.quirk(|quirks| quirks.invalid_name)
            .and_then(|invalid| invalid(name))
    }

    fn too_long(
        &self,
        name: &OsStr,
        limit: impl FnOnce(&Quirks) -> Option<(usize, Errno)>,
    ) -> Option<Errno> {
        self.quirk(limit)
            .filter(|(bytes, _)| name.len() > *bytes)
            .map(|(_, errno)| errno)
    }

    fn refusal(&self, name: &OsStr) -> Option<Errno> {
        self.invalid(name)
            .or_else(|| self.too_long(name, |quirks| quirks.name_limit))
    }

    fn stored<'a>(&self, name: &'a OsStr) -> Cow<'a, OsStr> {
        match self
            .quirk(|quirks| quirks.stored_as)
            .and_then(|stored| stored(name))
        {
            Some(stored) if !stored.is_empty() => Cow::Owned(stored),
            _ => Cow::Borrowed(name),
        }
    }

    /// The other existing name a lookup of `entry` would find: one that
    /// differs in case on a case-insensitive mount, or the name a create of
    /// `entry` stores. Lookups revalidate, so an already case-insensitive
    /// FUSE mount cannot answer from its cache for a name that has moved.
    fn alias(&self, entry: Entry<'_>) -> Option<OsString> {
        let (case_insensitive, stored_as) =
            self.quirk(|quirks| (quirks.case_insensitive, quirks.stored_as.is_some()));
        if !(case_insensitive || stored_as) || System.stat(entry, true).is_ok() {
            return None;
        }
        let stored = self.stored(entry.name);
        if !case_insensitive {
            let found = System
                .stat(
                    Entry {
                        dir: entry.dir,
                        name: &stored,
                    },
                    true,
                )
                .is_ok();
            return (found && stored != entry.name).then(|| stored.into_owned());
        }
        System
            .names(Entry {
                dir: entry.dir,
                name: OsStr::new("."),
            })
            .ok()?
            .into_iter()
            .find(|name| name != entry.name && name.eq_ignore_ascii_case(&stored))
    }

    fn stat(
        &self,
        at: Entry<'_>,
        path: Option<&Path>,
        force_sync: bool,
    ) -> Result<EntryStat, Errno> {
        let (stale_aliases, alias_inodes) =
            self.quirk(|quirks| (quirks.stale_aliases, quirks.alias_inodes));
        let path = path.filter(|_| stale_aliases);
        if let Some(path) = path {
            let mut state = self.state();
            if force_sync {
                state.stale.remove(path);
            } else if let Some(stat) = state.stale.get(path) {
                return Ok(*stat);
            }
        }
        let exact = System.stat(at, force_sync);
        let Some(alias) = self.alias_if_missing(at, &exact) else {
            return exact;
        };
        let mut stat = System.stat(
            Entry {
                dir: at.dir,
                name: &alias,
            },
            force_sync,
        )?;
        if alias_inodes {
            stat.identity.inode = !stat.identity.inode;
        }
        if let Some(path) = path {
            self.state().stale.insert(path.to_path_buf(), stat);
        }
        Ok(stat)
    }

    fn stale(&self, entry: Entry<'_>) -> bool {
        path_of(entry.dir).is_some_and(|dir| self.state().stale.contains_key(&dir.join(entry.name)))
    }

    fn occupied(&self, entry: Entry<'_>) -> bool {
        System.stat(entry, true).is_ok() || self.alias(entry).is_some()
    }

    fn collision(&self) -> Errno {
        if self.quirk(|quirks| quirks.collisions_as_eperm) {
            Errno::PERM
        } else {
            Errno::EXIST
        }
    }

    fn creating<T>(
        &self,
        at: Entry<'_>,
        create: impl FnOnce(Entry<'_>) -> Result<T, Errno>,
    ) -> Result<T, Errno> {
        let _naming = self.naming();
        if let Some(errno) = self.refusal(at.name) {
            return Err(errno);
        }
        if self.alias(at).is_some() {
            return Err(self.collision());
        }
        let stored = self.stored(at.name);
        let stored = Entry {
            dir: at.dir,
            name: &stored,
        };
        let created = create(stored).map_err(|error| {
            if error == Errno::EXIST {
                self.collision()
            } else {
                error
            }
        })?;
        if self.quirk(|quirks| quirks.births == Births::MadeUp)
            && let Ok(stat) = System.stat(stored, false)
        {
            self.state()
                .created
                .insert((stat.identity.device, stat.identity.inode));
        }
        Ok(created)
    }

    fn rename(&self, from: Entry<'_>, to: Entry<'_>, flags: RenameFlags) -> Result<(), Errno> {
        if let Some(folder) = path_of(from.dir).map(|dir| dir.join(from.name))
            && self.refuses_folder_rename(&folder)
        {
            return Err(Errno::ACCESS);
        }
        let _naming = self.naming();
        let (rename_flags, exchange, over_empty) = self.quirk(|quirks| {
            (
                quirks.rename_flags,
                quirks.exchange,
                quirks.rename_over_empty_dir,
            )
        });
        if flags.contains(RenameFlags::NOREPLACE) {
            if self.occupied(to) || self.stale(to) {
                return Err(Errno::EXIST);
            }
            if let Support::Rejected(error) = rename_flags {
                return Err(error);
            }
        }
        if let Some(errno) = self.refusal(to.name) {
            return Err(errno);
        }
        let stored = self.stored(to.name);
        let to = Entry {
            dir: to.dir,
            name: &stored,
        };
        if flags.contains(RenameFlags::EXCHANGE) {
            match exchange {
                Exchange::Supported
                    if System
                        .statfs(to.dir)
                        .is_some_and(|stat| stat.kind == FUSE_SUPER_MAGIC) =>
                {
                    return Err(Errno::INVAL);
                }
                Exchange::Supported => {}
                Exchange::Rejected => return Err(Errno::INVAL),
                Exchange::IgnoredOverwrites => {
                    return System.rename(from, to, RenameFlags::empty());
                }
            }
        }
        if flags.is_empty() {
            let source_is_directory = System
                .stat(from, false)
                .is_ok_and(|stat| stat.is_directory());
            let target_is_empty_directory =
                System.stat(to, false).is_ok_and(|stat| stat.is_directory())
                    && System.names(to).is_ok_and(|names| names.is_empty());
            if source_is_directory && target_is_empty_directory {
                match over_empty {
                    OverEmptyDirectory::Supported => {}
                    OverEmptyDirectory::NotEmpty => return Err(Errno::NOTEMPTY),
                    OverEmptyDirectory::Eperm => return Err(Errno::PERM),
                }
            }
            self.cut_rename_over_name(to)?;
            if let Some(alias) = self.alias(to) {
                return System.rename(
                    from,
                    Entry {
                        dir: to.dir,
                        name: &alias,
                    },
                    flags,
                );
            }
        }
        System.rename(from, to, flags)
    }

    fn cut_rename_over_name(&self, to: Entry<'_>) -> Result<(), Errno> {
        if !self.quirk(|quirks| quirks.cut_rename_over_names) {
            return Ok(());
        }
        let Ok(existing) = System.stat(to, false) else {
            return Ok(());
        };
        let temporary = format!("{}.ntfs-3g-0000000001", to.name.to_string_lossy())
            .encode_utf16()
            .collect::<Vec<_>>();
        if temporary.len() <= 255 {
            return Ok(());
        }
        let stored = match temporary.len() % 256 {
            0 => return Err(Errno::INVAL),
            kept => OsString::from(String::from_utf16_lossy(&temporary[..kept])),
        };
        let stray = Entry {
            dir: to.dir,
            name: &stored,
        };
        let _ = if existing.is_directory() {
            System.mkdir(stray, Mode::from_raw_mode(0o755))
        } else {
            System.link(to, stray)
        };
        Ok(())
    }

    fn link(&self, from: Entry<'_>, to: Entry<'_>) -> Result<(), Errno> {
        let _naming = self.naming();
        if self.invalid(to.name).is_some() {
            return Err(Errno::PERM);
        }
        if let Some(errno) = self.too_long(to.name, |quirks| quirks.name_limit) {
            return Err(errno);
        }
        let stored = self.stored(to.name);
        let to = Entry {
            dir: to.dir,
            name: &stored,
        };
        if self.occupied(to) {
            return Err(self.collision());
        }
        if let Support::Rejected(error) = self.quirk(|quirks| quirks.links) {
            return Err(error);
        }
        System.link(from, to).map_err(|error| {
            if error == Errno::EXIST {
                self.collision()
            } else {
                error
            }
        })?;
        if self.quirk(|quirks| quirks.link_inodes)
            && let (Some(path), Ok(stat)) = (path_of(to.dir), System.stat(to, false))
        {
            let mut state = self.state();
            state.links_made += 1;
            let reported = u64::MAX - state.links_made;
            state
                .link_names
                .insert(path.join(to.name), (stat.identity.inode, reported));
        }
        Ok(())
    }

    fn renamed(&self, from: Entry<'_>, to: Entry<'_>, flags: RenameFlags) {
        if self.quirk(|quirks| quirks.renames_reset_folder_births)
            && let (Ok(folder), Some(now)) = (System.stat_fd(to.dir), timespec(SystemTime::now()))
        {
            self.state()
                .reset_births
                .insert((folder.identity.device, folder.identity.inode), now);
        }
        let (Some(from), Some(to)) = (
            path_of(from.dir).map(|dir| dir.join(from.name)),
            path_of(to.dir).map(|dir| dir.join(to.name)),
        ) else {
            return;
        };
        let mut state = self.state();
        let cached = state.cached.remove(&from);
        state.cached.remove(&to);
        if let Some(cached) = cached {
            state.cached.insert(to.clone(), cached);
        }
        let renumbered = state.renumbered.remove(&from);
        let displaced_renumbered = state.renumbered.remove(&to);
        if renumbered {
            state.renumbered.insert(to.clone());
        }
        if displaced_renumbered && flags.contains(RenameFlags::EXCHANGE) {
            state.renumbered.insert(from.clone());
        }
        let moved = state.link_names.remove(&from);
        let displaced = state.link_names.remove(&to);
        if let Some(moved) = moved {
            state.link_names.insert(to, moved);
        }
        if let Some(displaced) = displaced.filter(|_| flags.contains(RenameFlags::EXCHANGE)) {
            state.link_names.insert(from, displaced);
        }
    }

    fn inode_kept_on_lost_reply(&self, to: Entry<'_>) -> Option<u64> {
        if !self.quirk(|quirks| quirks.lost_renames_keep_inodes) {
            return None;
        }
        System.stat(to, false).ok().map(|stat| stat.identity.inode)
    }

    fn keep_inode(&self, to: Entry<'_>, kept: u64) {
        if let (Some(dir), Ok(stat)) = (path_of(to.dir), System.stat(to, false))
            && stat.identity.inode != kept
        {
            self.state()
                .link_names
                .insert(dir.join(to.name), (stat.identity.inode, kept));
        }
    }

    fn unlinked(&self, at: Entry<'_>) {
        if let Some(path) = path_of(at.dir) {
            let path = path.join(at.name);
            let mut state = self.state();
            state.link_names.remove(&path);
            state.renumbered.remove(&path);
            state.cached.remove(&path);
        }
    }

    fn resolved<T>(
        &self,
        at: Entry<'_>,
        read: impl Fn(Entry<'_>) -> Result<T, Errno>,
    ) -> Result<T, Errno> {
        let exact = read(at);
        match self.alias_if_missing(at, &exact) {
            Some(alias) => read(Entry {
                dir: at.dir,
                name: &alias,
            }),
            None => exact,
        }
    }

    /// The alias of `at` once a call naming it exactly found nothing. A real
    /// case-insensitive lookup finds the entry whichever case it was renamed
    /// to meanwhile, so the exact name is tried first.
    fn alias_if_missing<T>(&self, at: Entry<'_>, exact: &Result<T, Errno>) -> Option<OsString> {
        matches!(exact, Err(Errno::NOENT))
            .then(|| self.alias(at))
            .flatten()
    }
}

fn held_by_this_process(device: u64, inode: u64) -> bool {
    refers_to(device, inode, true)
}

fn refers_to(device: u64, inode: u64, o_path: bool) -> bool {
    let path_only = |fd: &std::fs::DirEntry| {
        let info = format!("/proc/self/fdinfo/{}", fd.file_name().to_string_lossy());
        std::fs::read_to_string(info)
            .ok()
            .and_then(|info| {
                let flags = info.lines().find_map(|line| line.strip_prefix("flags:"))?;
                u32::from_str_radix(flags.trim(), 8).ok()
            })
            .is_some_and(|flags| flags & OFlags::PATH.bits() != 0)
    };
    std::fs::read_dir("/proc/self/fd").is_ok_and(|fds| {
        fds.filter_map(Result::ok).any(|fd| {
            (o_path || !path_only(&fd))
                && rustix::fs::stat(fd.path())
                    .is_ok_and(|open| (open.st_dev, open.st_ino) == (device, inode))
        })
    })
}

impl Aside {
    fn held_here(self, (device, inode): (u64, u64)) -> bool {
        refers_to(device, inode, matches!(self, Self::Nfs))
    }

    fn name(self, inode: u64, count: u64) -> Option<OsString> {
        let name = match self {
            Self::Nfs => format!(".nfs{inode:016x}{count:08x}"),
            Self::Smb => format!(".__smbfile_silly{count:x}"),
            Self::Fuse => format!(".fuse_hidden{:08x}{:08x}", inode as u32, count as u32),
            Self::Unlisted(_) => return None,
        };
        Some(OsString::from(name))
    }
}

impl Scope {
    fn refuses_folder_rename(&self, folder: &Path) -> bool {
        let Some(lingers) = self.quirk(|quirks| quirks.folder_renames_refused_below) else {
            return false;
        };
        if !std::fs::symlink_metadata(folder).is_ok_and(|metadata| metadata.is_dir()) {
            return false;
        }
        let now = clock_for_test();
        let state = self.state();
        let below = |path: &Path| path != folder && path.starts_with(folder);
        state
            .holds
            .keys()
            .filter_map(|key| state.held_paths.get(key))
            .any(|path| below(path))
            || state.closed_holds.iter().any(|(path, closed)| {
                below(path) && now.duration_since(*closed).unwrap_or_default() < lingers
            })
    }

    fn reap(&self) {
        let mut state = self.state();
        let Some(silly) = state.quirks.silly_rename else {
            return;
        };
        let now = clock_for_test();
        let State {
            holds, leftovers, ..
        } = &mut *state;
        leftovers.retain_mut(|leftover| {
            if holds.contains_key(&leftover.key) || silly.aside.held_here(leftover.key) {
                leftover.unheld_since = None;
                return true;
            }
            let since = *leftover.unheld_since.get_or_insert(now);
            if now.duration_since(since).unwrap_or_default() < silly.lingers {
                return true;
            }
            if let Some(name) = &leftover.name {
                let _ = std::fs::remove_file(leftover.folder.join(name));
            }
            false
        });
    }

    fn unlink_held(&self, at: Entry<'_>, flags: AtFlags) -> Option<Result<(), Errno>> {
        let silly = self.quirk(|quirks| quirks.silly_rename)?;
        let folder = path_of(at.dir)?;
        if flags.contains(AtFlags::REMOVEDIR) {
            let Aside::Unlisted(errno) = silly.aside else {
                return None;
            };
            let target = System.stat(at, false).ok()?.identity;
            let held_below = self
                .state()
                .leftovers
                .iter()
                .any(|leftover| leftover.folder_key == (target.device, target.inode));
            return held_below.then_some(Err(errno));
        }
        let stat = System
            .stat(at, false)
            .ok()
            .filter(|stat| stat.kind == FileType::RegularFile)?;
        let key = (stat.identity.device, stat.identity.inode);
        let folder_key = System.stat_fd(at.dir).ok()?.identity;
        let mut state = self.state();
        let leftover = state.leftovers.iter().position(|leftover| {
            leftover.folder == folder && leftover.name.as_deref() == Some(at.name)
        });
        match (leftover, silly.aside) {
            (Some(_), Aside::Nfs) => return Some(Err(Errno::BUSY)),
            (Some(_), Aside::Smb) => return Some(Err(Errno::NOENT)),
            (Some(_), _) => {}
            (None, aside) if !state.holds.contains_key(&key) && !aside.held_here(key) => {
                return None;
            }
            (None, _) => {}
        }
        let name = loop {
            state.silly_renames += 1;
            let name = silly.aside.name(key.1, state.silly_renames);
            let Some(silly_name) = &name else {
                break name;
            };
            match rustix::fs::statat(at.dir, silly_name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => {}
                Err(Errno::NOENT) => break name,
                Err(error) => return Some(Err(error)),
            }
        };
        let unlinked = match &name {
            Some(name) => System.rename(at, Entry { dir: at.dir, name }, RenameFlags::empty()),
            None => System.unlink(at, AtFlags::empty()),
        };
        if unlinked.is_ok() {
            match leftover {
                Some(index) => state.leftovers[index].name = name,
                None => state.leftovers.push(SillyLeftover {
                    folder,
                    folder_key: (folder_key.device, folder_key.inode),
                    name,
                    key,
                    unheld_since: None,
                }),
            }
        }
        Some(unlinked)
    }
}

fn path_of(dir: BorrowedFd<'_>) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{}", dir.as_raw_fd())).ok()
}

fn host_mount_type(mount_id: Option<u64>, root: &Path) -> Option<MountType> {
    let table = MountTable::parse(std::fs::read("/proc/self/mountinfo").ok()?);
    let mount = match mount_id {
        Some(id) => table.mount_of(id),
        None => table.mount_at(root),
    };
    mount.map(|(mount_type, _)| mount_type.clone())
}

/// The basename an fd-only call logs; `#<inode> (deleted)` for an unnamed file.
fn fd_name(fd: BorrowedFd<'_>) -> OsString {
    path_of(fd)
        .and_then(|path| path.file_name().map(OsStr::to_os_string))
        .unwrap_or_default()
}

fn on_fd<T>(
    fd: BorrowedFd<'_>,
    op: Op,
    extra: Option<&OsStr>,
    perform: impl FnOnce(Option<&Scope>) -> Result<T, Errno>,
) -> Result<T, Errno> {
    match scope_for(fd) {
        Some(scope) => {
            let name = fd_name(fd);
            let mut names = vec![name.as_os_str()];
            names.extend(extra);
            scope.run(op, &names, |scope| perform(Some(scope)))
        }
        None => perform(None),
    }
}

/// A data call that moves bytes from `source` into `target`, which fails
/// with `EXDEV` across scopes as it does across mounts.
fn between(
    op: Op,
    source: BorrowedFd<'_>,
    target: BorrowedFd<'_>,
    perform: impl FnOnce() -> Result<usize, Errno>,
) -> Result<usize, Errno> {
    let same = same_scope(&scope_for(source), &scope_for(target));
    on_fd(target, op, None, |scope| {
        if !same {
            return Err(Errno::XDEV);
        }
        if let Some(scope) = scope {
            scope.mark_written(target);
        }
        perform()
    })
}

fn scope_for(dir: BorrowedFd<'_>) -> Option<Arc<Scope>> {
    let path = path_of(dir)?;
    scopes()
        .iter()
        .filter(|scope| path.starts_with(&scope.root))
        .max_by_key(|scope| scope.root.as_os_str().len())
        .cloned()
}

fn timespec(time: SystemTime) -> Option<(i64, u32)> {
    let since = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Some((i64::try_from(since.as_secs()).ok()?, since.subsec_nanos()))
}

fn own_device(path: &Path) -> Option<u64> {
    scopes()
        .iter()
        .filter(|scope| scope.own_device && path.starts_with(&scope.root))
        .max_by_key(|scope| scope.root.as_os_str().len())
        .map(|scope| u64::from(u32::MAX) - scope.id)
}

pub(crate) fn gio_device(file: &gio::File, device: Option<u64>) -> Option<u64> {
    file.path()
        .and_then(|path| own_device(&path.canonicalize().unwrap_or(path)))
        .or(device)
}

fn scope_for_folder(path: &Path) -> Option<Arc<Scope>> {
    let folder = path.canonicalize().ok()?;
    scopes()
        .iter()
        .filter(|scope| folder.starts_with(&scope.root))
        .max_by_key(|scope| scope.root.as_os_str().len())
        .cloned()
}

fn scope_for_path(path: &Path) -> Option<Arc<Scope>> {
    scope_containing(path.parent()?)
}

fn scope_containing(path: &Path) -> Option<Arc<Scope>> {
    let path = path.canonicalize().ok()?;
    scopes()
        .iter()
        .filter(|scope| path.starts_with(&scope.root))
        .max_by_key(|scope| scope.root.as_os_str().len())
        .cloned()
}

pub(crate) fn recreated(path: &Path) {
    let (Some(scope), Some(name)) = (scope_for_path(path), path.file_name()) else {
        return;
    };
    if let Some(parent) = path.parent().and_then(|parent| parent.canonicalize().ok()) {
        *scope.state().births.entry(parent.join(name)).or_default() += 1;
    }
    if let Ok(stat) = rustix::fs::lstat(path) {
        let key = (stat.st_dev, stat.st_ino);
        scope.state().recorded_births.remove(&key);
        first_folder_births().remove(&key);
    }
}

/// Whether a row that sends the malformed `name` as it is to the filesystem holding `root` is
/// skipped there: a FUSE daemon gets what a preset over it passes on, and exfat-fuse writes an
/// overlong `/` into its volume as a real one.
pub(crate) fn skips_malformed_name_on_fuse(root: &Path, name: &OsStr) -> bool {
    let skipped = name.to_str().is_none()
        && rustix::fs::statfs(root).is_ok_and(|stat| stat.f_type == FUSE_SUPER_MAGIC);
    if skipped {
        eprintln!("skipped on a FUSE base, which a malformed name can corrupt: {name:?}");
    }
    skipped
}

/// Makes whatever is at `path` report another inode number until it is
/// unlinked, as libfuse without `use_ino` and cifs without server inode
/// numbers do once the kernel evicts an entry.
pub(crate) fn renumbered(path: &Path) {
    renumber(path);
}

fn renumber(path: &Path) -> bool {
    let (Some(scope), Some(name)) = (scope_for_path(path), path.file_name()) else {
        return false;
    };
    let parent = path.parent().and_then(|parent| parent.canonicalize().ok());
    parent.is_some_and(|parent| {
        scope.state().renumbered.insert(parent.join(name));
        true
    })
}

pub(crate) fn evict(path: &Path) -> bool {
    let Ok(stat) = rustix::fs::lstat(path) else {
        return false;
    };
    let held = held_by_this_process(stat.st_dev, stat.st_ino);
    let Some(scope) = scope_for_path(path).filter(|_| !held) else {
        return false;
    };
    {
        let mut state = scope.state();
        state.made_up.remove(&(stat.st_dev, stat.st_ino));
        if let (Some(parent), Some(name)) = (
            path.parent().and_then(|parent| parent.canonicalize().ok()),
            path.file_name(),
        ) {
            state.cached.remove(&parent.join(name));
        }
    }
    !scope.server_numbers() && renumber(path)
}

pub(crate) fn hold(path: &Path) -> HoldGuard {
    let scope = scope_for_path(path).expect("a scope holding the file");
    let stat = rustix::fs::lstat(path).expect("the held file");
    let key = (stat.st_dev, stat.st_ino);
    let mut state = scope.state();
    *state.holds.entry(key).or_default() += 1;
    if let Ok(path) = path.canonicalize() {
        state.held_paths.insert(key, path);
    }
    drop(state);
    HoldGuard { scope, key }
}

pub(crate) fn autodisable_serverino(path: &Path) -> bool {
    let Some(scope) = scope_containing(path) else {
        return false;
    };
    scope.state().serverino_disabled = true;
    if scope.server_numbers() {
        return true;
    }
    let mut folders = vec![scope.root.clone()];
    while let Some(folder) = folders.pop() {
        let Ok(dir) = std::fs::File::open(&folder) else {
            continue;
        };
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                folders.push(entry.path());
            }
            let name = entry.file_name();
            if let Ok(stat) = System.stat(Entry::new(&dir, &name), false) {
                let stat = scope.reported(Some(entry.path()), stat);
                scope
                    .state()
                    .cached
                    .insert(entry.path(), (stat.identity.inode, stat.identity.birth));
            }
        }
    }
    true
}

fn same_scope(left: &Option<Arc<Scope>>, right: &Option<Arc<Scope>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

pub(crate) struct Scoped;

pub(crate) static SCOPED: Scoped = Scoped;

impl FsEnvironment for Scoped {
    fn can_trash(&self, path: &Path) -> bool {
        assert_off_main_thread();
        match scope_for_path(path) {
            Some(scope) => scope
                .run(Op::CanTrash, &[], |scope| {
                    Ok(match scope.quirk(|quirks| quirks.trash) {
                        Trash::Supported => System.can_trash(path),
                        Trash::Unsupported => false,
                        Trash::FalseWithoutError
                        | Trash::SucceedsWithoutMoving
                        | Trash::Refused
                        | Trash::ProtocolRefused => true,
                    })
                })
                .unwrap_or(false),
            None => System.can_trash(path),
        }
    }

    fn trash(&self, path: &Path, cancellable: &gio::Cancellable) -> Result<bool, glib::Error> {
        assert_off_main_thread();
        let Some(scope) = scope_for_path(path) else {
            return System.trash(path, cancellable);
        };
        let name = path.file_name().unwrap_or_default();
        let mut outcome = Ok(true);
        let ran = scope.run(Op::Trash, &[name], |scope| {
            let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            if scope.refuses_folder_rename(&real) {
                return Err(Errno::ACCESS);
            }
            outcome = match scope.quirk(|quirks| quirks.trash) {
                Trash::Supported => System.trash(path, cancellable),
                Trash::Unsupported | Trash::Refused => Err(glib::Error::new(
                    gio::IOErrorEnum::NotSupported,
                    "Trash is not supported here",
                )),
                Trash::ProtocolRefused => Err(glib::Error::new(
                    gio::IOErrorEnum::Failed,
                    "The backend refused the Trash protocol",
                )),
                Trash::FalseWithoutError => Ok(false),
                Trash::SucceedsWithoutMoving => Ok(true),
            };
            Ok(())
        });
        if let Err(errno) = ran {
            let code = gio::io_error_from_errno(errno.raw_os_error());
            return Err(glib::Error::new(code, &errno.to_string()));
        }
        outcome
    }

    fn mount_type(&self, fd: BorrowedFd<'_>) -> Option<(MountType, MountSite)> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope.mount(),
            None => System.mount_type(fd),
        }
    }

    fn folder_mount(&self, path: &Path, id: Option<u64>) -> Option<(u64, MountType)> {
        assert_off_main_thread();
        let scope = match id {
            Some(id) => scopes()
                .iter()
                .find(|scope| u64::MAX - scope.id == id)
                .cloned(),
            None => scope_containing(path),
        };
        match scope {
            Some(scope) => scope
                .mount()
                .map(|(mount_type, _)| (u64::MAX - scope.id, mount_type)),
            None => System.folder_mount(path, id),
        }
    }

    fn watch(
        &self,
        file: &gio::File,
        watched: Watched,
        flags: gio::FileMonitorFlags,
    ) -> Result<gio::FileMonitor, glib::Error> {
        assert_off_main_thread();
        let path = file.path();
        let Some((scope, path)) = path
            .as_deref()
            .and_then(|path| Some((scope_containing(path)?, path)))
        else {
            return System.watch(file, watched, flags);
        };
        let mut monitor = None;
        let ran = scope.run(Op::Watch, &[path.file_name().unwrap_or_default()], |_| {
            monitor = Some(System.watch(file, watched, flags));
            Ok(())
        });
        match (ran, monitor) {
            (Ok(()), Some(monitor)) => monitor,
            (Err(errno), _) => Err(glib::Error::new(
                gio::io_error_from_errno(errno.raw_os_error()),
                &errno.to_string(),
            )),
            (Ok(()), None) => unreachable!("a call that ran watched the file"),
        }
    }

    fn unwatch(&self, file: &gio::File, monitor: gio::FileMonitor) {
        assert_off_main_thread();
        let path = file.path();
        if let Some((scope, path)) = path
            .as_deref()
            .and_then(|path| Some((scope_containing(path)?, path)))
        {
            let _ = scope.run(Op::Unwatch, &[path.file_name().unwrap_or_default()], |_| {
                Ok(())
            });
        }
        System.unwatch(file, monitor);
    }

    fn current_mount_type(&self, fd: BorrowedFd<'_>) -> Option<MountType> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope
                .run(Op::CurrentMountType, &[], |scope| {
                    Ok(scope.current_mount_type())
                })
                .ok()
                .flatten(),
            None => System.current_mount_type(fd),
        }
    }

    fn file_handle_inputs(
        &self,
        at: Entry<'_>,
        stat: &EntryStat,
    ) -> (MountTable, Option<(i64, u32)>) {
        let Some(scope) = scope_for(at.dir) else {
            return System.file_handle_inputs(at, stat);
        };
        let path = path_of(at.dir).map(|dir| dir.join(at.name));
        let reported = scope.reported(path, scope.cached(*stat, false));
        (scope.mounts(), reported.identity.birth)
    }

    fn file_handle(&self, at: Entry<'_>) -> Result<u64, Errno> {
        let made_up = scope_for(at.dir)
            .and_then(|_| System.current_mount_type(at.dir))
            .filter(|base| gives_no_handles(base) || family(&base.fs_type) == Family::Fuse);
        match made_up {
            Some(base) => made_up_handle(at, &base),
            None => System.file_handle(at),
        }
    }

    fn device_format(&self, fd: BorrowedFd<'_>) -> Result<Option<String>, Errno> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope.run(Op::DeviceFormat, &[], |scope| {
                match scope.quirk(|quirks| (quirks.mount_type, quirks.device_format)) {
                    (None, None) => System.device_format(fd),
                    (_, format) => Ok(format.map(str::to_owned)),
                }
            }),
            None => System.device_format(fd),
        }
    }

    fn rotational(&self, fd: BorrowedFd<'_>) -> Option<bool> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope.quirk(|quirks| quirks.rotational),
            None => System.rotational(fd),
        }
    }

    fn holders(&self, folder: BorrowedFd<'_>, leftovers: &[OsString]) -> Option<Holder> {
        assert_off_main_thread();
        let this_process = || holders::scan(folder, leftovers, &[std::process::id()]);
        match scope_for(folder) {
            Some(scope) => scope
                .run(Op::Holders, &[], |scope| {
                    Ok(scope
                        .quirk(|quirks| quirks.holder.clone())
                        .or_else(this_process))
                })
                .ok()
                .flatten(),
            None => this_process(),
        }
    }
}

pub(crate) fn smb_base(fd: BorrowedFd<'_>) -> bool {
    System
        .current_mount_type(fd)
        .is_some_and(|base| family(&base.fs_type) == Family::Smb)
}

pub(crate) fn handleless_base(fd: BorrowedFd<'_>) -> Option<MountType> {
    System.current_mount_type(fd).filter(gives_no_handles)
}

fn gives_no_handles(base: &MountType) -> bool {
    base.fs_type == "exfat" || family(&base.fs_type) == Family::Smb
}

/// The base's birth time stands for the generation a reused number changes.
fn made_up_handle(at: Entry<'_>, base: &MountType) -> Result<u64, Errno> {
    use std::hash::{Hash, Hasher};
    let flags = if at.name.is_empty() {
        AtFlags::EMPTY_PATH
    } else {
        AtFlags::empty()
    };
    let stat = rustix::fs::statx(
        at.dir,
        at.name,
        flags | AtFlags::SYMLINK_NOFOLLOW,
        StatxFlags::TYPE | StatxFlags::INO | StatxFlags::BTIME,
    )?;
    let folder = FileType::from_raw_mode(u32::from(stat.stx_mode)) == FileType::Directory;
    let birth = (StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::BTIME)
        && (!folder || base.identity().folder_births))
        .then_some((stat.stx_btime.tv_sec, stat.stx_btime.tv_nsec));
    let mut digest = std::hash::DefaultHasher::new();
    (stat.stx_dev_major, stat.stx_dev_minor, stat.stx_ino, birth).hash(&mut digest);
    Ok(digest.finish())
}

/// Links as on Linux 6.10 and later, where `AT_EMPTY_PATH` needs no privilege, whatever the
/// host kernel runs: `ProcOnly` and `Unavailable` quirks stand for the older kernels. Tests
/// otherwise count a failed plain link and a `/proc` retry on some hosts and one call on others.
fn link_fd_as_since_linux_6_10(
    fd: BorrowedFd<'_>,
    to: Entry<'_>,
    through_proc: bool,
) -> Result<(), Errno> {
    match System.link_fd(fd, to, through_proc) {
        Err(Errno::NOENT) if !through_proc => System.link_fd(fd, to, true),
        result => result,
    }
}

impl FsOps for Scoped {
    fn open_at(
        &self,
        at: Entry<'_>,
        flags: OFlags,
        mode: Mode,
        resolve: Option<ResolveFlags>,
    ) -> Result<OwnedFd, Errno> {
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::OpenAt, &[at.name], |scope| {
                scope
                    .resolved(at, |at| System.open_at(at, flags, mode, resolve))
                    .map(|opened| scope.record_open(opened))
            }),
            None => System.open_at(at, flags, mode, resolve),
        }
    }

    fn open_path(&self, path: &OsStr, flags: OFlags, mode: Mode) -> Result<OwnedFd, Errno> {
        let file = std::path::Path::new(path);
        let scope = std::fs::File::open(file)
            .or_else(|error| file.parent().map_or(Err(error), std::fs::File::open))
            .ok()
            .and_then(|opened| scope_for(opened.as_fd()));
        match scope {
            Some(scope) => scope.run(Op::OpenPath, &[path], |scope| {
                System
                    .open_path(path, flags, mode)
                    .map(|opened| scope.record_open(opened))
            }),
            None => System.open_path(path, flags, mode),
        }
    }

    fn dup(&self, fd: BorrowedFd<'_>) -> Result<OwnedFd, Errno> {
        on_fd(fd, Op::Dup, None, |scope| {
            let opened = System.dup(fd)?;
            if let Some(scope) = scope {
                let mut state = scope.state();
                let reported = state
                    .reported_losses
                    .get(&fd.as_raw_fd())
                    .copied()
                    .unwrap_or_default();
                state.reported_losses.insert(opened.as_raw_fd(), reported);
            }
            Ok(opened)
        })
    }

    fn stat_vfs(&self, fd: BorrowedFd<'_>) -> Result<rustix::fs::StatVfs, Errno> {
        on_fd(fd, Op::StatVfs, None, |_| System.stat_vfs(fd))
    }
    fn rename(&self, from: Entry<'_>, to: Entry<'_>, flags: RenameFlags) -> Result<(), Errno> {
        assert_off_main_thread();
        let source = scope_for(from.dir);
        let target = scope_for(to.dir);
        if !same_scope(&source, &target) {
            return Err(Errno::XDEV);
        }
        let op = if flags.contains(RenameFlags::EXCHANGE) {
            Op::Exchange
        } else {
            Op::Rename
        };
        match target {
            Some(scope) => {
                let kept = scope.inode_kept_on_lost_reply(to);
                let renamed = scope.run(op, &[from.name, to.name], |scope| {
                    scope.rename(from, to, flags)?;
                    scope.renamed(from, to, flags);
                    Ok(())
                });
                if let (Err(_), Some(kept)) = (renamed, kept) {
                    scope.keep_inode(to, kept);
                }
                renamed
            }
            None => System.rename(from, to, flags),
        }
    }

    fn link(&self, from: Entry<'_>, to: Entry<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        let source = scope_for(from.dir);
        let target = scope_for(to.dir);
        if !same_scope(&source, &target) {
            return Err(Errno::XDEV);
        }
        match target {
            Some(scope) => scope.run(Op::Link, &[from.name, to.name], |scope| {
                scope.link(from, to)
            }),
            None => System.link(from, to),
        }
    }

    fn unlink(&self, at: Entry<'_>, flags: AtFlags) -> Result<(), Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::Unlink, &[at.name], |scope| {
                if let Some(unlinked) = scope.unlink_held(at, flags) {
                    return unlinked;
                }
                scope.resolved(at, |at| System.unlink(at, flags))?;
                scope.unlinked(at);
                Ok(())
            }),
            None => System.unlink(at, flags),
        }
    }

    fn mkdir(&self, at: Entry<'_>, mode: Mode) -> Result<(), Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::Mkdir, &[at.name], |scope| {
                scope.creating(at, |at| {
                    match scope.too_long(at.name, |quirks| quirks.folder_name_limit) {
                        Some(errno) => Err(errno),
                        None => System.mkdir(at, mode),
                    }
                })
            }),
            None => System.mkdir(at, mode),
        }
    }

    fn symlink(&self, target: &OsStr, at: Entry<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::Symlink, &[at.name], |scope| {
                if let Support::Rejected(error) = scope.quirk(|quirks| quirks.symlinks) {
                    return Err(if scope.occupied(at) {
                        scope.collision()
                    } else {
                        error
                    });
                }
                scope.creating(at, |at| System.symlink(target, at))
            }),
            None => System.symlink(target, at),
        }
    }

    fn readlink(&self, at: Entry<'_>) -> Result<OsString, Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::Readlink, &[at.name], |scope| {
                scope.resolved(at, |at| System.readlink(at))
            }),
            None => System.readlink(at),
        }
    }

    fn stat(&self, at: Entry<'_>, force_sync: bool) -> Result<EntryStat, Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => {
                let path = path_of(at.dir).map(|dir| dir.join(at.name));
                if force_sync
                    && scope.quirk(|quirks| quirks.stale_read_only)
                    && let Some(path) = path.as_deref().and_then(|path| path.canonicalize().ok())
                {
                    scope.state().read_only_unseen.remove(&path);
                }
                scope
                    .run_logged(Op::Stat, &[at.name], force_sync, |scope| {
                        scope.stat(at, path.as_deref(), force_sync)
                    })
                    .map(|stat| {
                        let stat = if force_sync {
                            scope.revalidate_written_time(&stat);
                            stat
                        } else {
                            scope.with_written_time(stat)
                        };
                        scope.reported(path, scope.cached(stat, force_sync))
                    })
            }
            None => System.stat(at, force_sync),
        }
    }

    fn stat_fd(&self, fd: BorrowedFd<'_>) -> Result<EntryStat, Errno> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope
                .run(Op::StatFd, &[&fd_name(fd)], |_| System.stat_fd(fd))
                .map(|stat| {
                    let stat = scope.with_written_time(stat);
                    scope.reported(path_of(fd), scope.cached(stat, false))
                }),
            None => System.stat_fd(fd),
        }
    }

    fn open_parent(&self, dir: BorrowedFd<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match scope_for(dir) {
            Some(scope) => scope.run(Op::OpenParent, &[], |_| System.open_parent(dir)),
            None => System.open_parent(dir),
        }
    }

    fn create_exclusive(&self, at: Entry<'_>, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::Create, &[at.name], |scope| {
                scope
                    .creating(at, |at| System.create_exclusive(at, mode))
                    .inspect(|opened| scope.opened_for_writing(opened.as_fd()))
            }),
            None => System.create_exclusive(at, mode),
        }
    }

    fn names(&self, dir: Entry<'_>) -> Result<Vec<OsString>, Errno> {
        assert_off_main_thread();
        match scope_for(dir.dir) {
            Some(scope) => scope.run(Op::Names, &[dir.name], |_| System.names(dir)),
            None => System.names(dir),
        }
    }

    fn mount_key(&self, dir: BorrowedFd<'_>) -> MountKey {
        assert_off_main_thread();
        match scope_for(dir) {
            Some(scope) => scope
                .run(Op::MountKey, &[], |scope| Ok(MountKey::Test(scope.id)))
                .unwrap_or(MountKey::Test(scope.id)),
            None => System.mount_key(dir),
        }
    }

    fn statfs(&self, dir: BorrowedFd<'_>) -> Option<FsStat> {
        assert_off_main_thread();
        match scope_for(dir) {
            Some(scope) => scope
                .run(Op::Statfs, &[], |scope| {
                    let system = System.statfs(dir);
                    let (kind, name_max) = scope.quirk(|quirks| (quirks.fs_type, quirks.name_max));
                    Ok(kind.or(system.map(|stat| stat.kind)).map(|kind| FsStat {
                        kind,
                        name_max: name_max.or(system.map(|stat| stat.name_max)).unwrap_or(0),
                    }))
                })
                .ok()
                .flatten(),
            None => System.statfs(dir),
        }
    }

    fn open_source(
        &self,
        at: Entry<'_>,
        listed: bool,
    ) -> Result<Option<(OwnedFd, EntryStat)>, Errno> {
        assert_off_main_thread();
        let Some(scope) = scope_for(at.dir) else {
            return System.open_source(at, listed);
        };
        let path = path_of(at.dir).map(|dir| dir.join(at.name));
        let opened = scope.run(Op::OpenSource, &[at.name], |scope| {
            if let Some((pattern, until)) = scope.quirk(|quirks| quirks.deferred_close)
                && matches(pattern, &[at.name])
                && clock_for_test() < until
            {
                return Err(Errno::INVAL);
            }
            let opened = scope.resolved(at, |at| System.open_source(at, listed))?;
            let name_opens = scope.quirk(|quirks| quirks.name_opens);
            if let (Some((fd, stat)), Some(path)) = (&opened, &path)
                && name_opens != NameOpens::Any
            {
                let key = (stat.identity.device, stat.identity.inode);
                let mut state = scope.state();
                if name_opens == NameOpens::OneAtATime {
                    state
                        .opened_names
                        .retain(|&(key, ref name, fd)| still_open(fd, name, key));
                }
                if state
                    .opened_names
                    .iter()
                    .any(|(other, name, _)| *other == key && name != path)
                {
                    return Err(Errno::INVAL);
                }
                state.opened_names.push((key, path.clone(), fd.as_raw_fd()));
            }
            Ok(opened)
        })?;
        Ok(opened.map(|(fd, stat)| (fd, scope.reported(path, scope.cached(stat, false)))))
    }

    fn open_tmpfile(&self, dir: BorrowedFd<'_>, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match scope_for(dir) {
            Some(scope) => scope.run(Op::OpenTmpfile, &[], |scope| {
                match scope.quirk(|quirks| quirks.tmpfile) {
                    Support::Rejected(error) => Err(error),
                    Support::Supported if smb_base(dir) => Err(Errno::OPNOTSUPP),
                    Support::Supported => System.open_tmpfile(dir, mode),
                }
            }),
            None => System.open_tmpfile(dir, mode),
        }
    }

    fn link_fd(&self, fd: BorrowedFd<'_>, to: Entry<'_>, through_proc: bool) -> Result<(), Errno> {
        assert_off_main_thread();
        let target = scope_for(to.dir);
        if !same_scope(&scope_for(fd), &target) {
            return Err(Errno::XDEV);
        }
        let Some(scope) = target else {
            return link_fd_as_since_linux_6_10(fd, to, through_proc);
        };
        scope.run(Op::LinkFd, &[to.name], |scope| {
            scope.creating(to, |to| {
                let (links, tmpfile_link) =
                    scope.quirk(|quirks| (quirks.links, quirks.tmpfile_link));
                if let Support::Rejected(error) = links {
                    return Err(error);
                }
                match (tmpfile_link, through_proc) {
                    (TmpfileLink::ProcOnly, false) | (TmpfileLink::Unavailable, _) => {
                        Err(Errno::NOENT)
                    }
                    _ => link_fd_as_since_linux_6_10(fd, to, through_proc),
                }
            })
        })
    }

    fn reflink(&self, target: BorrowedFd<'_>, source: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        let refused =
            scope_for(target).and_then(|scope| match scope.quirk(|quirks| quirks.reflink) {
                Support::Rejected(error) => Some(error),
                Support::Supported => None,
            });
        between(Op::Clone, source, target, || match refused {
            Some(error) => Err(error),
            None => System.reflink(target, source).map(|()| 0),
        })
        .map(drop)
    }

    fn copy_range(
        &self,
        source: BorrowedFd<'_>,
        source_offset: u64,
        target: BorrowedFd<'_>,
        target_offset: u64,
        len: usize,
    ) -> Result<usize, Errno> {
        assert_off_main_thread();
        let end = scope_for(source).and_then(|scope| {
            scope.quirk(|quirks| {
                if quirks.virtual_sizes {
                    Some(0)
                } else {
                    quirks.copy_range_end
                }
            })
        });
        let len = end.map_or(len, |end| {
            len.min(usize::try_from(end.saturating_sub(source_offset)).unwrap_or(len))
        });
        between(Op::CopyRange, source, target, || {
            if len == 0 {
                return Ok(0);
            }
            System.copy_range(source, source_offset, target, target_offset, len)
        })
    }

    fn read_at(&self, fd: BorrowedFd<'_>, buffer: &mut [u8], offset: u64) -> Result<usize, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::Read, None, |scope| {
            let limit = scope
                .and_then(|scope| scope.quirk(|quirks| quirks.short_io))
                .unwrap_or(buffer.len())
                .min(buffer.len());
            System.read_at(fd, &mut buffer[..limit], offset)
        })
    }

    fn write_at(&self, fd: BorrowedFd<'_>, buffer: &[u8], offset: u64) -> Result<usize, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::Write, None, |scope| {
            let limit = scope
                .and_then(|scope| scope.quirk(|quirks| quirks.short_io))
                .unwrap_or(buffer.len())
                .min(buffer.len());
            if let Some(scope) = scope {
                scope.mark_written(fd);
            }
            System.write_at(fd, &buffer[..limit], offset)
        })
    }

    fn set_len(&self, fd: BorrowedFd<'_>, len: u64) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SetLen, None, |_| System.set_len(fd, len))
    }

    fn seek_data(&self, fd: BorrowedFd<'_>, offset: u64) -> Result<u64, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SeekData, None, |_| System.seek_data(fd, offset))
    }

    fn seek_hole(&self, fd: BorrowedFd<'_>, offset: u64) -> Result<u64, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SeekHole, None, |_| System.seek_hole(fd, offset))
    }

    fn sync_data(&self, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SyncData, None, |_| System.sync_data(fd))
    }

    fn sync_fs(&self, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SyncFs, None, |scope| {
            if let Some(scope) = scope {
                let mut state = scope.state();
                let lost = state.writebacks_lost;
                let reported = state.reported_losses.entry(fd.as_raw_fd()).or_default();
                if lost > *reported {
                    *reported = lost;
                    return Err(Errno::IO);
                }
            }
            System.sync_fs(fd)
        })
    }

    fn sync_dir(&self, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SyncDir, None, |_| System.sync_dir(fd))
    }

    fn close(&self, fd: OwnedFd) -> Result<(), Errno> {
        assert_off_main_thread();
        let Some(scope) = scope_for(fd.as_fd()) else {
            return System.close(fd);
        };
        let name = fd_name(fd.as_fd());
        // A fault leaves `fd` to its drop, which closes it.
        scope.run(Op::Close, &[&name], move |scope| {
            scope.closing(fd.as_fd());
            System.close(fd)
        })
    }

    fn chown_fd(
        &self,
        fd: BorrowedFd<'_>,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::ChownFd, None, |scope| {
            match scope.map(|scope| scope.quirk(|quirks| quirks.chown)) {
                Some(Support::Rejected(error)) => Err(error),
                _ => System.chown_fd(fd, uid, gid),
            }
        })
    }

    fn chmod_fd(&self, fd: BorrowedFd<'_>, mode: u32) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::ChmodFd, None, |scope| {
            if let (Some(scope), Some(path)) = (scope, path_of(fd))
                && scope.quirk(|quirks| quirks.stale_read_only)
                && path.is_dir()
            {
                let mut state = scope.state();
                if mode & 0o200 == 0 {
                    state.read_only_unseen.insert(path);
                } else if state.read_only_unseen.contains(&path) {
                    return Ok(());
                }
            }
            System.chmod_fd(fd, mode)
        })
    }

    fn set_times_fd(
        &self,
        fd: BorrowedFd<'_>,
        atime: (i64, u32),
        mtime: (i64, u32),
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SetTimes, None, |scope| {
            System.set_times_fd(fd, atime, mtime)?;
            if let (Some(scope), Ok(stat)) = (scope, System.stat_fd(fd)) {
                scope.revalidate_written_time(&stat);
            }
            Ok(())
        })
    }

    fn xattr_names(&self, fd: BorrowedFd<'_>) -> Result<Vec<OsString>, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::XattrNames, None, |scope| {
            let extra = scope.map_or_else(Vec::new, |scope| {
                scope.quirk(|quirks| quirks.extra_xattrs.clone())
            });
            let mut names = match System.xattr_names(fd) {
                Ok(names) => names,
                Err(_) if !extra.is_empty() => Vec::new(),
                Err(error) => return Err(error),
            };
            names.extend(extra.iter().map(|(name, _)| OsString::from(name)));
            if scope.is_some_and(|scope| scope.quirk(|quirks| quirks.acls_listed_first)) {
                names.sort_by_key(|name| !name.as_encoded_bytes().starts_with(b"system."));
            }
            Ok(names)
        })
    }

    fn get_xattr(&self, fd: BorrowedFd<'_>, name: &OsStr) -> Result<Vec<u8>, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::GetXattr, Some(name), |scope| {
            let extra = scope.and_then(|scope| {
                scope.quirk(|quirks| {
                    quirks
                        .extra_xattrs
                        .iter()
                        .find(|(extra, _)| OsStr::new(extra) == name)
                        .map(|(_, value)| value.to_vec())
                })
            });
            extra.map_or_else(|| System.get_xattr(fd, name), Ok)
        })
    }

    fn set_xattr(&self, fd: BorrowedFd<'_>, name: &OsStr, value: &[u8]) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SetXattr, Some(name), |scope| {
            let refused = scope.and_then(|scope| {
                scope.quirk(|quirks| {
                    quirks
                        .xattrs_refused
                        .iter()
                        .find(|(prefix, _)| name.as_encoded_bytes().starts_with(prefix.as_bytes()))
                        .map(|(_, error)| *error)
                })
            });
            refused.map_or_else(|| System.set_xattr(fd, name, value), Err)
        })
    }

    fn open_dir(&self, at: Entry<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::OpenDir, &[at.name], |scope| {
                let opened = scope.resolved(at, |at| System.open_dir(at))?;
                Ok(scope.record_open(opened))
            }),
            None => System.open_dir(at),
        }
    }

    fn list(&self, dir: BorrowedFd<'_>) -> Result<Vec<(OsString, FileType)>, Errno> {
        assert_off_main_thread();
        on_fd(dir, Op::List, None, |_| System.list(dir))
    }

    fn list_chunks(
        &self,
        dir: BorrowedFd<'_>,
        each: &mut dyn FnMut(Listed) -> ControlFlow<()>,
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        let Some(scope) = scope_for(dir) else {
            return System.list_chunks(dir, each);
        };
        let name = fd_name(dir);
        let mut listed = Vec::new();
        System.list_chunks(dir, &mut |chunk| {
            listed.extend(chunk);
            ControlFlow::Continue(())
        })?;
        if let Some(folder) = path_of(dir) {
            listed = scope.listed(&folder, listed);
        }
        let size = scope
            .quirk(|quirks| quirks.listing_chunk)
            .unwrap_or(usize::MAX)
            .max(1);
        let mut chunks = listed.chunks(size).map(<[_]>::to_vec);
        loop {
            let chunk = chunks.next();
            scope.run(Op::ListChunks, &[name.as_os_str()], |_| Ok(()))?;
            if chunk.is_none_or(|chunk| each(chunk).is_break()) {
                return Ok(());
            }
        }
    }

    fn folder_stamp(&self, path: &Path, force_sync: bool) -> Result<FolderStamp, Errno> {
        assert_off_main_thread();
        match scope_for_folder(path) {
            Some(scope) => scope
                .run_logged(
                    Op::FolderStamp,
                    &[path.file_name().unwrap_or_default()],
                    force_sync,
                    |_| System.folder_stamp(path, force_sync),
                )
                .map(|stamp| scope.reported_stamp(path, scope.cached_stamp(stamp, force_sync))),
            None => System.folder_stamp(path, force_sync),
        }
    }

    fn access(&self, path: &Path, access: Access) -> Result<(), Errno> {
        assert_off_main_thread();
        match scope_for_folder(path) {
            Some(scope) => scope.run(Op::Access, &[path.file_name().unwrap_or_default()], |_| {
                System.access(path, access)
            }),
            None => System.access(path, access),
        }
    }

    fn create_node(&self, at: Entry<'_>, kind: FileType, mode: Mode) -> Result<(), Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::CreateNode, &[at.name], |scope| {
                let support = scope.quirk(|quirks| match kind {
                    FileType::Fifo => quirks.fifos,
                    _ => quirks.node_create,
                });
                scope.creating(at, |at| match support {
                    Support::Rejected(error) => Err(error),
                    Support::Supported => System.create_node(at, kind, mode),
                })
            }),
            None => System.create_node(at, kind, mode),
        }
    }

    fn open_fifo(&self, at: Entry<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::OpenFifo, &[at.name], |scope| {
                scope.resolved(at, |at| System.open_fifo(at))
            }),
            None => System.open_fifo(at),
        }
    }

    fn create_readonly(&self, at: Entry<'_>, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::CreateReadonly, &[at.name], |scope| {
                scope.creating(at, |at| System.create_readonly(at, mode))
            }),
            None => System.create_readonly(at, mode),
        }
    }

    fn open_write(&self, at: Entry<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::OpenWrite, &[at.name], |scope| {
                scope
                    .resolved(at, |at| System.open_write(at))
                    .inspect(|opened| scope.opened_for_writing(opened.as_fd()))
            }),
            None => System.open_write(at),
        }
    }

    fn file_flags(&self, fd: BorrowedFd<'_>) -> Result<u32, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::FileFlags, None, |_| System.file_flags(fd))
    }

    fn set_link_times(
        &self,
        at: Entry<'_>,
        atime: (i64, u32),
        mtime: (i64, u32),
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => scope.run(Op::SetLinkTimes, &[at.name], |scope| {
                scope.resolved(at, |at| {
                    System.set_link_times(at, atime, mtime)?;
                    if let Ok(stat) = System.stat(at, false) {
                        scope.revalidate_written_time(&stat);
                    }
                    Ok(())
                })
            }),
            None => System.set_link_times(at, atime, mtime),
        }
    }
}

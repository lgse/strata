// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Op {
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
    pub fn changes(self) -> bool {
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
pub enum Support {
    #[default]
    Supported,
    Rejected(Errno),
}

#[derive(Clone, Copy, Debug, Default)]
pub enum Exchange {
    #[default]
    Supported,
    Rejected,
    /// Overwrites the target instead of swapping, as some FUSE daemons do.
    IgnoredOverwrites,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum OverEmptyDirectory {
    #[default]
    Supported,
    NotEmpty,
    Eperm,
    InSteps,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum Trash {
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
pub enum TmpfileLink {
    #[default]
    Any,
    /// `AT_EMPTY_PATH` needs privileges, as before Linux 6.10.
    ProcOnly,
    /// Neither form works, as before 6.10 without `/proc`.
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NameOpens {
    #[default]
    Any,
    FirstOnly,
    /// While another name is open.
    OneAtATime,
}

#[derive(Clone, Copy, Debug)]
pub enum Aside {
    Nfs,
    Smb,
    Fuse,
    Unlisted(Errno),
}

#[derive(Clone, Copy, Debug)]
pub struct Silly {
    pub aside: Aside,
    pub lingers: Duration,
}

impl Silly {
    pub fn nfs() -> Self {
        Self {
            aside: Aside::Nfs,
            lingers: Duration::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CachedTimes {
    pub folders: Duration,
    pub files: Duration,
}

#[derive(Clone, Copy, Debug)]
pub struct CachedListing {
    pub kept_for: Duration,
    pub renewed_by_use: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Births {
    #[default]
    Reported,
    MadeUp,
    /// A birth time per file, kept across renumbering, on a base that reports none (as FAT).
    Recorded,
}

pub type Racer = Box<dyn FnMut() + Send>;
pub type Preset = fn() -> Quirks;

/// Patterns match a name exactly, or by prefix when they end in `*`.
#[derive(Default)]
pub struct Quirks {
    pub rename_flags: Support,
    pub exchange: Exchange,
    pub links: Support,
    pub collisions_as_eperm: bool,
    pub rename_over_empty_dir: OverEmptyDirectory,
    pub cut_rename_over_names: bool,
    pub case_insensitive: bool,
    pub stale_aliases: bool,
    /// A case alias reports its own inode number, as libfuse without `use_ino` does.
    pub alias_inodes: bool,
    pub link_inodes: bool,
    /// Every entry reports one link, as sshfs does, whose SFTP attributes have none.
    pub single_link_count: bool,
    pub lost_renames_keep_inodes: bool,
    pub invalid_name: Option<fn(&OsStr) -> Option<Errno>>,
    pub stored_as: Option<fn(&OsStr) -> Option<OsString>>,
    pub name_limit: Option<(usize, Errno)>,
    pub folder_name_limit: Option<(usize, Errno)>,
    pub symlinks: Support,
    pub fs_type: Option<FsWord>,
    pub name_max: Option<u32>,
    pub trash: Trash,
    pub reused_inodes: Option<&'static str>,
    /// As on NFSv3, ntfs-3g and exfat-fuse.
    pub no_birth_time: bool,
    pub births: Births,
    pub ctime_is_mtime: bool,
    pub renames_reset_folder_births: bool,
    /// Performs the call, then reports this errno once.
    pub lost_reply: Vec<(Op, &'static str, Errno)>,
    /// Fails the matching call after skipping the given number of matches, once.
    pub fail: Vec<(Op, &'static str, usize, Errno)>,
    /// Runs once, before the matching call after skipping the given number of matches.
    pub before: Vec<(Op, &'static str, usize, Racer)>,
    pub after: Vec<(Op, &'static str, usize, Racer)>,
    pub skip_forced_stamps: bool,
    pub unplugged_after: Option<(Op, &'static str, usize)>,
    pub mount_type: Option<(&'static str, &'static str)>,
    pub mount_root: Option<&'static str>,
    pub device_format: Option<&'static str>,
    pub rotational: Option<bool>,
    pub tmpfile: Support,
    pub tmpfile_link: TmpfileLink,
    /// The most bytes one read or write moves.
    pub short_io: Option<usize>,
    pub times_reset_at_close: bool,
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
    pub virtual_sizes: bool,
    /// `FICLONE`, which btrfs and XFS hosts would otherwise perform.
    pub reflink: Support,
    pub copy_range_end: Option<u64>,
    pub reported_mode: Option<(&'static str, u32)>,
    pub name_opens: NameOpens,
    pub deferred_close: Option<(&'static str, SystemTime)>,
    pub stale_read_only: bool,
    pub acls_listed_first: bool,
    pub own_device: bool,
    pub no_mount_ids: bool,
    pub cached_change_times: Option<Duration>,
    pub cached_times: Option<CachedTimes>,
    pub daemon_cache: Option<Duration>,
    pub cached_listing: Option<CachedListing>,
    pub coarse_clock: Option<SystemTime>,
    pub listing_chunk: Option<usize>,
}

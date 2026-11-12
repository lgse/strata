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

/// Opening a name of a file fails with `EINVAL` where another name of it
/// is in the way, as the SMB client's one lease per file makes a second
/// open fail, or a Samba server defer it forever.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NameOpens {
    #[default]
    Any,
    /// Once another name was opened at all, which pins that a copy never
    /// opens a later name.
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
    /// `fail`, `before` and `after` pass over forced stamps, which a browser's
    /// background freshness checks make, so they reach only the call under test.
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

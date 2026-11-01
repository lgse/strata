// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use super::names::{NameEncoding, NameProfile, NameRules, PrivateUse, Reserved, Strip, byte_limit};
use crate::{
    model::hidden_names::ForeignLeftover,
    services::{IdentityTraits, InodeNumbers},
};

/// Parsed without touching mounted filesystems, including unresponsive mounts.
#[derive(Clone)]
pub(crate) struct MountTable {
    entries: Arc<[MountEntry]>,
}

struct MountEntry {
    id: u64,
    device: (u32, u32),
    site: MountSite,
    mount_type: MountType,
    source: Option<PathBuf>,
}

impl MountEntry {
    fn parts(&self) -> (&MountType, &MountSite) {
        (&self.mount_type, &self.site)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MountSite {
    pub point: PathBuf,
    pub root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MountType {
    pub fs_type: String,
    pub super_options: String,
}

impl MountType {
    /// FAT and exFAT, libfuse without `use_ino` and cifs without server inode numbers number an
    /// entry afresh once the kernel has evicted it, and vboxsf keeps a name's number for whatever
    /// a rename puts there. The kernel's exFAT driver stamps a folder's birth time at each rename
    /// into or within it.
    pub(crate) fn identity(&self) -> IdentityTraits {
        let smb = family(&self.fs_type) == Family::Smb;
        let numbers = match (family(&self.fs_type), self.fs_type.as_str()) {
            (Family::Fat | Family::Fuse | Family::Smb, _) | (_, "vboxsf") => InodeNumbers::Renewed,
            (
                Family::Other,
                "9p" | "virtiofs" | "ecryptfs" | "ceph" | "hfs" | "hfsplus" | "ntfs" | "ramfs",
            ) => InodeNumbers::Kept,
            (Family::Other, _) => InodeNumbers::Unknown,
            _ => InodeNumbers::Kept,
        };
        IdentityTraits {
            numbers,
            folder_births: self.fs_type != "exfat",
            made_up_births: smb,
        }
    }

    pub(crate) fn wants_file_handle(&self, birth: Option<(i64, u32)>) -> bool {
        matches!(
            family(&self.fs_type),
            Family::Ext | Family::Nfs | Family::Nfs4
        ) && birth.is_none_or(|birth| birth == (0, 0))
    }

    pub(crate) fn server_numbers(&self) -> bool {
        family(&self.fs_type) == Family::Smb
            && self
                .super_options
                .split(',')
                .any(|option| option == "serverino")
    }

    /// How long the SMB client keeps a closed file's handle open
    /// (`closetimeo`, one second by default).
    pub(crate) fn deferred_close(&self) -> Option<Duration> {
        if family(&self.fs_type) != Family::Smb {
            return None;
        }
        let seconds = self
            .super_options
            .split(',')
            .find_map(|option| option.strip_prefix("closetimeo="))
            .and_then(|seconds| seconds.parse().ok())
            .unwrap_or(1);
        Some(Duration::from_secs(seconds))
    }

    pub(crate) fn silly_names(&self) -> Option<SillyNames> {
        match family(&self.fs_type) {
            Family::Nfs | Family::Nfs4 => Some(SillyNames::Renamed(ForeignLeftover::Nfs)),
            Family::Fuse => Some(SillyNames::Renamed(ForeignLeftover::Fuse)),
            Family::Smb if self.smb_posix() => Some(SillyNames::Unlisted),
            Family::Smb => Some(SillyNames::Renamed(ForeignLeftover::Smb)),
            _ => None,
        }
    }

    fn smb_posix(&self) -> bool {
        let has = |name| self.super_options.split(',').any(|option| option == name);
        has("posix") || has("unix") && has("posixpaths")
    }

    /// How long the SMB client answers stats from the attributes it cached
    /// for a file (`acregmax`, shown as `actimeo` when folders share it, one
    /// second by default); `None` where nothing is cached.
    pub(crate) fn attribute_cache(&self) -> Option<Duration> {
        if family(&self.fs_type) != Family::Smb {
            return None;
        }
        let option = |name: &str| {
            self.super_options
                .split(',')
                .find_map(|option| option.strip_prefix(name)?.strip_prefix('='))
                .and_then(|seconds| seconds.parse().ok())
        };
        let seconds = option("acregmax")
            .or_else(|| option("actimeo"))
            .unwrap_or(1);
        (seconds > 0).then(|| Duration::from_secs(seconds))
    }

    /// Whether a rename over an existing entry stores a name longer than
    /// [`LONGEST_NAME_NTFS_3G_RENAMES_OVER`] cut. A `fuseblk` mount whose device udev cannot
    /// tell may be ntfs-3g.
    pub(crate) fn cuts_long_names_renamed_over(&self, device_format: Option<&str>) -> bool {
        self.fs_type == "fuseblk" && matches!(device_format, Some("ntfs") | None)
    }

    /// A `fuseblk` mount whose device udev cannot tell may be exFAT.
    pub(crate) fn coarse_times(&self, fuse_format: Option<FuseFormat>) -> Option<CoarseTimes> {
        let here = |seconds| CoarseTimes {
            window: Duration::from_secs(seconds),
            stamped_here: true,
        };
        match (family(&self.fs_type), self.fs_type.as_str(), fuse_format) {
            (Family::Fat, ..) | (_, "fuseblk", Some(FuseFormat::Exfat) | None) => Some(here(2)),
            (_, "hfs" | "hfsplus", _) => Some(here(1)),
            // A one-second tick, and two seconds for the server's clock
            // running behind ours. A server further behind can still hide
            // a rewrite.
            (_, "fuse.sshfs", _) => Some(CoarseTimes {
                window: Duration::from_secs(3),
                stamped_here: false,
            }),
            _ => None,
        }
    }

    /// `name_max` is the mount's `f_namelen`, which only byte-counting filesystems mean. A
    /// `fuseblk` mount whose device udev cannot tell is taken for a Windows format, as for
    /// `coarse_times`.
    pub(crate) fn name_profile(&self, device_format: Option<&str>, name_max: u32) -> NameProfile {
        let has = |name| self.super_options.split(',').any(|option| option == name);
        let bytes = byte_limit(name_max);
        if fat_format(&self.fs_type, device_format).is_some() {
            return NameProfile {
                folds: true,
                ..NameProfile::windows(Reserved::Bare, self.fat_encoding())
            };
        }
        match (self.fs_type.as_str(), device_format) {
            ("ntfs3" | "ntfs", _) if has("windows_names") => {
                NameProfile::windows(Reserved::Full, NameEncoding::Utf8)
            }
            ("ntfs3" | "ntfs", _) => NameProfile::windows(Reserved::Bare, NameEncoding::Utf8),
            // udisks mounts ntfs-3g with `windows_names`, which mountinfo hides.
            ("fuseblk", Some("ntfs")) => NameProfile::windows(Reserved::Full, NameEncoding::Utf8),
            ("fuseblk", _) => NameProfile::windows(Reserved::Bare, NameEncoding::Utf8).learning(),
            ("cifs" | "smb3", _) if self.smb_posix() => NameProfile::smb_posix(bytes),
            ("cifs" | "smb3", _) => NameProfile {
                encoding: if has("nocase") {
                    NameEncoding::Utf8Bmp
                } else {
                    NameEncoding::Utf8
                },
                mapped: if has("mapposix") {
                    Some(PrivateUse::Sfm)
                } else if has("mapchars") {
                    Some(PrivateUse::Sfu)
                } else {
                    None
                },
                ..NameProfile::smb(bytes)
            },
            // Counted in UTF-16 units like the Windows formats, without their rules.
            ("hfsplus", _) => NameProfile {
                rules: NameRules::Posix,
                strip: Strip::None,
                ..NameProfile::windows(Reserved::None, NameEncoding::Utf8)
            },
            (fs_type, _) => match family(fs_type) {
                Family::Ext
                | Family::Xfs
                | Family::Btrfs
                | Family::F2fs
                | Family::Tmpfs
                | Family::Bcachefs
                | Family::Zfs
                | Family::Overlay
                | Family::Nfs
                | Family::Nfs4 => NameProfile::posix(bytes),
                _ => NameProfile::unknown(name_max),
            },
        }
    }

    /// Without `utf8`, vfat converts names through its I/O charset, and so
    /// does exFAT unless that is UTF-8. Code pages other than ASCII store
    /// other bytes on vfat and refuse them on exFAT, and are not supported.
    fn fat_encoding(&self) -> NameEncoding {
        let has = |name| self.super_options.split(',').any(|option| option == name);
        let iocharset = self
            .super_options
            .split(',')
            .find_map(|option| option.strip_prefix("iocharset="));
        match (self.fs_type.as_str(), iocharset) {
            ("vfat", Some("utf8")) if !has("utf8") => NameEncoding::Utf8Bmp,
            ("vfat" | "exfat", Some("ascii")) if !has("utf8") => NameEncoding::Ascii,
            _ => NameEncoding::Utf8,
        }
    }

    pub(crate) fn is_gvfs_bridge(&self) -> bool {
        self.fs_type == "fuse.gvfsd-fuse"
    }

    pub(crate) fn outside_writers(&self) -> bool {
        self.fs_type != "fuseblk" && is_remote_fs_type(&self.fs_type)
    }

    pub(crate) fn daemon_cache(&self) -> Option<Duration> {
        (self.fs_type == "fuse.sshfs").then_some(SSHFS_DAEMON_CACHE)
    }
}

pub(crate) const SSHFS_DAEMON_CACHE: Duration = Duration::from_secs(21);

pub(crate) const FAT32_MAX_FILE_SIZE: u64 = u32::MAX as u64;

/// In UTF-16 units: ntfs-3g renames over an existing entry through
/// `<name>.ntfs-3g-<ten digits>`, which it stores cut past 255 units.
pub(crate) const LONGEST_NAME_NTFS_3G_RENAMES_OVER: usize = 236;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FatFormat {
    Fat,
    Exfat,
}

fn fat_format(fs_type: &str, device_format: Option<&str>) -> Option<FatFormat> {
    match (fs_type, device_format) {
        ("vfat" | "msdos", _) | ("fuseblk", Some("vfat")) => Some(FatFormat::Fat),
        ("exfat", _) | ("fuseblk", Some("exfat")) => Some(FatFormat::Exfat),
        _ => None,
    }
}

/// Drivers without symbolic links: the kernel's FAT and exFAT have no
/// `symlink`, and exfat-fuse implements none.
pub(crate) fn holds_symlinks(fs_type: &str, device_format: Option<&str>) -> Option<bool> {
    fat_format(fs_type, device_format).map(|_| false)
}

pub(crate) fn max_file_size(fs_type: &str, device_format: Option<&str>) -> Option<u64> {
    (fat_format(fs_type, device_format) == Some(FatFormat::Fat)).then_some(FAT32_MAX_FILE_SIZE)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SillyNames {
    Renamed(ForeignLeftover),
    Unlisted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoarseTimes {
    pub window: Duration,
    /// Whether this machine's clock stamps them, as on a local disk,
    /// rather than a server's.
    pub stamped_here: bool,
}

impl CoarseTimes {
    /// A time with no fraction of a second may come from a filesystem as coarse as FAT's that the
    /// mount does not show, such as one a server shares.
    pub(crate) fn whole_second(nanoseconds: u32) -> Option<Self> {
        (nanoseconds == 0).then_some(Self {
            window: Duration::from_secs(2),
            stamped_here: false,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FuseFormat {
    Ntfs,
    Exfat,
}

impl FuseFormat {
    pub(crate) fn of(id_fs_type: &str) -> Option<Self> {
        match id_fs_type {
            "ntfs" => Some(Self::Ntfs),
            "exfat" => Some(Self::Exfat),
            _ => None,
        }
    }
}

/// Filesystems by their mountinfo type, which tells NFS versions and FUSE
/// apart where the `statfs` magic cannot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Family {
    Ext,
    Xfs,
    Btrfs,
    F2fs,
    Tmpfs,
    Bcachefs,
    Zfs,
    Overlay,
    /// vfat, msdos and the kernel's exFAT.
    Fat,
    Ntfs3,
    Nfs,
    Nfs4,
    Smb,
    Fuse,
    Other,
}

impl Family {
    /// Where a stat cannot tell a rewrite within one tick of the clock that
    /// stamps times: the kernel's coarse clock, except for the multigrain
    /// timestamps ext4, XFS, btrfs and tmpfs get from Linux 6.13, or a
    /// server's or a daemon's.
    pub(crate) fn coarse_clock(self, kernel: (u32, u32, u32)) -> Option<CoarseTimes> {
        let multigrain = matches!(self, Self::Ext | Self::Xfs | Self::Btrfs | Self::Tmpfs);
        if multigrain && kernel >= (6, 13, 0) {
            return None;
        }
        Some(CoarseTimes {
            window: Duration::ZERO,
            stamped_here: !matches!(
                self,
                Self::Nfs | Self::Nfs4 | Self::Smb | Self::Fuse | Self::Other
            ),
        })
    }
}

pub(crate) fn family(fs_type: &str) -> Family {
    match fs_type {
        "ext2" | "ext3" | "ext4" => Family::Ext,
        "xfs" => Family::Xfs,
        "btrfs" => Family::Btrfs,
        "f2fs" => Family::F2fs,
        "tmpfs" => Family::Tmpfs,
        "bcachefs" => Family::Bcachefs,
        "zfs" => Family::Zfs,
        "overlay" => Family::Overlay,
        "vfat" | "msdos" | "exfat" => Family::Fat,
        "ntfs3" => Family::Ntfs3,
        "nfs" => Family::Nfs,
        "nfs4" => Family::Nfs4,
        "cifs" | "smb3" => Family::Smb,
        "fuse" | "fuseblk" => Family::Fuse,
        fs_type if fs_type.starts_with("fuse.") => Family::Fuse,
        _ => Family::Other,
    }
}

/// One mount instance. A remount gets a new kernel mount ID; the device and
/// mount point guard against a reused ID matching a stale key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MountKey {
    pub(crate) mount_id: u64,
    pub(crate) device: (u32, u32),
    pub(crate) mount_point: PathBuf,
    pub(crate) fs_type: String,
}

#[cfg(test)]
static MOUNTINFO_FOR_TEST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Replaces `/proc/self/mountinfo` for the whole process, so only a test that
/// runs in its own process may use it.
#[cfg(test)]
pub(crate) fn set_mountinfo_for_isolated_test(mountinfo: Option<String>) {
    assert!(
        std::env::var_os("STRATA_ISOLATED_GTK_TEST").is_some(),
        "the mount table override needs an isolated test process"
    );
    *MOUNTINFO_FOR_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = mountinfo;
}

#[cfg(test)]
pub(super) fn mountinfo_for_test() -> Option<String> {
    MOUNTINFO_FOR_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

impl MountTable {
    pub(crate) fn current() -> Self {
        crate::util::ui_thread::assert_off();
        #[cfg(test)]
        if let Some(mountinfo) = mountinfo_for_test() {
            return Self::parse(mountinfo);
        }
        std::fs::read("/proc/self/mountinfo")
            .map(Self::parse)
            .unwrap_or_else(|_| Self::parse(b""))
    }

    pub(crate) fn parse(mountinfo: impl AsRef<[u8]>) -> Self {
        let entries = mountinfo
            .as_ref()
            .split(|byte| *byte == b'\n')
            .filter_map(|line| {
                let separator = line.windows(3).position(|bytes| bytes == b" - ")?;
                let (front, back) = (&line[..separator], &line[separator + 3..]);
                let mut fields = front.split(|byte| *byte == b' ');
                let id = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
                let (major, minor) = std::str::from_utf8(fields.nth(1)?).ok()?.split_once(':')?;
                let device = (major.parse().ok()?, minor.parse().ok()?);
                let root = fields.next()?;
                let mount_point = fields.next()?;
                let mut back = back.split(|byte| *byte == b' ');
                let fs_type = std::str::from_utf8(back.next()?).ok()?;
                let source = back.next().map(|source| PathBuf::from(unescape(source)));
                let super_options = back
                    .next()
                    .and_then(|options| std::str::from_utf8(options).ok())
                    .unwrap_or_default();
                Some(MountEntry {
                    id,
                    device,
                    site: MountSite {
                        point: PathBuf::from(unescape(mount_point)),
                        root: PathBuf::from(unescape(root)),
                    },
                    mount_type: MountType {
                        fs_type: fs_type.to_owned(),
                        super_options: super_options.to_owned(),
                    },
                    source,
                })
            })
            .collect::<Vec<_>>();
        Self {
            entries: entries.into(),
        }
    }

    pub(crate) fn fs_type_for(&self, path: &Path) -> Option<&str> {
        self.innermost(path)
            .map(|entry| entry.mount_type.fs_type.as_str())
    }

    pub(crate) fn mount_of(&self, id: u64) -> Option<(&MountType, &MountSite)> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(MountEntry::parts)
    }

    pub(crate) fn wants_file_handle(&self, id: Option<u64>, birth: Option<(i64, u32)>) -> bool {
        id.and_then(|id| self.mount_of(id))
            .is_some_and(|(mount, _)| mount.wants_file_handle(birth))
    }

    pub(crate) fn points(&self) -> impl Iterator<Item = (u64, &Path)> {
        self.entries
            .iter()
            .map(|entry| (entry.id, entry.site.point.as_path()))
    }

    pub(crate) fn mount_at(&self, path: &Path) -> Option<(&MountType, &MountSite)> {
        self.innermost(path).map(MountEntry::parts)
    }

    pub(crate) fn mount_for(&self, path: &Path) -> Option<MountKey> {
        self.innermost(path).map(|entry| MountKey {
            mount_id: entry.id,
            device: entry.device,
            mount_point: entry.site.point.clone(),
            fs_type: entry.mount_type.fs_type.clone(),
        })
    }

    /// Remote filesystems other than FUSE drivers for local block devices,
    /// such as ntfs-3g and exfat-fuse on a USB drive.
    pub(crate) fn is_network_path(&self, path: &Path) -> bool {
        self.innermost(path)
            .is_some_and(|entry| entry.mount_type.outside_writers())
    }

    pub(crate) fn has_mount_inside(&self, directory: &Path) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.site.point.parent() == Some(directory))
    }

    pub(crate) fn is_mount_point(&self, path: &Path) -> bool {
        self.entries.iter().any(|entry| entry.site.point == path)
    }

    pub(crate) fn sources(&self) -> impl Iterator<Item = (&Path, &Path)> {
        self.entries
            .iter()
            .filter_map(|entry| Some((entry.site.point.as_path(), entry.source.as_deref()?)))
    }

    pub(crate) fn source_for(&self, path: &Path) -> Option<&Path> {
        self.innermost(path)?.source.as_deref()
    }

    pub(crate) fn mount_point_for(&self, path: &Path) -> Option<&Path> {
        self.innermost(path).map(|entry| entry.site.point.as_path())
    }

    /// Excludes virtual or potentially blocking filesystems from fallback scans.
    pub(crate) fn trash_scan_mounts(&self) -> impl Iterator<Item = &Path> {
        self.entries.iter().filter_map(|entry| {
            is_trash_scan_fs_type(&entry.mount_type.fs_type).then_some(entry.site.point.as_path())
        })
    }

    pub(crate) fn non_virtual_mounts(&self) -> impl Iterator<Item = &Path> {
        self.entries.iter().filter_map(|entry| {
            (!is_virtual_fs_type(&entry.mount_type.fs_type)).then_some(entry.site.point.as_path())
        })
    }

    fn innermost(&self, path: &Path) -> Option<&MountEntry> {
        self.entries
            .iter()
            .filter(|entry| path.starts_with(&entry.site.point))
            .max_by_key(|entry| entry.site.point.as_os_str().len())
    }
}

/// Includes `fuseblk`, the FUSE drivers for local block devices.
pub(crate) fn is_fuse_fs_type(fs_type: &str) -> bool {
    fs_type.starts_with("fuse")
}

/// FUSE filesystems that layer or encrypt local storage rather than reach a
/// remote provider.
pub(crate) fn is_local_fuse_overlay(fs_type: &str) -> bool {
    matches!(
        fs_type.strip_prefix("fuse."),
        Some(
            "mergerfs"
                | "gocryptfs"
                | "encfs"
                | "cryfs"
                | "securefs"
                | "bindfs"
                | "fuse-overlayfs"
                | "unionfs"
                | "unionfs-fuse"
                | "mhddfs"
                | "squashfuse"
                | "archivemount"
                | "fuse-zip"
                | "ciopfs"
        )
    )
}

fn is_trash_scan_fs_type(fs_type: &str) -> bool {
    !is_remote_fs_type(fs_type) && !is_virtual_fs_type(fs_type)
}

fn is_virtual_fs_type(fs_type: &str) -> bool {
    matches!(
        fs_type,
        "proc"
            | "sysfs"
            | "devtmpfs"
            | "tmpfs"
            | "cgroup"
            | "cgroup2"
            | "overlay"
            | "autofs"
            | "securityfs"
            | "debugfs"
            | "tracefs"
            | "ramfs"
            | "hugetlbfs"
            | "mqueue"
            | "bpf"
            | "pstore"
            | "configfs"
            | "fusectl"
            | "rpc_pipefs"
            | "nsfs"
            | "binfmt_misc"
            | "devpts"
            | "efivarfs"
    )
}

/// Includes FUSE because an unavailable userspace daemon can block filesystem calls.
pub(super) fn is_remote_fs_type(fs_type: &str) -> bool {
    matches!(
        fs_type,
        "nfs"
            | "nfs4"
            | "cifs"
            | "smb"
            | "smb2"
            | "smb3"
            | "smbfs"
            | "ncpfs"
            | "afs"
            | "afp"
            | "9p"
            | "ceph"
            | "coda"
            | "davfs"
            | "glusterfs"
            | "gfs"
            | "gfs2"
            | "lustre"
            | "ocfs2"
            | "webdav"
            | "vboxsf"
            | "virtiofs"
    ) || is_fuse_fs_type(fs_type)
}

/// Decodes mountinfo's octal escapes without assuming pathname bytes are UTF-8.
fn unescape(field: impl AsRef<[u8]>) -> OsString {
    let bytes = field.as_ref();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let octal = (bytes[index] == b'\\' && index + 3 < bytes.len())
            .then(|| &bytes[index + 1..index + 4])
            .filter(|digits| digits.iter().all(|digit| (b'0'..=b'7').contains(digit)))
            .map(|digits| {
                digits
                    .iter()
                    .fold(0u32, |code, digit| code * 8 + u32::from(digit - b'0'))
            });
        match octal.and_then(|code| u8::try_from(code).ok()) {
            Some(byte) => {
                out.push(byte);
                index += 4;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    OsString::from_vec(out)
}

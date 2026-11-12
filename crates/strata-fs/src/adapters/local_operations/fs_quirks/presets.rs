// SPDX-License-Identifier: MIT

use super::*;

impl Quirks {
    pub fn without_rename_flags() -> Self {
        Self {
            rename_flags: Support::Rejected(Errno::INVAL),
            ..Self::default()
        }
    }

    /// No `RENAME_NOREPLACE`, no hard links, and no rename over an empty folder.
    pub fn vboxsf() -> Self {
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
    pub fn without_folder_placement() -> Self {
        Self {
            rename_over_empty_dir: OverEmptyDirectory::NotEmpty,
            ..Self::without_rename_flags()
        }
    }

    /// No rung that cannot replace: no `RENAME_NOREPLACE` or rename over an
    /// empty folder, and links fail with an errno that says nothing about the
    /// mount, so the reserved-name rung stays unconfirmed.
    pub fn without_placement() -> Self {
        Self {
            links: Support::Rejected(Errno::MLINK),
            ..Self::without_folder_placement()
        }
    }

    pub fn ext4() -> Self {
        Self::default().on_ext4()
    }

    pub fn on_ext4(self) -> Self {
        Self {
            mount_type: Some(EXT4),
            fs_type: Some(EXT4_SUPER_MAGIC),
            ..self
        }
    }

    /// NOREPLACE fails with `EINVAL`, and a write delegation sets the
    /// times of a written file when it is closed.
    pub fn nfs42() -> Self {
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

    pub fn nfs3() -> Self {
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
    pub fn vfat() -> Self {
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

    pub fn exfat() -> Self {
        Self {
            mount_type: Some(("exfat", EXFAT_OPTIONS)),
            fs_type: Some(0x2011_bab0),
            invalid_name: Some(exfat_refuses),
            renames_reset_folder_births: true,
            ..Self::vfat()
        }
    }

    pub fn exfat_fuse() -> Self {
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

    pub fn ntfs3() -> Self {
        Self {
            mount_type: Some(("ntfs3", NTFS3_OPTIONS)),
            fs_type: Some(0x7366_746e),
            name_max: Some(255),
            invalid_name: Some(ntfs3_refuses),
            ..Self::default()
        }
    }

    pub fn ntfs3_windows_names() -> Self {
        Self {
            mount_type: Some(("ntfs3", NTFS3_WINDOWS_NAMES_OPTIONS)),
            invalid_name: Some(windows_names_refuses),
            ..Self::ntfs3()
        }
    }

    /// udisks passes `windows_names`, which mountinfo does not show.
    pub fn ntfs_3g() -> Self {
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
    pub fn cifs() -> Self {
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

    pub fn cifs_noserverino() -> Self {
        Self {
            mount_type: Some(("cifs", CIFS_NOSERVERINO_OPTIONS)),
            ..Self::cifs()
        }
    }

    pub fn cifs_nocase() -> Self {
        Self {
            mount_type: Some(("cifs", CIFS_NOCASE_OPTIONS)),
            invalid_name: Some(cifs_nocase_refuses),
            ..Self::cifs()
        }
    }

    pub fn cifs_posix() -> Self {
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

    pub fn sshfs() -> Self {
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

pub const FUSE_SUPER_MAGIC: FsWord = 0x6573_5546;
pub const FUSEBLK_SILLY: Silly = Silly {
    aside: Aside::Fuse,
    lingers: Duration::ZERO,
};
pub const EXT4_SUPER_MAGIC: FsWord = 0xEF53;
pub const EXT4: (&str, &str) = ("ext4", "rw");
pub const VBOXSF_OPTIONS: &str = "rw,nodev,relatime,iocharset=utf8,uid=1000,gid=1000";
pub const VFAT_OPTIONS: &str = concat!(
    "rw,uid=1000,gid=1000,fmask=0022,dmask=0022,codepage=437,iocharset=ascii,",
    "shortname=mixed,showexec,utf8,flush,errors=remount-ro"
);
pub const EXFAT_OPTIONS: &str =
    "rw,uid=1000,gid=1000,fmask=0022,dmask=0022,iocharset=utf8,errors=remount-ro";
pub const FUSEBLK_OPTIONS: &str =
    "rw,user_id=0,group_id=0,default_permissions,allow_other,blksize=4096";
pub const NTFS3_OPTIONS: &str = "rw,uid=1000,gid=1000,acl,iocharset=utf8,prealloc";
pub const NTFS3_WINDOWS_NAMES_OPTIONS: &str =
    "rw,uid=1000,gid=1000,windows_names,acl,iocharset=utf8,prealloc";
pub const CIFS_OPTIONS: &str = concat!(
    "rw,vers=3.1.1,cache=strict,uid=1000,forceuid,gid=1000,forcegid,soft,nounix,serverino,",
    "mapposix,reparse=nfs,nativesocket,symlink=native,actimeo=1,closetimeo=1"
);
pub const CIFS_NOSERVERINO_OPTIONS: &str = concat!(
    "rw,vers=3.1.1,cache=strict,uid=1000,forceuid,gid=1000,forcegid,soft,nounix,mapposix,",
    "reparse=nfs,nativesocket,symlink=native,actimeo=1,closetimeo=1"
);
pub const CIFS_NOCASE_OPTIONS: &str = concat!(
    "rw,vers=3.1.1,cache=strict,uid=1000,forceuid,gid=1000,forcegid,nocase,soft,nounix,",
    "serverino,mapposix,reparse=nfs,nativesocket,symlink=native,actimeo=1,closetimeo=1"
);
pub const CIFS_POSIX_OPTIONS: &str = concat!(
    "rw,vers=3.1.1,cache=strict,uid=1000,forceuid,gid=1000,forcegid,soft,posix,posixpaths,",
    "serverino,reparse=nfs,nonativesocket,symlink=native,actimeo=1,closetimeo=1"
);

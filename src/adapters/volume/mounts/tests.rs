// SPDX-License-Identifier: MIT

use std::{path::Path, time::Duration};

use super::{
    FAT32_MAX_FILE_SIZE, MountKey, MountTable, MountType, holds_symlinks, is_remote_fs_type,
    max_file_size, unescape,
};
use crate::adapters::volume::names::{
    NameEncoding, NameLimit, NameProfile, NameRules, PrivateUse, Reserved, Strip,
};
use crate::services::{IdentityTraits, InodeNumbers};

fn remote(table: &MountTable, path: &Path) -> bool {
    table.fs_type_for(path).is_some_and(is_remote_fs_type)
}

mod byte_paths;

const SAMPLE: &str = "\
22 1 0:20 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
40 22 0:36 / /mnt/nfs rw,relatime shared:22 - nfs4 server:/export rw,vers=4.2
41 40 8:17 / /mnt/nfs/local rw,relatime shared:23 - ext4 /dev/sdb1 rw
52 22 0:45 / /run/user/1000/gvfs rw,nosuid,nodev,relatime shared:30 - fuse.gvfsd-fuse gvfsd-fuse rw,user_id=1000
60 22 0:50 / /mnt/my\\040share rw,relatime shared:31 - cifs //server/share rw
malformed line without separator
";

#[test]
fn parses_mount_points_and_filesystem_types() {
    let table = MountTable::parse(SAMPLE);
    assert_eq!(
        table.fs_type_for(Path::new("/home/user/file")),
        Some("ext4")
    );
    assert_eq!(table.fs_type_for(Path::new("/mnt/nfs/docs")), Some("nfs4"));
    let fs_type_of = |id| table.mount_of(id).map(|(mount, _)| mount.fs_type.as_str());
    assert_eq!(fs_type_of(41), Some("ext4"));
    assert_eq!(fs_type_of(60), Some("cifs"));
    assert_eq!(fs_type_of(99), None);
    assert_eq!(
        table.fs_type_for(Path::new(
            "/run/user/1000/gvfs/smb-share:server=host,share=s/file"
        )),
        Some("fuse.gvfsd-fuse")
    );
}

#[test]
fn only_mount_types_known_to_keep_inode_numbers_are_trusted_to() {
    let table = MountTable::parse(
        "\
70 22 0:60 / /mnt/a rw - cifs //server/a rw,vers=3.1.1,cache=strict,serverino,mapposix
71 22 0:61 / /mnt/b rw - cifs //server/b rw,vers=3.1.1,cache=strict,mapposix
72 22 0:62 / /mnt/c rw - fuse.sshfs host:/ rw,user_id=1000
73 22 0:63 / /mnt/d rw - ext4 /dev/sdc1 rw,relatime
74 22 0:64 / /mnt/e rw - smb3 //server/e rw,vers=3.1.1,posix,serverino
75 22 8:33 / /mnt/f rw - vfat /dev/sdc2 rw,flush
76 22 8:34 / /mnt/g rw - exfat /dev/sdc3 rw
77 22 8:35 / /mnt/h rw - msdos /dev/sdc4 rw
78 22 0:65 / /mnt/i rw - vboxsf share rw,nodev,relatime,iocharset=utf8
79 22 0:66 / /mnt/j rw - 9p share rw,trans=virtio
80 22 8:36 / /mnt/k rw - jfs /dev/sdc5 rw
81 22 0:67 / /mnt/l rw - nfs server:/l rw,vers=3,serverino
",
    );
    let rows = [
        (70, InodeNumbers::Renewed, true, true),
        (71, InodeNumbers::Renewed, true, false),
        (72, InodeNumbers::Renewed, true, false),
        (73, InodeNumbers::Kept, true, false),
        (74, InodeNumbers::Renewed, true, true),
        (75, InodeNumbers::Renewed, true, false),
        (76, InodeNumbers::Renewed, false, false),
        (77, InodeNumbers::Renewed, true, false),
        (78, InodeNumbers::Renewed, true, false),
        (79, InodeNumbers::Kept, true, false),
        (80, InodeNumbers::Unknown, true, false),
        (81, InodeNumbers::Kept, true, false),
    ];
    for (id, numbers, folder_births, server_numbers) in rows {
        let mount = table.mount_of(id).map(|(mount, _)| mount);
        let smb = matches!(id, 70 | 71 | 74);
        assert_eq!(
            mount.map(MountType::identity),
            Some(IdentityTraits {
                numbers,
                folder_births,
                made_up_births: smb,
            }),
            "mount {id}"
        );
        assert_eq!(
            mount.map(MountType::server_numbers),
            Some(server_numbers),
            "mount {id}"
        );
    }
}

#[test]
fn file_handles_are_read_only_where_they_tell_a_reused_number_apart() {
    let table = MountTable::parse(
        "\
90 22 0:70 / /mnt/a rw - nfs server:/a rw,vers=3
91 22 0:71 / /mnt/b rw - nfs4 server:/b rw,vers=4.2
92 22 8:40 / /mnt/c rw - ext4 /dev/sdd1 rw
93 22 8:41 / /mnt/d rw - ext3 /dev/sdd2 rw
94 22 0:72 / /mnt/e rw - fuse.sshfs host:/ rw,user_id=1000
95 22 8:42 / /mnt/f rw - fuseblk /dev/sdd3 rw,user_id=0
96 22 0:73 / /mnt/g rw - cifs //server/g rw,vers=3.1.1,serverino
97 22 8:43 / /mnt/h rw - exfat /dev/sdd4 rw
",
    );
    let born = Some((1_700_000_000, 5));
    let rows = [
        (Some(90), None, true),
        (Some(91), Some((0, 0)), true),
        (Some(91), born, false),
        (Some(92), None, true),
        (Some(93), None, true),
        (Some(92), born, false),
        (Some(94), None, false),
        (Some(95), None, false),
        (Some(96), None, false),
        (Some(97), None, false),
        (Some(99), None, false),
        (None, None, false),
    ];
    for (id, birth, wanted) in rows {
        assert_eq!(
            table.wants_file_handle(id, birth),
            wanted,
            "mount {id:?}, birth {birth:?}"
        );
    }
}

#[test]
fn smb_mounts_keep_closed_handles_for_their_close_timeout() {
    let table = MountTable::parse(
        "\
80 22 0:70 / /mnt/a rw - cifs //server/a rw,vers=3.1.1,actimeo=1
81 22 0:71 / /mnt/b rw - cifs //server/b rw,vers=3.1.1,actimeo=1,closetimeo=0
82 22 0:72 / /mnt/c rw - smb3 //server/c rw,vers=3.1.1,closetimeo=5,cache=strict
83 22 0:73 / /mnt/d rw - nfs4 server:/d rw,vers=4.2
",
    );
    let deferred = |id| {
        table
            .mount_of(id)
            .and_then(|(mount, _)| mount.deferred_close())
    };
    assert_eq!(deferred(80), Some(Duration::from_secs(1)));
    assert_eq!(deferred(81), Some(Duration::ZERO));
    assert_eq!(deferred(82), Some(Duration::from_secs(5)));
    assert_eq!(deferred(83), None);
}

#[test]
fn smb_mounts_cache_file_attributes_for_their_regular_file_timeout() {
    let table = MountTable::parse(
        "\
80 22 0:70 / /mnt/a rw - cifs //server/a rw,vers=3.1.1,actimeo=1
81 22 0:71 / /mnt/b rw - cifs //server/b rw,vers=3.1.1,acdirmax=30,acregmax=5
82 22 0:72 / /mnt/c rw - smb3 //server/c rw,vers=3.1.1,actimeo=0
83 22 0:73 / /mnt/d rw - cifs //server/d rw,vers=3.1.1
84 22 0:74 / /mnt/e rw - nfs4 server:/e rw,vers=4.2,acregmax=60
",
    );
    let cached = |id| {
        table
            .mount_of(id)
            .and_then(|(mount, _)| mount.attribute_cache())
    };
    assert_eq!(cached(80), Some(Duration::from_secs(1)));
    assert_eq!(cached(81), Some(Duration::from_secs(5)));
    assert_eq!(cached(82), None);
    assert_eq!(cached(83), Some(Duration::from_secs(1)));
    assert_eq!(cached(84), None);
}

#[test]
fn mount_keys_identify_one_mount_instance() {
    let table = MountTable::parse(SAMPLE);
    let share = table
        .mount_for(Path::new("/mnt/my share/file"))
        .expect("share mount");
    assert_eq!(
        share,
        MountKey {
            mount_id: 60,
            device: (0, 50),
            mount_point: "/mnt/my share".into(),
            fs_type: "cifs".to_owned(),
        }
    );
    let remounted = MountTable::parse(SAMPLE.replace("60 22 0:50", "61 22 0:50"));
    assert_ne!(
        remounted.mount_for(Path::new("/mnt/my share/file")),
        Some(share)
    );
    assert!(
        MountTable::parse("x 0 0:1 / / rw - ext4 /dev/x rw\n")
            .mount_for(Path::new("/"))
            .is_none()
    );
}

#[test]
fn innermost_mount_wins() {
    let table = MountTable::parse(SAMPLE);
    let nested = Path::new("/mnt/nfs/local/file");
    assert_eq!(table.fs_type_for(nested), Some("ext4"));
    assert_eq!(
        table.mount_point_for(nested),
        Some(Path::new("/mnt/nfs/local"))
    );
    assert!(!remote(&table, nested));
    assert!(remote(&table, Path::new("/mnt/nfs/file")));
    assert_eq!(
        table.mount_point_for(Path::new("/mnt/nfs/docs")),
        Some(Path::new("/mnt/nfs"))
    );
    assert_eq!(
        table.mount_point_for(Path::new("/home/user")),
        Some(Path::new("/"))
    );
    assert_eq!(
        table.mount_point_for(Path::new("/mnt/nfsdata")),
        Some(Path::new("/"))
    );
}

#[test]
fn mount_point_escapes_are_decoded() {
    let table = MountTable::parse(SAMPLE);
    assert_eq!(
        table.fs_type_for(Path::new("/mnt/my share/file")),
        Some("cifs")
    );
    assert_eq!(unescape("a\\040b\\134c"), "a b\\c");
    assert_eq!(unescape("trailing\\"), "trailing\\");
    assert_eq!(unescape("not\\9octal"), "not\\9octal");
}

#[test]
fn detects_mount_points_exactly() {
    let table = MountTable::parse(SAMPLE);
    assert!(table.is_mount_point(Path::new("/")));
    assert!(table.is_mount_point(Path::new("/mnt/nfs")));
    assert!(table.is_mount_point(Path::new("/mnt/nfs/local")));
    assert!(table.is_mount_point(Path::new("/mnt/my share")));
    assert!(!table.is_mount_point(Path::new("/mnt/nfs/docs")));
    assert!(!table.is_mount_point(Path::new("/home/user")));
}

#[test]
fn trash_scan_skips_virtual_remote_and_fuse_mounts() {
    let table = MountTable::parse(SAMPLE);
    let scanned: Vec<_> = table.trash_scan_mounts().collect();
    assert_eq!(scanned, vec![Path::new("/"), Path::new("/mnt/nfs/local")]);
}

#[test]
fn prefix_matching_is_component_wise() {
    let table = MountTable::parse("1 0 0:1 / /mnt/nfs rw - nfs4 s:/e rw\n");
    assert!(table.fs_type_for(Path::new("/mnt/nfsdata/file")).is_none());
    assert!(
        table
            .mount_point_for(Path::new("/mnt/nfsdata/file"))
            .is_none()
    );
    assert!(table.fs_type_for(Path::new("relative/path")).is_none());
}

#[test]
fn fuse_drivers_for_local_drives_are_not_network_shares() {
    let table = MountTable::parse(format!(
        "{SAMPLE}70 22 8:33 / /run/media/user/USB rw,relatime shared:40 - fuseblk /dev/sdc1 rw\n"
    ));
    let usb = Path::new("/run/media/user/USB/photos");
    assert!(remote(&table, usb), "a FUSE daemon can still block");
    assert!(!table.is_network_path(usb));
    for share in [
        "/mnt/nfs/docs",
        "/run/user/1000/gvfs/share",
        "/mnt/my share/docs",
    ] {
        assert!(table.is_network_path(Path::new(share)), "{share}");
    }
    assert!(!table.is_network_path(Path::new("/mnt/nfs/local/docs")));
}

#[test]
fn network_and_fuse_filesystems_are_remote_and_written_from_elsewhere() {
    for (fs_type, remote, outside_writers) in [
        ("nfs", true, true),
        ("nfs4", true, true),
        ("cifs", true, true),
        ("smb3", true, true),
        ("fuse.sshfs", true, true),
        ("fuse.gvfsd-fuse", true, true),
        ("fuse", true, true),
        ("9p", true, true),
        ("virtiofs", true, true),
        ("vboxsf", true, true),
        ("ceph", true, true),
        ("gfs2", true, true),
        ("fuse.mergerfs", true, true),
        ("fuse.gocryptfs", true, true),
        ("fuseblk", true, false),
        ("ext4", false, false),
        ("btrfs", false, false),
        ("xfs", false, false),
        ("tmpfs", false, false),
        ("vfat", false, false),
        ("exfat", false, false),
        ("overlay", false, false),
    ] {
        let mount = MountType {
            fs_type: fs_type.to_owned(),
            super_options: String::new(),
        };
        assert_eq!(
            (is_remote_fs_type(fs_type), mount.outside_writers()),
            (remote, outside_writers),
            "{fs_type}"
        );
    }
}

/// The lab's mountinfo lines (2026-10-14), then options it did not mount.
const NAME_MOUNTS: &str = "\
1241 1219 0:135 / /tmp rw,nosuid,nodev,relatime - tmpfs tmpfs rw,size=16777216k,inode64,huge=advise
1128 1219 7:0 / /mnt/fslab/ext4 rw,relatime - ext4 /dev/loop0 rw,errors=remount-ro
1150 1219 7:2 / /mnt/fslab/vfat rw,relatime - vfat /dev/loop2 rw,uid=1000,gid=1000,fmask=0022,dmask=0022,codepage=437,iocharset=ascii,shortname=mixed,showexec,utf8,flush,errors=remount-ro
1167 1219 7:3 / /mnt/fslab/exfat rw,relatime - exfat /dev/loop3 rw,uid=1000,gid=1000,fmask=0022,dmask=0022,iocharset=utf8,errors=remount-ro
1262 1219 7:10 / /mnt/fslab/exfat-fuse rw,nosuid,nodev,relatime - fuseblk /dev/loop10 rw,user_id=0,group_id=0,default_permissions,allow_other,blksize=4096
1168 1219 7:4 / /mnt/fslab/ntfs3 rw,relatime - ntfs3 /dev/loop4 rw,uid=1000,gid=1000,acl,iocharset=utf8,prealloc
1170 1219 7:4 / /mnt/fslab/ntfs3-wn rw,relatime - ntfs3 /dev/loop4 rw,uid=1000,gid=1000,windows_names,acl,iocharset=utf8,prealloc
1177 1219 0:137 / /mnt/fslab/nfs42 rw,relatime - nfs4 127.0.0.1:/v42 rw,vers=4.2,rsize=1048576,wsize=1048576,namlen=255,hard,proto=tcp,timeo=600,retrans=2,sec=sys,clientaddr=127.0.0.1,local_lock=none,addr=127.0.0.1
1178 1219 0:138 / /mnt/fslab/cifs rw,relatime - cifs //127.0.0.1/cifs rw,vers=3.1.1,cache=strict,username=strata,uid=1000,forceuid,gid=1000,forcegid,addr=127.0.0.1,soft,nounix,serverino,mapposix,reparse=nfs,nativesocket,symlink=native,actimeo=1,closetimeo=1
1179 1219 0:139 / /mnt/fslab/cifs-nocase rw,relatime - cifs //127.0.0.1/nocase rw,vers=3.1.1,cache=strict,username=strata,uid=1000,forceuid,gid=1000,forcegid,addr=127.0.0.1,nocase,soft,nounix,serverino,mapposix,reparse=nfs,nativesocket,symlink=native,actimeo=1,closetimeo=1
1180 1219 0:140 / /mnt/fslab/cifs-posix rw,relatime - cifs //127.0.0.1/posix rw,vers=3.1.1,cache=strict,username=strata,uid=1000,forceuid,gid=1000,forcegid,addr=127.0.0.1,soft,posix,posixpaths,serverino,reparse=nfs,nonativesocket,symlink=native,actimeo=1,closetimeo=1
1261 1219 0:142 / /mnt/fslab/sshfs rw,nosuid,nodev,relatime - fuse.sshfs 127.0.0.1:/srv/sshfs rw,user_id=0,group_id=0,allow_other
2001 22 8:1 / /mnt/a rw - vfat /dev/sdb1 rw,codepage=437,iocharset=utf8
2002 22 8:2 / /mnt/b rw - vfat /dev/sdb2 rw,codepage=437,iocharset=ascii
2003 22 8:3 / /mnt/c rw - vfat /dev/sdb3 rw,codepage=850,iocharset=iso8859-1
2004 22 8:4 / /mnt/d rw - msdos /dev/sdb4 rw
2005 22 0:50 / /mnt/e rw - cifs //server/e rw,unix,mapposix
2006 22 0:51 / /mnt/f rw - cifs //server/f rw,unix,posixpaths
2007 22 0:52 / /mnt/g rw - smb3 //server/g rw,nounix,mapchars
2008 22 0:53 / /mnt/h rw - zfs pool/h rw,xattr,posixacl
2009 22 0:54 / /mnt/i rw - ecryptfs /mnt/i rw,ecryptfs_cipher=aes
2010 22 8:5 / /mnt/j rw - hfsplus /dev/sdb5 rw,umask=22,nls=utf8
2011 22 0:55 / /mnt/k rw - 9p shared rw,trans=virtio
2012 22 0:56 / /mnt/l rw - fuse.example example rw,user_id=1000
2013 22 8:6 / /mnt/m rw - vfat /dev/sdb6 rw,shortname=mixed,iocharset=utf8,utf8,flush
2014 22 8:7 / /mnt/n rw - exfat /dev/sdb7 rw,fmask=0022,dmask=0022,iocharset=ascii,errors=remount-ro
2015 22 0:57 / /mnt/o rw - nfs 127.0.0.1:/v3 rw,vers=3,namlen=255,hard,proto=tcp,sec=sys
2016 22 8:8 / /mnt/p rw - xfs /dev/sdb8 rw,attr2,inode64,logbufs=8,noquota
2017 22 0:58 / /mnt/q rw - btrfs /dev/sdb9 rw,ssd,space_cache=v2,subvolid=5,subvol=/
2018 22 8:10 / /mnt/r rw - f2fs /dev/sdb10 rw,lazytime,background_gc=on
2019 22 8:11 / /mnt/s rw - bcachefs /dev/sdb11 rw
2020 22 0:59 / /mnt/t rw - overlay overlay rw,lowerdir=/l,upperdir=/u,workdir=/w
2021 22 8:12 / /mnt/u rw - ntfs /dev/sdb12 rw,uid=1000,gid=1000,iocharset=utf8
2022 22 8:13 / /mnt/v rw - ntfs /dev/sdb13 rw,uid=1000,gid=1000,windows_names,iocharset=utf8
2023 22 0:60 / /mnt/w rw - smb3 //server/w rw,vers=3.1.1,posix,serverino
";

#[test]
fn a_bind_mount_shows_the_folder_of_its_filesystem_it_mounts() {
    let table = MountTable::parse(
        "\
40 22 0:61 / /run/user/1000/gvfs rw,nosuid,nodev - fuse.gvfsd-fuse gvfsd-fuse rw,user_id=1000
41 22 0:61 /mtp:host=Pixel/Internal\\040storage /home/u/phone rw - fuse.gvfsd-fuse gvfsd-fuse rw
",
    );
    for (id, point, root) in [
        (40, "/run/user/1000/gvfs", "/"),
        (41, "/home/u/phone", "/mtp:host=Pixel/Internal storage"),
    ] {
        let (_, site) = table.mount_of(id).expect("listed mount");
        assert_eq!(
            (site.point.as_path(), site.root.as_path()),
            (Path::new(point), Path::new(root)),
            "{id}"
        );
    }
}

#[test]
fn name_rules_follow_the_driver_its_options_and_its_name_length() {
    let posix = |bytes, learns| NameProfile {
        rules: NameRules::Posix,
        strip: Strip::None,
        reserved: Reserved::None,
        encoding: NameEncoding::AnyBytes,
        mapped: None,
        limit: NameLimit {
            bytes,
            ..NameLimit::default()
        },
        folds: false,
        learns,
    };
    let windows = |reserved, encoding, learns| NameProfile {
        rules: NameRules::Windows,
        strip: Strip::Trailing,
        reserved,
        encoding,
        mapped: None,
        limit: NameLimit {
            units: Some(255),
            ..NameLimit::default()
        },
        folds: false,
        learns,
    };
    let fat = |encoding| NameProfile {
        folds: true,
        ..windows(Reserved::Bare, encoding, false)
    };
    let smb = |encoding, mapped| NameProfile {
        mapped,
        limit: NameLimit {
            bytes: Some(255),
            units: Some(255),
            folder_bytes: Some(213),
        },
        folds: true,
        ..windows(Reserved::Full, encoding, false)
    };
    let smb_posix = NameProfile {
        encoding: NameEncoding::Utf8,
        limit: NameLimit {
            bytes: Some(255),
            folder_bytes: Some(213),
            ..NameLimit::default()
        },
        ..posix(Some(255), false)
    };
    let bare = windows(Reserved::Bare, NameEncoding::Utf8, false);
    let fat_utf8 = fat(NameEncoding::Utf8);
    let full = windows(Reserved::Full, NameEncoding::Utf8, false);
    let fat32 = Some(FAT32_MAX_FILE_SIZE);
    let table = MountTable::parse(NAME_MOUNTS);
    for (id, device_format, name_max, profile, symlinks, max_size) in [
        (1241, None, 255, posix(Some(255), false), None, None),
        (1128, None, 255, posix(Some(255), false), None, None),
        (1128, None, 0, posix(None, false), None, None),
        // `f_namelen` 1530 means 255 UTF-16 units of up to six bytes.
        (1150, None, 1530, fat_utf8, Some(false), fat32),
        (
            2001,
            None,
            1530,
            fat(NameEncoding::Utf8Bmp),
            Some(false),
            fat32,
        ),
        (
            2002,
            None,
            1530,
            fat(NameEncoding::Ascii),
            Some(false),
            fat32,
        ),
        (2003, None, 1530, fat_utf8, Some(false), fat32),
        (2013, None, 1530, fat_utf8, Some(false), fat32),
        (2004, None, 12, fat_utf8, Some(false), fat32),
        (1167, None, 1530, fat_utf8, Some(false), None),
        (
            2014,
            None,
            1530,
            fat(NameEncoding::Ascii),
            Some(false),
            None,
        ),
        (1262, Some("exfat"), 255, fat_utf8, Some(false), None),
        (1262, Some("vfat"), 255, fat_utf8, Some(false), fat32),
        (1262, Some("ntfs"), 255, full, None, None),
        (
            1262,
            None,
            255,
            windows(Reserved::Bare, NameEncoding::Utf8, true),
            None,
            None,
        ),
        (
            1262,
            Some("ext4"),
            255,
            windows(Reserved::Bare, NameEncoding::Utf8, true),
            None,
            None,
        ),
        (1168, None, 255, bare, None, None),
        (1170, None, 255, full, None, None),
        (1177, None, 255, posix(Some(255), false), None, None),
        (2015, None, 255, posix(Some(255), false), None, None),
        (2016, None, 255, posix(Some(255), false), None, None),
        (2017, None, 255, posix(Some(255), false), None, None),
        (2018, None, 255, posix(Some(255), false), None, None),
        (2019, None, 255, posix(Some(255), false), None, None),
        (2020, None, 255, posix(Some(255), false), None, None),
        (2021, None, 255, bare, None, None),
        (2022, None, 255, full, None, None),
        (2023, None, 255, smb_posix, None, None),
        (
            1178,
            None,
            255,
            smb(NameEncoding::Utf8, Some(PrivateUse::Sfm)),
            None,
            None,
        ),
        (
            1179,
            None,
            255,
            smb(NameEncoding::Utf8Bmp, Some(PrivateUse::Sfm)),
            None,
            None,
        ),
        (1180, None, 255, smb_posix, None, None),
        (
            2005,
            None,
            255,
            smb(NameEncoding::Utf8, Some(PrivateUse::Sfm)),
            None,
            None,
        ),
        (2006, None, 255, smb_posix, None, None),
        (
            2007,
            None,
            255,
            smb(NameEncoding::Utf8, Some(PrivateUse::Sfu)),
            None,
            None,
        ),
        (1261, None, 255, posix(Some(255), true), None, None),
        (2008, None, 1023, posix(Some(1023), false), None, None),
        (2009, None, 143, posix(Some(143), true), None, None),
        (
            2010,
            None,
            255,
            NameProfile {
                encoding: NameEncoding::Utf8,
                limit: NameLimit {
                    units: Some(255),
                    ..NameLimit::default()
                },
                ..posix(None, false)
            },
            None,
            None,
        ),
        (2011, None, 255, posix(Some(255), true), None, None),
        // A FUSE daemon's `f_namelen` is whatever it reports.
        (2012, None, 0, posix(None, true), None, None),
        (2012, None, 10, posix(Some(64), true), None, None),
        (2012, None, 10_000, posix(Some(4095), true), None, None),
    ] {
        let (mount, _) = table.mount_of(id).expect("listed mount");
        assert_eq!(
            mount.name_profile(device_format, name_max),
            profile,
            "{id} {device_format:?} {name_max}"
        );
        assert_eq!(
            holds_symlinks(&mount.fs_type, device_format),
            symlinks,
            "{id} {device_format:?}"
        );
        assert_eq!(
            max_file_size(&mount.fs_type, device_format),
            max_size,
            "{id} {device_format:?}"
        );
    }
}

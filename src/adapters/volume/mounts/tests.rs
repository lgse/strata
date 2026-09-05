// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use super::{MountTable, is_remote_fs_type, unescape};

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
    assert_eq!(
        table.fs_type_for(Path::new(
            "/run/user/1000/gvfs/smb-share:server=host,share=s/file"
        )),
        Some("fuse.gvfsd-fuse")
    );
}

#[test]
fn innermost_mount_wins() {
    let table = MountTable::parse(SAMPLE);
    assert_eq!(
        table.fs_type_for(Path::new("/mnt/nfs/local/file")),
        Some("ext4")
    );
    assert!(!table.is_remote_path(Path::new("/mnt/nfs/local/file")));
    assert!(table.is_remote_path(Path::new("/mnt/nfs/file")));
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
fn prefix_matching_is_component_wise() {
    let table = MountTable::parse("1 0 0:1 / /mnt/nfs rw - nfs4 s:/e rw\n");
    assert!(table.fs_type_for(Path::new("/mnt/nfsdata/file")).is_none());
    assert!(table.fs_type_for(Path::new("relative/path")).is_none());
}

#[test]
fn network_and_fuse_filesystems_are_remote() {
    for fs_type in [
        "nfs",
        "nfs4",
        "cifs",
        "smb3",
        "fuse.sshfs",
        "fuse.gvfsd-fuse",
        "fuse",
        "9p",
    ] {
        assert!(is_remote_fs_type(fs_type), "{fs_type} should be remote");
    }
    for fs_type in ["ext4", "btrfs", "xfs", "tmpfs", "vfat", "exfat", "overlay"] {
        assert!(!is_remote_fs_type(fs_type), "{fs_type} should be local");
    }
}

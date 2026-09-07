// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

#[test]
fn raw_non_utf8_mountinfo_preserves_all_mounts() {
    let table = MountTable::parse(b"1 0 8:1 / / rw - ext4 /dev/root rw\n2 1 0:2 / /media/disk-\xff rw - fuse.test source-\xfe rw\n3 1 0:3 / /tmp rw - tmpfs tmpfs rw\n");
    let mount = Path::new(OsStr::from_bytes(b"/media/disk-\xff"));
    assert_eq!(table.mount_point_for(&mount.join("report")), Some(mount));
    assert!(table.is_remote_path(&mount.join("report")));
    assert_eq!(
        table.mount_point_for(Path::new("/tmp/report")),
        Some(Path::new("/tmp"))
    );
    assert_eq!(
        table.mount_point_for(Path::new("/home/report")),
        Some(Path::new("/"))
    );
}

#[test]
fn mountinfo_combines_raw_bytes_and_octal_escapes() {
    let table = MountTable::parse(b"1 0 0:1 / /mnt/\xff\\040disk\\134name rw - ext4 /dev/x rw\n");
    let mount = Path::new(OsStr::from_bytes(b"/mnt/\xff disk\\name"));
    assert_eq!(table.mount_point_for(&mount.join("file")), Some(mount));
}

#[test]
fn malformed_filesystem_type_does_not_discard_other_entries() {
    let table =
        MountTable::parse(b"1 0 0:1 / / rw - ext4 /dev/x rw\n2 1 0:2 / /bad rw - \xff x rw\n");
    assert_eq!(
        table.mount_point_for(Path::new("/home/file")),
        Some(Path::new("/"))
    );
}

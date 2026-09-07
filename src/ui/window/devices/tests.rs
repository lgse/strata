// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn global_search_always_includes_home_and_all_mounted_drives() {
    assert_eq!(
        search_roots(
            Path::new("/home/me"),
            ["/run/media/me/USB", "/mnt/Backup"].map(PathBuf::from)
        ),
        ["/home/me", "/mnt/Backup", "/run/media/me/USB"].map(PathBuf::from)
    );
    assert_eq!(
        search_roots(Path::new("/home/me"), []),
        [PathBuf::from("/home/me")]
    );
}

#[test]
fn global_search_deduplicates_roots_without_dropping_nested_mounts() {
    assert_eq!(
        search_roots(
            Path::new("/home/me"),
            [
                "/home/me",
                "/home/me/USB",
                "/home/me/USB",
                "/home/me/USB/nested"
            ]
            .map(PathBuf::from)
        ),
        ["/home/me", "/home/me/USB", "/home/me/USB/nested"].map(PathBuf::from)
    );
}

#[test]
fn global_search_does_not_expand_into_system_mounts_or_gvfs_mirrors() {
    assert_eq!(
        search_roots(
            Path::new("/home/me"),
            [
                "/",
                "/home",
                "/boot",
                "/proc",
                "/sys",
                "relative",
                "/run/user/1000/gvfs"
            ]
            .map(PathBuf::from)
        ),
        [PathBuf::from("/home/me")]
    );
}

#[test]
fn a_non_system_drive_containing_home_is_still_included() {
    assert_eq!(
        search_roots(
            Path::new("/mnt/Storage/Home"),
            [PathBuf::from("/mnt/Storage")]
        ),
        ["/mnt/Storage/Home", "/mnt/Storage"].map(PathBuf::from)
    );
}

#[test]
fn mount_table_fallback_deduplicates_gio_and_repeated_mount_roots() {
    let device = |root| MountedDevice {
        name: "USB".into(),
        root: PathBuf::from(root),
    };
    let devices = unrepresented_devices(
        vec![
            device("/mnt/known"),
            device("/mnt/missing"),
            device("/mnt/missing"),
        ],
        [PathBuf::from("/mnt/known")],
    );
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].root, Path::new("/mnt/missing"));
}

#[test]
fn mount_table_fallback_keeps_storage_but_not_system_mounts() {
    let devices = mounted_devices_from_table(
        br"25 1 8:1 / / rw - ext4 /dev/sda1 rw
26 1 8:2 / /boot rw - vfat /dev/sda2 rw
27 1 0:1 / /proc rw - proc proc rw
28 1 8:3 / /run/media/me/USB\040Backup rw shared:1 - exfat /dev/sdb1 rw
29 1 8:4 / /mnt/Archive ro - fuseblk /dev/sdc1 ro
30 1 0:2 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw
malformed
31 1 8:5 / /mnt/missing-source rw - ext4
",
    );
    assert_eq!(
        devices
            .iter()
            .map(|device| device.root.clone())
            .collect::<Vec<_>>(),
        vec![
            PathBuf::from("/run/media/me/USB Backup"),
            PathBuf::from("/mnt/Archive")
        ]
    );
    assert_eq!(devices[0].name, "USB Backup");
}

#[test]
fn mount_paths_decode_kernel_escapes_without_losing_native_bytes() {
    assert_eq!(
        mount_path(br"/mnt/a\040b\011c\012d\134e"),
        Some(PathBuf::from("/mnt/a b\tc\nd\\e"))
    );
    assert_eq!(
        mount_path(b"/mnt/\xff"),
        Some(std::ffi::OsString::from_vec(b"/mnt/\xff".to_vec()).into())
    );
    assert_eq!(mount_path(br"/mnt/bad\04"), None);
    assert_eq!(mount_path(br"/mnt/bad\999"), None);
}

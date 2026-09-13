// SPDX-License-Identifier: MIT

use super::*;
use gtk::gio::DriveStartStopType;

#[test]
fn device_identity_prefers_unix_device_then_uuid() {
    assert_eq!(
        device_identity(Some("/dev/sda1"), Some("uuid-a"), Some("drive")),
        Some("/dev/sda1".into())
    );
    assert_eq!(
        device_identity(None, Some("uuid-a"), Some("drive")),
        Some("uuid-a".into())
    );
    assert_eq!(
        device_identity(Some("  "), Some("uuid-a"), None),
        Some("uuid-a".into())
    );
    assert_eq!(
        device_identity(None, None, Some("drive-id")),
        Some("drive-id".into())
    );
    assert_eq!(device_identity(None, None, None), None);
}

#[test]
fn encrypted_device_detection() {
    let none = BlockCryptoRef::default();
    assert!(is_encrypted_device(
        &["drive-harddisk-usb", "changes-prevent"],
        None,
        none
    ));
    assert!(is_encrypted_device(
        &["changes-allow-symbolic"],
        Some(DriveStartStopType::Shutdown),
        none
    ));
    assert!(is_encrypted_device(
        &["drive-harddisk-encrypted-symbolic"],
        None,
        none
    ));
    assert!(!is_encrypted_device(
        &["drive-harddisk-usb", "drive-harddisk"],
        Some(DriveStartStopType::Shutdown),
        none
    ));
    assert!(is_encrypted_device(
        &["drive-harddisk"],
        Some(DriveStartStopType::Shutdown),
        BlockCryptoRef {
            dm_uuid: Some("CRYPT-LUKS2-abc-root"),
            ..BlockCryptoRef::default()
        }
    ));
    assert!(!is_encrypted_device(
        &["drive-harddisk"],
        Some(DriveStartStopType::Shutdown),
        BlockCryptoRef {
            dm_uuid: Some("LVM-abc"),
            ..BlockCryptoRef::default()
        }
    ));
    assert!(is_encrypted_device(
        &["drive-harddisk-usb"],
        Some(DriveStartStopType::Shutdown),
        BlockCryptoRef {
            id_fs_usage: Some("crypto"),
            id_fs_type: Some("crypto_LUKS"),
            ..BlockCryptoRef::default()
        }
    ));
    assert!(!is_encrypted_device(
        &["drive-harddisk-usb"],
        Some(DriveStartStopType::Shutdown),
        BlockCryptoRef {
            id_fs_usage: Some("filesystem"),
            id_fs_type: Some("ext4"),
            ..BlockCryptoRef::default()
        }
    ));
}

#[test]
fn padlock_emblem_decides_lock_state() {
    assert!(encrypted_device_is_locked(&["changes-prevent"], true));
    assert!(!encrypted_device_is_locked(&["changes-allow"], false));
    assert!(encrypted_device_is_locked(
        &["drive-harddisk-encrypted"],
        false
    ));
    assert!(!encrypted_device_is_locked(
        &["drive-harddisk-encrypted"],
        true
    ));
}

#[test]
fn missing_unix_device_has_no_crypto_hint() {
    assert_eq!(
        probe_block_crypto("/dev/this-device-does-not-exist-537"),
        BlockCryptoHint::default()
    );
}

#[test]
fn luks_uuid_from_mapper_and_dm() {
    assert_eq!(
        luks_uuid_from_dm_uuid(
            "CRYPT-LUKS2-6e5d75a7e4e24c7d9c1c8e5a5e5d75a7-luks-6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7"
        )
        .as_deref(),
        Some("6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7")
    );
    assert_eq!(
        luks_uuid_from_dm_uuid("CRYPT-LUKS1-AABBCCDDEEFF00112233445566778899-crypt").as_deref(),
        Some("aabbccdd-eeff-0011-2233-445566778899")
    );
    assert_eq!(luks_uuid_from_dm_uuid("LVM-abc"), None);
    assert_eq!(luks_uuid_from_dm_uuid("CRYPT-PLAIN-abc"), None);
    assert_eq!(
        luks_uuid_from_unix_device("/dev/mapper/luks-6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7")
            .as_deref(),
        Some("6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7")
    );
    assert_eq!(luks_uuid_from_unix_device("/dev/dm-0"), None);
}

#[test]
fn crypto_password_uuid_prefers_mapper_over_filesystem() {
    let unlocked = BlockCryptoRef {
        dm_uuid: Some("CRYPT-LUKS2-6e5d75a7e4e24c7d9c1c8e5a5e5d75a7-crypt"),
        ..BlockCryptoRef::default()
    };
    assert_eq!(
        crypto_password_uuid(
            Some("11111111-2222-3333-4444-555555555555"),
            Some("/dev/dm-0"),
            unlocked,
            false
        )
        .as_deref(),
        Some("6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7")
    );
    assert_eq!(
        crypto_password_uuid(
            Some("11111111-2222-3333-4444-555555555555"),
            Some("/dev/dm-0"),
            BlockCryptoRef::default(),
            false
        ),
        None
    );
    assert_eq!(
        crypto_password_uuid(
            Some("6E5D75A7-E4E2-4C7D-9C1C-8E5A5E5D75A7"),
            Some("/dev/sdb1"),
            BlockCryptoRef::default(),
            true
        )
        .as_deref(),
        Some("6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7")
    );
}

#[test]
fn orphaned_password_drive_listing_uses_production_filter() {
    let covered = vec!["/dev/loop0".to_owned()];
    assert!(
        password_drive_is_orphaned(DriveStartStopType::Password, Some("/dev/sdb"), &[], false),
        "uncovered password drive should stay in the listing"
    );
    assert!(
        !password_drive_is_orphaned(
            DriveStartStopType::Password,
            Some("/dev/loop0"),
            &covered,
            false
        ),
        "a volume with the same identity should omit the password drive"
    );
    assert!(
        !password_drive_is_orphaned(DriveStartStopType::Password, Some("/dev/sdb"), &[], true),
        "a volume that claims the drive should omit the password drive"
    );
    assert!(
        !password_drive_is_orphaned(DriveStartStopType::Shutdown, Some("/dev/sr0"), &[], false),
        "non-password drives are not listed as orphaned unlock targets"
    );
}

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
        devices,
        vec![
            PathBuf::from("/run/media/me/USB Backup"),
            PathBuf::from("/mnt/Archive")
        ]
    );
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

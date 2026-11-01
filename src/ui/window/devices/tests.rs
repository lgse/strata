// SPDX-License-Identifier: MIT

use super::*;
use gtk::gio::DriveStartStopType;

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
fn removable_mount_policy_excludes_unsafe_roots_and_fixed_storage() {
    let eligible = RemovableMountFacts {
        shadowed: false,
        native_root: true,
        removable: true,
    };
    assert!(removable_mount_is_eligible(eligible));

    for (reason, facts) in [
        (
            "shadowed mount",
            RemovableMountFacts {
                shadowed: true,
                ..eligible
            },
        ),
        (
            "non-native root",
            RemovableMountFacts {
                native_root: false,
                ..eligible
            },
        ),
        (
            "fixed storage",
            RemovableMountFacts {
                removable: false,
                ..eligible
            },
        ),
    ] {
        assert!(
            !removable_mount_is_eligible(facts),
            "{reason} must not be offered"
        );
    }
}

#[test]
fn send_to_identity_uses_namespaced_volume_drive_unix_and_root_fallbacks() {
    assert_eq!(
        send_to_device_identity(
            Some(" volume-id "),
            Some("drive-id"),
            Some("/dev/sdb1"),
            Some("/dev/sdb"),
            Some("file:///run/media/me/USB"),
        )
        .as_deref(),
        Some("volume:volume-id")
    );
    assert_eq!(
        send_to_device_identity(
            Some("  "),
            Some("drive-id"),
            Some("/dev/sdb1"),
            Some("/dev/sdb"),
            Some("file:///usb"),
        )
        .as_deref(),
        Some("drive:drive-id")
    );
    assert_eq!(
        send_to_device_identity(
            None,
            None,
            Some("/dev/sdb1"),
            Some("/dev/sdb"),
            Some("file:///usb")
        )
        .as_deref(),
        Some("unix:/dev/sdb1")
    );
    assert_eq!(
        send_to_device_identity(None, None, None, Some("/dev/sdb"), Some("file:///usb")).as_deref(),
        Some("unix:/dev/sdb")
    );
    assert_eq!(
        send_to_device_identity(None, None, Some(""), Some("/dev/sdb"), Some("file:///usb"))
            .as_deref(),
        Some("unix:/dev/sdb")
    );
    assert_eq!(
        send_to_device_identity(
            None,
            None,
            Some("  \t"),
            Some("/dev/sdb"),
            Some("file:///usb")
        )
        .as_deref(),
        Some("unix:/dev/sdb")
    );
    assert_eq!(
        send_to_device_identity(None, None, Some(""), None, Some("file:///usb")).as_deref(),
        Some("root:file:///usb")
    );
    assert_eq!(
        send_to_device_identity(Some("same"), Some("same"), None, None, Some("same")),
        Some("volume:same".to_owned())
    );
    assert_eq!(
        send_to_device_identity(None, Some("same"), None, None, Some("same")),
        Some("drive:same".to_owned())
    );
    assert_eq!(
        send_to_device_identity(Some("  "), Some(""), None, None, Some(" ")),
        None
    );
}

#[test]
fn duplicate_send_to_identities_are_omitted_and_names_sort_deterministically() {
    let destination = |id: &str, name: &str| RemovableDestination {
        id: id.to_owned(),
        name: name.to_owned(),
        root: PathBuf::from(format!("/mnt/{id}")),
    };
    let mut unique = without_ambiguous_destinations(vec![
        destination("duplicate", "Kingston"),
        destination("duplicate", "Other name"),
        destination("z", "sanDisk"),
        destination("a", "SanDisk"),
    ]);
    sort_removable_destinations(&mut unique);
    assert_eq!(
        unique
            .iter()
            .map(|destination| (destination.name.as_str(), destination.id.as_str()))
            .collect::<Vec<_>>(),
        [("SanDisk", "a"), ("sanDisk", "z")]
    );
}

#[test]
fn padlock_emblem_decides_lock_state_before_mount_and_mapping() {
    for (icon, has_mount, cleartext_open, locked) in [
        ("changes-prevent", true, true, true),
        ("changes-allow", false, false, false),
        ("drive-harddisk-encrypted", false, false, true),
        ("drive-harddisk-encrypted", true, false, false),
        ("drive-harddisk-encrypted", false, true, false),
    ] {
        assert_eq!(
            encrypted_device_is_locked(&[icon], has_mount, cleartext_open),
            locked,
            "{icon} mounted={has_mount} open={cleartext_open}"
        );
    }
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
fn strata_label_identity_survives_mount_changes_without_using_recycled_device_paths() {
    assert_eq!(
        label_identity(Some("fixture-uuid"), Some("file:///media/old")),
        label_identity(Some("fixture-uuid"), Some("file:///media/new"))
    );
    assert_ne!(
        label_identity(Some("fixture-uuid"), None),
        label_identity(Some("other-uuid"), None)
    );
    assert_eq!(
        label_identity(None, Some("smb://server/share")),
        Some("root:smb://server/share".to_owned())
    );
    assert_eq!(label_identity(Some(""), None), None);
}

#[test]
fn properties_resolve_the_innermost_block_mount_including_btrfs_and_escaped_paths() {
    let table = &MountTable::parse(
        br"25 1 8:1 / / rw - ext4 /dev/sda1 rw
26 25 0:40 /@home /home rw - btrfs /dev/nvme0n1p2 rw
27 26 8:2 / /home/USB\040Backup rw - exfat /dev/sdb1 rw
28 27 0:2 / /home/USB\040Backup/cache rw - tmpfs tmpfs rw
",
    );
    for (device, root) in [
        ("/dev/sda1", Some("/")),
        ("/dev/nvme0n1p2", Some("/home")),
        ("/dev/sdb1", Some("/home/USB Backup")),
        ("/dev/strata-unmounted-fixture", None),
    ] {
        assert_eq!(
            mounted_path_from_table(table, Path::new(device)),
            root.map(PathBuf::from)
        );
    }
    for (path, device) in [
        ("/", Some("/dev/sda1")),
        ("/home/Documents", Some("/dev/nvme0n1p2")),
        ("/home/USB Backup/file.txt", Some("/dev/sdb1")),
        ("/home/USB Backup/cache", None),
        ("/home/USB Backup-other", Some("/dev/nvme0n1p2")),
    ] {
        assert_eq!(
            block_device_from_mount_table(table, Path::new(path)),
            device.map(PathBuf::from)
        );
    }
}

#[test]
fn mount_table_fallback_keeps_storage_but_not_system_mounts() {
    crate::test_support::gtk_test(
        "ui::window::devices::tests::mount_table_fallback_keeps_storage_but_not_system_mounts",
        || {
            crate::adapters::mount_watch::publish_for_isolated_test(Some(MountTable::parse(
                br"25 1 8:1 / / rw - ext4 /dev/sda1 rw
26 1 8:2 / /boot rw - vfat /dev/sda2 rw
27 1 0:1 / /proc rw - proc proc rw
28 1 8:3 / /run/media/me/USB\040Backup rw shared:1 - exfat /dev/sdb1 rw
29 1 8:4 / /mnt/Archive ro - fuseblk /dev/sdc1 ro
30 1 0:2 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw
malformed
31 1 8:5 / /mnt/missing-source rw - ext4
",
            )));
            let roots = global_search_roots();
            for (root, searched) in [
                ("/", false),
                ("/run/media/me/USB Backup", true),
                ("/mnt/Archive", true),
                ("/boot", false),
                ("/proc", false),
                ("/run/user/1000/gvfs", false),
                ("/mnt/missing-source", false),
            ] {
                assert_eq!(roots.contains(&PathBuf::from(root)), searched, "{root}");
            }
        },
    );
}

#[test]
fn crypto_hints_are_read_off_the_ui_thread_and_rebuild_the_sidebar_when_they_arrive() {
    crate::test_support::gtk_test(
        "ui::window::devices::tests::crypto_hints_are_read_off_the_ui_thread_and_rebuild_the_sidebar_when_they_arrive",
        || {
            static PROBED_ON_UI_THREAD: std::sync::Mutex<Vec<bool>> =
                std::sync::Mutex::new(Vec::new());
            fn probe(_: &str) -> BlockCryptoHint {
                PROBED_ON_UI_THREAD
                    .lock()
                    .expect("probes")
                    .push(crate::util::ui_thread::is_current());
                BlockCryptoHint {
                    id_fs_usage: Some("crypto".to_owned()),
                    ..BlockCryptoHint::default()
                }
            }
            let rebuilt = Rc::new(Cell::new(0));
            let counted = rebuilt.clone();
            let _sidebar = super::super::device_release::watch_sidebars(move || {
                counted.set(counted.get() + 1);
            });
            assert_eq!(
                block_crypto_hint_with("/dev/fixture-crypt", probe),
                BlockCryptoHint::default()
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while rebuilt.get() == 0 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the hint never arrived"
                );
                gtk::glib::MainContext::default().iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert_eq!(
                block_crypto_hint_with("/dev/fixture-crypt", probe)
                    .id_fs_usage
                    .as_deref(),
                Some("crypto")
            );
            assert_eq!(rebuilt.get(), 1);
            assert_eq!(*PROBED_ON_UI_THREAD.lock().expect("probes"), [false]);
        },
    );
}

#[test]
fn a_device_check_refused_while_its_mount_stalls_never_hides_the_device() {
    crate::test_support::gtk_test(
        "ui::window::devices::tests::a_device_check_refused_while_its_mount_stalls_never_hides_the_device",
        || {
            let device = tempfile::tempdir().expect("device fixture");
            let root = |name: &str| {
                let root = device.path().join(name);
                std::fs::create_dir_all(&root).expect("device root");
                root
            };
            let (slow, steady, late) = (root("slow"), root("steady"), root("late"));
            let parked = crate::test_support::parked::Parked::default();
            let _device = crate::adapters::FsQuirks::install(
                device.path(),
                crate::adapters::Quirks {
                    before: vec![(crate::adapters::FsOp::Access, "slow", 0, parked.racer())],
                    ..crate::adapters::Quirks::default()
                },
            );
            let offer = |roots: &[&PathBuf]| {
                crate::ui::offer_removable_destinations_for_test(
                    roots
                        .iter()
                        .map(|root| {
                            let name = root.file_name().expect("name").to_string_lossy();
                            RemovableDestination {
                                id: format!("volume:{name}"),
                                name: name.into_owned(),
                                root: (*root).clone(),
                            }
                        })
                        .collect(),
                );
            };
            let shown = || {
                removable_destinations()
                    .into_iter()
                    .map(|destination| destination.name)
                    .collect::<Vec<_>>()
            };
            let settle = |what: &str, until: &dyn Fn() -> bool| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !until() {
                    assert!(std::time::Instant::now() < deadline, "{what}");
                    gtk::glib::MainContext::default().iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            };

            offer(&[&steady, &late]);
            settle("both devices checked", &|| shown() == ["late", "steady"]);
            offer(&[&slow, &steady]);
            settle("the slow check parked", &|| parked.reached());
            let parked_at = std::time::Instant::now();
            settle("the mount stalls", &|| {
                parked_at.elapsed()
                    > crate::adapters::mount_lanes::STALLED_AFTER
                        + std::time::Duration::from_millis(300)
            });
            offer(&[&slow, &steady, &late]);
            let offered_at = std::time::Instant::now();
            settle("the refused checks land", &|| {
                offered_at.elapsed() > std::time::Duration::from_millis(200)
            });
            assert_eq!(shown(), ["steady"]);

            parked.release();
            settle(
                "the parked check's answer and a re-check after a refusal show every device",
                &|| shown() == ["late", "slow", "steady"],
            );
        },
    );
}

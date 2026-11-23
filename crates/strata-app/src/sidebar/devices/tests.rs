// SPDX-License-Identifier: MIT

use std::{collections::HashMap, path::PathBuf};

use gio::prelude::*;
use rune::{Key, Mods};
use serde_json::Value;
use strata_components::sidebar::LockState;
use strata_core::model::Location;

use super::{
    DeviceMenuInfo, MediaRelease,
    crypto::{
        BlockCryptoRef, crypto_password_uuid, encrypted_device_is_locked, is_encrypted_device,
        luks_uuid_from_dm_uuid, luks_uuid_from_unix_device, normalize_luks_uuid,
        password_drive_is_orphaned,
    },
    display_name, leaves,
    listing::{Seen, Source, device_identity, is_network_share, is_system_loop, label_identity},
    lock::{
        ForgetCachedPasswordError, LockDeviceError, LockGuard, forget_failure_from_delete_prompt,
        forget_failure_from_search, lock_encrypted_device, luks_password_lookups,
    },
    order::{DeviceHome, DevicePlacement, DriveRank, group_by_drive},
    release::{
        DeviceIds, ReleaseKey, ReleaseKind, body_for_progress, clear_pending_for_test, decide_gio,
        device_matches, push_pending_for_test, release_error_is_quiet,
    },
    release_action, seed_for_test,
    unlock::{
        DeviceKeys, ForeignVolumeWaitFollowUp, ForeignVolumeWaitOutcome, UnlockSlotGuard,
        UnlockViewFollowUp, VolumeSuccessorKind, device_volume_mount_is_ready,
        foreign_volume_wait_follow_up, unlock_view_follow_up,
        volume_error_is_authentication_failure, volume_error_is_in_flight_mount,
    },
};
use crate::{
    App, properties, shell,
    sidebar::{
        keys::DEVICES_HEADING,
        menu::{
            DISCONNECT, EJECT, LOCK, MOUNT, MenuRow, OPEN_IN_TAB, PROPERTIES, UNLOCK, UNMOUNT,
            device_rows,
        },
    },
    slots::Inbox,
    test_support::{
        PATIENCE, fixture, hosted,
        view::{all, idle, press, text},
    },
};

const MODULE: &str = "sidebar::devices::tests";

fn keyed<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    all(tree, &|node| node["key"] == key).into_iter().next()
}

fn synthetic_device(
    id: &str,
    name: &str,
    root: Option<Location>,
    lock: Option<LockState>,
    release: Option<MediaRelease>,
    can_mount: bool,
    network: bool,
) -> Seen {
    Seen {
        id: id.to_owned(),
        name: name.to_owned(),
        label_id: None,
        description: root
            .as_ref()
            .map(Location::display_path)
            .unwrap_or_else(|| format!("/dev/{id}")),
        ids: DeviceIds {
            unix_device: Some(format!("/dev/{id}")),
            uuid: None,
            mount_root: root
                .as_ref()
                .and_then(Location::native_path)
                .map(ToOwned::to_owned),
        },
        root,
        release,
        lock,
        can_mount,
        network,
        source: Source::Synthetic,
    }
}

#[test]
fn release_prefers_eject_then_unmount_and_offers_nothing_for_fixed_storage() {
    let cases = [
        ((true, false, true), Some(MediaRelease::EjectVolume)),
        ((true, true, true), Some(MediaRelease::EjectVolume)),
        ((false, true, true), Some(MediaRelease::EjectMount)),
        ((false, false, true), Some(MediaRelease::UnmountMount)),
        ((false, false, false), None),
    ];
    for ((volume, mount_eject, mount_unmount), expected) in cases {
        assert_eq!(
            release_action(volume, mount_eject, mount_unmount),
            expected,
            "volume ejects {volume}, mount ejects {mount_eject}, mount unmounts {mount_unmount}",
        );
    }
}

#[test]
fn the_browser_leaves_the_device_root_and_folders_under_it_only() {
    let root = Location::local("/run/media/me/stick");
    let cases = [
        ("/run/media/me/stick", true),
        ("/run/media/me/stick/photos/2024", true),
        ("/run/media/me/stick2", false),
        ("/run/media/me", false),
        ("/home/me", false),
    ];
    for (active, expected) in cases {
        assert_eq!(
            leaves(&Location::local(active), &root),
            expected,
            "{active}"
        );
    }
}

#[test]
fn a_device_shows_a_label_the_settings_may_hold_else_its_system_name() {
    let labels = HashMap::from([
        ("volume:1234".to_owned(), "Photos".to_owned()),
        ("volume:blank".to_owned(), "  ".to_owned()),
        ("volume:control".to_owned(), "Bad\u{7}".to_owned()),
    ]);
    let cases = [
        (Some("volume:1234"), "Photos"),
        (Some("volume:blank"), "SanDisk"),
        (Some("volume:control"), "SanDisk"),
        (Some("volume:other"), "SanDisk"),
        (None, "SanDisk"),
    ];
    for (id, expected) in cases {
        assert_eq!(display_name(id, "SanDisk", &labels), expected, "{id:?}");
    }
}

#[test]
fn identities_take_the_first_value_given() {
    assert_eq!(
        device_identity(Some(" "), Some("1234-ABCD"), Some("drive")).as_deref(),
        Some("1234-ABCD"),
    );
    assert_eq!(device_identity(None, None, None), None);
    assert_eq!(
        label_identity(Some("1234"), Some("file:///media/x")).as_deref(),
        Some("volume:1234"),
    );
    assert_eq!(
        label_identity(Some(""), Some("file:///media/x")).as_deref(),
        Some("root:file:///media/x"),
    );
}

#[test]
fn only_loops_the_user_did_not_attach_are_system_loops() {
    assert!(is_system_loop(Some("loop"), false));
    assert!(!is_system_loop(Some("loop"), true));
    assert!(!is_system_loop(Some("device"), false));
    assert!(!is_system_loop(None, false));
}

#[test]
fn network_backends_are_shares_and_local_mounts_are_not() {
    for (location, expected) in [
        (Location::uri("smb://nas/share"), true),
        (Location::uri("SFTP://host/home"), true),
        (Location::uri("mtp://phone/"), false),
        (Location::local("/media/stick"), false),
    ] {
        assert_eq!(is_network_share(&location), expected, "{location:?}");
    }
}

#[test]
fn encrypted_volume_detection_and_lock_state_follow_emblems_and_dm_crypt() {
    let empty = BlockCryptoRef::default();
    assert!(is_encrypted_device(
        &["drive-harddisk", "changes-prevent-symbolic"],
        None,
        empty
    ));
    assert!(is_encrypted_device(
        &["drive-harddisk", "changes-allow"],
        None,
        empty
    ));
    assert!(is_encrypted_device(
        &["drive-harddisk-encrypted"],
        None,
        empty
    ));
    assert!(is_encrypted_device(
        &[],
        Some(gio::DriveStartStopType::Password),
        empty
    ));
    assert!(is_encrypted_device(
        &[],
        None,
        BlockCryptoRef {
            dm_uuid: Some("CRYPT-LUKS2-0123456789abcdef0123456789abcdef-vault"),
            ..empty
        }
    ));
    assert!(is_encrypted_device(
        &[],
        None,
        BlockCryptoRef {
            id_fs_usage: Some("crypto"),
            ..empty
        }
    ));
    assert!(!is_encrypted_device(&["drive-harddisk"], None, empty));

    // Padlock emblems win over mount/cleartext state.
    assert!(encrypted_device_is_locked(
        &["changes-prevent-symbolic"],
        true,
        true
    ));
    assert!(!encrypted_device_is_locked(
        &["changes-allow-symbolic"],
        false,
        false
    ));
    // Without emblems, an open cleartext mapping counts as unlocked even if unmounted.
    assert!(!encrypted_device_is_locked(
        &["drive-harddisk"],
        false,
        true
    ));
    assert!(!encrypted_device_is_locked(
        &["drive-harddisk"],
        true,
        false
    ));
    assert!(encrypted_device_is_locked(
        &["drive-harddisk"],
        false,
        false
    ));
}

#[test]
fn crypto_password_uuid_extracts_luks_uuid_and_ignores_filesystem_uuid_when_unlocked() {
    let luks = "01234567-89ab-cdef-0123-456789abcdef";
    let fs_uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    assert_eq!(
        luks_uuid_from_dm_uuid("CRYPT-LUKS2-0123456789abcdef0123456789abcdef-luks").as_deref(),
        Some(luks)
    );
    assert_eq!(
        luks_uuid_from_unix_device("/dev/mapper/luks-01234567-89ab-cdef-0123-456789abcdef")
            .as_deref(),
        Some(luks)
    );
    assert_eq!(
        normalize_luks_uuid("0123456789ABCDEF0123456789abcdef").as_deref(),
        Some(luks)
    );
    assert_eq!(normalize_luks_uuid("not-a-uuid"), None);

    // Unlocked: volume_uuid is the cleartext filesystem UUID and must NOT be returned.
    assert_eq!(
        crypto_password_uuid(Some(fs_uuid), None, BlockCryptoRef::default(), false),
        None
    );
    // Locked: volume_uuid IS the LUKS container UUID.
    assert_eq!(
        crypto_password_uuid(Some(luks), None, BlockCryptoRef::default(), true).as_deref(),
        Some(luks)
    );
    // Unlocked with dm-crypt hint: returns the LUKS container UUID.
    assert_eq!(
        crypto_password_uuid(
            Some(fs_uuid),
            None,
            BlockCryptoRef {
                dm_uuid: Some("CRYPT-LUKS1-0123456789abcdef0123456789abcdef-luks"),
                ..BlockCryptoRef::default()
            },
            false
        )
        .as_deref(),
        Some(luks)
    );
}

#[test]
fn password_drives_are_listed_only_when_orphaned() {
    let identities = vec!["/dev/sdb".to_owned()];
    assert!(password_drive_is_orphaned(
        gio::DriveStartStopType::Password,
        Some("/dev/sdc"),
        &identities,
        false
    ));
    assert!(!password_drive_is_orphaned(
        gio::DriveStartStopType::Password,
        Some("/dev/sdb"),
        &identities,
        false
    ));
    assert!(!password_drive_is_orphaned(
        gio::DriveStartStopType::Password,
        Some("/dev/sdc"),
        &identities,
        true
    ));
    assert!(!password_drive_is_orphaned(
        gio::DriveStartStopType::Shutdown,
        Some("/dev/sdc"),
        &identities,
        false
    ));
}

#[test]
fn secret_service_lookups_and_delete_errors_cover_all_gvfs_attributes() {
    let lookups = luks_password_lookups("01234567-89ab-cdef-0123-456789abcdef");
    assert_eq!(lookups.len(), 4);
    assert!(lookups.contains(&(
        "gvfs-luks-uuid",
        "01234567-89ab-cdef-0123-456789abcdef".into()
    )));
    assert!(lookups.contains(&("gvfs-luks-uuid", "0123456789abcdef0123456789abcdef".into())));
    assert!(lookups.contains(&(
        "gvfs.crypto.luks.uuid",
        "01234567-89ab-cdef-0123-456789abcdef".into()
    )));
    assert_eq!(forget_failure_from_delete_prompt("/"), Ok(()));
    assert_eq!(
        forget_failure_from_delete_prompt("/org/freedesktop/secrets/prompt/p1"),
        Err(ForgetCachedPasswordError::NeedsConfirmation)
    );
    assert_eq!(forget_failure_from_search(0), Ok(()));
    assert_eq!(
        forget_failure_from_search(2),
        Err(ForgetCachedPasswordError::ItemLocked)
    );
}

#[test]
fn udisks2_lock_follows_crypto_backing_device_and_locks_open_cleartext() {
    let mut locked_object: Option<String> = None;
    let result = lock_encrypted_device(
        Some("/dev/dm-2"),
        None,
        |path, interface, method, params| match (path, interface, method) {
            (
                "/org/freedesktop/UDisks2/Manager",
                "org.freedesktop.UDisks2.Manager",
                "ResolveDevice",
            ) => Ok((vec![
                glib::variant::ObjectPath::try_from("/org/freedesktop/UDisks2/block_devices/dm_2")
                    .expect("path"),
            ],)
                .to_variant()),
            (
                "/org/freedesktop/UDisks2/block_devices/dm_2",
                "org.freedesktop.DBus.Properties",
                "Get",
            ) => {
                let (_, prop) = params.get::<(String, String)>().expect("prop");
                assert_eq!(prop, "CryptoBackingDevice");
                let backing = glib::variant::ObjectPath::try_from(
                    "/org/freedesktop/UDisks2/block_devices/sda3",
                )
                .expect("backing");
                Ok((backing.to_variant(),).to_variant())
            }
            (
                "/org/freedesktop/UDisks2/block_devices/sda3",
                "org.freedesktop.DBus.Properties",
                "Get",
            ) => {
                let cleartext = glib::variant::ObjectPath::try_from(
                    "/org/freedesktop/UDisks2/block_devices/dm_2",
                )
                .expect("cleartext");
                Ok((cleartext.to_variant(),).to_variant())
            }
            (
                "/org/freedesktop/UDisks2/block_devices/sda3",
                "org.freedesktop.UDisks2.Encrypted",
                "Lock",
            ) => {
                locked_object = Some(path.to_owned());
                Ok(().to_variant())
            }
            other => Err(LockDeviceError::CommandFailed(format!(
                "unexpected {other:?}"
            ))),
        },
    );
    assert_eq!(result, Ok(()));
    assert_eq!(
        locked_object.as_deref(),
        Some("/org/freedesktop/UDisks2/block_devices/sda3")
    );
}

#[test]
fn already_mounted_race_and_auth_or_in_flight_errors_are_classified_accurately() {
    let already = glib::Error::new(gio::IOErrorEnum::AlreadyMounted, "already mounted");
    assert!(device_volume_mount_is_ready(&Err(already), false));
    assert!(device_volume_mount_is_ready(
        &Err(glib::Error::new(gio::IOErrorEnum::Failed, "other")),
        true
    ));
    assert!(!device_volume_mount_is_ready(
        &Err(glib::Error::new(gio::IOErrorEnum::Failed, "other")),
        false
    ));

    let bad_pass = glib::Error::new(
        gio::IOErrorEnum::Failed,
        "No key available with this passphrase.",
    );
    assert!(volume_error_is_authentication_failure(&bad_pass));
    assert!(!volume_error_is_authentication_failure(&glib::Error::new(
        gio::IOErrorEnum::Failed,
        "I/O error"
    )));

    assert!(volume_error_is_in_flight_mount(&glib::Error::new(
        gio::IOErrorEnum::Pending,
        "pending"
    )));
    assert!(volume_error_is_in_flight_mount(&glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Volume is already unlocking"
    )));

    assert_eq!(
        unlock_view_follow_up(
            Some(&Location::local("/media/vault/sub")),
            &Location::local("/media/vault"),
            true,
            true
        ),
        UnlockViewFollowUp::Reload
    );
    assert_eq!(
        unlock_view_follow_up(
            Some(&Location::local("/home/me")),
            &Location::local("/media/vault"),
            true,
            false
        ),
        UnlockViewFollowUp::Navigate
    );
    assert_eq!(
        unlock_view_follow_up(
            Some(&Location::local("/home/me")),
            &Location::local("/media/vault"),
            true,
            true
        ),
        UnlockViewFollowUp::None
    );

    assert_eq!(
        foreign_volume_wait_follow_up(
            ForeignVolumeWaitOutcome::Gone,
            false,
            VolumeSuccessorKind::Mounted
        ),
        ForeignVolumeWaitFollowUp::Navigate
    );
    assert_eq!(
        foreign_volume_wait_follow_up(
            ForeignVolumeWaitOutcome::StillLocked,
            true,
            VolumeSuccessorKind::Locked
        ),
        ForeignVolumeWaitFollowUp::Quiet
    );
}

#[test]
fn release_decisions_and_key_matching_cover_safe_to_remove_and_progress_errors() {
    let safe = decide_gio(ReleaseKind::Eject, None, Ok(()), true);
    assert!(safe.safe_to_remove);
    assert!(safe.dialog_title.is_none());

    // Lock never shows "Safe to remove".
    let locked = decide_gio(ReleaseKind::Lock, None, Ok(()), true);
    assert!(!locked.safe_to_remove);

    // Aborted / Cancelled stay quiet.
    let aborted = glib::Error::new(gio::IOErrorEnum::Failed, "Operation aborted by user");
    assert!(release_error_is_quiet(&aborted));
    let quiet = decide_gio(ReleaseKind::Unmount, None, Err(aborted), false);
    assert!(quiet.dialog_title.is_none());

    // Error appends unmount progress line when present.
    let busy = glib::Error::new(gio::IOErrorEnum::Busy, "Target is busy");
    let failed = decide_gio(
        ReleaseKind::Unmount,
        Some("1 process is using the drive"),
        Err(busy),
        false,
    );
    assert_eq!(
        failed.dialog_title.as_deref(),
        Some("Unable to unmount device")
    );
    assert!(
        failed
            .dialog_message
            .as_deref()
            .is_some_and(|m| m.contains("1 process is using the drive"))
    );
    assert!(body_for_progress(Some("Syncing")).contains("Syncing"));

    let key = ReleaseKey {
        unix_device: Some("/dev/sdb1".into()),
        uuid: Some("uuid-1".into()),
        mount_root: Some(PathBuf::from("/media/stick")),
        display_name: "Stick".into(),
    };
    assert!(device_matches(
        &key,
        &DeviceIds {
            unix_device: None,
            uuid: Some("uuid-1".into()),
            mount_root: None,
        }
    ));
    assert!(!device_matches(
        &key,
        &DeviceIds {
            unix_device: Some("/dev/sdc1".into()),
            uuid: None,
            mount_root: None,
        }
    ));
}

#[test]
fn device_context_menu_sections_match_every_device_state() {
    let mounted_root = Some(Location::local("/media/vault"));

    // 1. Locked encrypted removable volume (unmounted): Unlock + Eject.
    let locked = device_rows(
        &DeviceMenuInfo {
            id: "luks".into(),
            root: None,
            lock: Some(LockState::Locked),
            release: Some(MediaRelease::EjectVolume),
            can_mount: true,
            network: false,
            pending: false,
        },
        false,
    );
    assert_eq!(locked, vec![vec![UNLOCK, EJECT]]);

    // 2. Unlocked encrypted mounted volume: Open in New Tab | Lock, Eject, Properties.
    let unlocked_mounted = device_rows(
        &DeviceMenuInfo {
            id: "luks".into(),
            root: mounted_root.clone(),
            lock: Some(LockState::Unlocked),
            release: Some(MediaRelease::EjectVolume),
            can_mount: false,
            network: false,
            pending: false,
        },
        false,
    );
    assert_eq!(
        unlocked_mounted,
        vec![vec![OPEN_IN_TAB], vec![LOCK, EJECT, PROPERTIES]]
    );

    // 3. Unlocked encrypted unmounted volume: Lock, Eject, Mount.
    let unlocked_unmounted = device_rows(
        &DeviceMenuInfo {
            id: "luks".into(),
            root: None,
            lock: Some(LockState::Unlocked),
            release: Some(MediaRelease::EjectVolume),
            can_mount: true,
            network: false,
            pending: false,
        },
        false,
    );
    assert_eq!(unlocked_unmounted, vec![vec![LOCK, EJECT, MOUNT]]);

    // 4. Plain unmounted partition: Mount.
    let plain_unmounted = device_rows(
        &DeviceMenuInfo {
            id: "part".into(),
            root: None,
            lock: None,
            release: None,
            can_mount: true,
            network: false,
            pending: false,
        },
        false,
    );
    assert_eq!(plain_unmounted, vec![vec![MOUNT]]);

    // 5. Plain mounted non-ejectable volume: Open in New Tab | Unmount, Properties.
    let plain_mounted = device_rows(
        &DeviceMenuInfo {
            id: "part".into(),
            root: mounted_root.clone(),
            lock: None,
            release: Some(MediaRelease::UnmountMount),
            can_mount: false,
            network: false,
            pending: false,
        },
        false,
    );
    assert_eq!(
        plain_mounted,
        vec![vec![OPEN_IN_TAB], vec![UNMOUNT, PROPERTIES]]
    );

    // 6. Network share: Open in New Tab | Properties | Disconnect (danger).
    let share = device_rows(
        &DeviceMenuInfo {
            id: "smb".into(),
            root: Some(Location::uri("smb://nas/public")),
            lock: None,
            release: None,
            can_mount: false,
            network: true,
            pending: false,
        },
        false,
    );
    assert_eq!(
        share,
        vec![vec![OPEN_IN_TAB], vec![PROPERTIES], vec![DISCONNECT]]
    );

    // 7. Chooser mode omits Open in New Tab.
    let chooser_share = device_rows(
        &DeviceMenuInfo {
            id: "smb".into(),
            root: Some(Location::uri("smb://nas/public")),
            lock: None,
            release: None,
            can_mount: false,
            network: true,
            pending: false,
        },
        true,
    );
    assert_eq!(chooser_share, vec![vec![PROPERTIES], vec![DISCONNECT]]);

    // 8. Pending release shows no menu.
    let pending: Vec<Vec<MenuRow>> = device_rows(
        &DeviceMenuInfo {
            id: "part".into(),
            root: mounted_root,
            lock: Some(LockState::Unlocked),
            release: Some(MediaRelease::EjectVolume),
            can_mount: false,
            network: false,
            pending: true,
        },
        false,
    );
    assert!(pending.is_empty());
}

#[test]
fn encrypted_rows_ghost_stand_in_and_device_context_menus_work_in_hosted_sidebar() {
    hosted(
        MODULE,
        "encrypted_rows_ghost_stand_in_and_device_context_menus_work_in_hosted_sidebar",
        |mut host| {
            let folder = fixture();
            let mounted = Location::local(folder.path());
            clear_pending_for_test();
            push_pending_for_test(
                ReleaseKey {
                    unix_device: Some("/dev/gone-stick".into()),
                    uuid: None,
                    mount_root: None,
                    display_name: "Ghost USB".into(),
                },
                ReleaseKind::Eject,
            );

            host.runtime().update(|app, _| {
                seed_for_test(
                    app,
                    vec![
                        vec![
                            synthetic_device(
                                "vault-locked",
                                "31 GB Encrypted",
                                None,
                                Some(LockState::Locked),
                                Some(MediaRelease::EjectVolume),
                                true,
                                false,
                            ),
                            synthetic_device(
                                "vault-open",
                                "Unlocked Vault",
                                Some(mounted.clone()),
                                Some(LockState::Unlocked),
                                Some(MediaRelease::EjectVolume),
                                false,
                                false,
                            ),
                        ],
                        vec![synthetic_device(
                            "nas",
                            "Home on nas",
                            Some(Location::uri("smb://nas/home")),
                            None,
                            None,
                            false,
                            true,
                        )],
                    ],
                );
            });
            idle(&mut host);

            let tree = host.tree();
            // Locked volume exposes Unlock button beside Eject.
            let unlock_btn =
                keyed(&tree, "sidebar-device-vault-locked-lock").expect("locked row lock button");
            assert_eq!(text(unlock_btn, "name"), "Unlock");
            let eject_btn =
                keyed(&tree, "sidebar-device-vault-locked-eject").expect("locked row eject button");
            assert_eq!(text(eject_btn, "name"), "Eject");

            // Unlocked volume exposes Lock button beside Eject.
            let lock_btn =
                keyed(&tree, "sidebar-device-vault-open-lock").expect("unlocked row lock button");
            assert_eq!(text(lock_btn, "name"), "Lock");

            // Disappeared device mid-release keeps a stand-in ghost row without a divider.
            let groups = host.app().sidebar.devices.groups();
            let ghost_group = groups.last().expect("ghost group");
            assert!(!ghost_group.divided, "ghost row has no leading divider");
            assert_eq!(ghost_group.devices[0].label, "Ghost USB");
            assert!(ghost_group.devices[0].pending);
            let ghost_row = keyed(&tree, "sidebar-device-releasing-0").expect("ghost row in tree");
            assert_eq!(text(ghost_row, "name"), "Ghost USB");

            // Right-clicking the unlocked mounted volume opens its context menu and Properties works.
            let target = host
                .runtime()
                .elements()
                .into_iter()
                .find(|el| !el.exiting && el.key.as_deref() == Some("sidebar-device-vault-open"))
                .expect("vault-open row");
            let [x, y, w, h] = target.visible;
            let (rx, ry) = (x + w / 2.0, y + h / 2.0);
            host.runtime().pointer_move(rx, ry);
            host.runtime().secondary_down(rx, ry);
            host.turn();
            host.runtime().secondary_up(rx, ry);
            idle(&mut host);
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "sidebar-menu-menu").is_some()
            }));
            let menu_tree = host.tree();
            let items: Vec<String> = all(&menu_tree, &|n| {
                text(n, "key").starts_with("sidebar-menu-item")
            })
            .into_iter()
            .map(|n| text(n, "name"))
            .collect();
            assert_eq!(items, ["Open in New Tab", "Lock", "Eject", "Properties"]);

            // Pick Properties from the device menu and verify it opens on the mounted root.
            let props_key = all(&menu_tree, &|n| {
                text(n, "key").starts_with("sidebar-menu-item") && text(n, "name") == "Properties"
            })
            .into_iter()
            .next()
            .map(|n| text(n, "key"))
            .expect("Properties menu item key");
            let props_el = host
                .runtime()
                .elements()
                .into_iter()
                .find(|el| !el.exiting && el.key.as_deref() == Some(props_key.as_str()))
                .expect("Properties menu item");
            let [px, py, pw, ph] = props_el.visible;
            host.runtime().pointer_move(px + pw / 2.0, py + ph / 2.0);
            host.runtime().pointer_down(px + pw / 2.0, py + ph / 2.0);
            host.turn();
            host.runtime().pointer_up(px + pw / 2.0, py + ph / 2.0);
            idle(&mut host);
            assert_eq!(properties::shown_location(host.app()), Some(&mounted));

            // Close Properties, focus the locked encrypted volume row, and open its menu via Menu key.
            press(&mut host, Key::Escape, Mods::default());
            idle(&mut host);
            host.runtime().update(|_, tx| {
                tx.focus("sidebar-device-vault-locked", true);
            });
            idle(&mut host);
            press(&mut host, Key::ContextMenu, Mods::default());
            idle(&mut host);
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "sidebar-menu-menu").is_some()
            }));
            let locked_items: Vec<String> = all(&host.tree(), &|n| {
                text(n, "key").starts_with("sidebar-menu-item")
            })
            .into_iter()
            .map(|n| text(n, "name"))
            .collect();
            assert_eq!(locked_items, ["Unlock", "Eject"]);

            clear_pending_for_test();
        },
    );
}

#[test]
fn lock_and_unlock_guards_release_on_drop_even_when_window_is_gone() {
    let first_lock = LockGuard::acquire("vault-guard-test").expect("first lock guard");
    assert!(LockGuard::acquire("vault-guard-test").is_none());
    drop(first_lock);
    assert!(LockGuard::acquire("vault-guard-test").is_some());

    let inbox = Inbox::<App>::new();
    let dead_later = inbox.later().downgrade();
    drop(inbox);

    let keys = DeviceKeys::new([Some("/dev/mapper/vault-guard-test".into())], []);
    let first_unlock =
        UnlockSlotGuard::acquire(dead_later.clone(), keys.clone()).expect("first unlock guard");
    assert!(UnlockSlotGuard::acquire(dead_later.clone(), keys.clone()).is_none());
    drop(first_unlock);
    assert!(UnlockSlotGuard::acquire(dead_later, keys).is_some());
}

#[test]
fn first_key_falls_back_to_devices_heading_when_only_devices_are_shown() {
    hosted(
        MODULE,
        "first_key_falls_back_to_devices_heading_when_only_devices_are_shown",
        |mut host| {
            host.runtime().update(|app, _| {
                shell::saved(app).change(|settings| {
                    settings.sidebar_show_home = false;
                    settings.sidebar_show_trash = false;
                    settings.sidebar_show_network = false;
                    settings.sidebar_show_recent = false;
                    settings.sidebar_show_desktop = false;
                    settings.sidebar_show_documents = false;
                    settings.sidebar_show_downloads = false;
                    settings.sidebar_show_music = false;
                    settings.sidebar_show_pictures = false;
                    settings.sidebar_show_videos = false;
                });
                crate::sidebar::relist_if_started(app);
                seed_for_test(
                    app,
                    vec![vec![synthetic_device(
                        "only-dev",
                        "USB Stick",
                        None,
                        None,
                        Some(MediaRelease::EjectVolume),
                        true,
                        false,
                    )]],
                );
            });
            idle(&mut host);
            assert_eq!(
                host.app().sidebar.first_key().as_deref(),
                Some(DEVICES_HEADING)
            );
        },
    );
}

fn drive(sort_key: Option<&str>, name: &str, identity: &str) -> DeviceHome {
    DeviceHome::Drive(DriveRank {
        sort_key: sort_key.map(str::to_owned),
        name: name.to_owned(),
        identity: Some(identity.to_owned()),
    })
}

fn nvme() -> DeviceHome {
    drive(
        Some("00coldplug/00fixed/nvme0"),
        "Samsung SSD",
        "/dev/nvme0n1",
    )
}

fn usb() -> DeviceHome {
    drive(
        Some("00coldplug/12removable/sd____a"),
        "External USB 3.0",
        "/dev/sda",
    )
}

fn row(home: &DeviceHome, name: &str, identity: &str) -> (DevicePlacement, String) {
    let placement = DevicePlacement {
        home: home.clone(),
        name: name.to_owned(),
        identity: Some(identity.to_owned()),
    };
    (placement, format!("{name} {identity}"))
}

fn names(groups: Vec<Vec<String>>) -> Vec<Vec<String>> {
    groups
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .map(|row| row.split(' ').next().unwrap_or_default().to_owned())
                .collect()
        })
        .collect()
}

#[test]
fn interleaved_partitions_are_gathered_under_their_drive_fixed_disk_first() {
    let groups = group_by_drive(vec![
        row(&usb(), "T2ERP", "/dev/sda1"),
        row(&nvme(), "Windows", "/dev/nvme0n1p3"),
        row(&usb(), "Guoxue", "/dev/sda2"),
        row(&nvme(), "Data", "/dev/nvme0n1p4"),
        row(&usb(), "2TEUI", "/dev/sda3"),
    ]);

    assert_eq!(
        names(groups),
        [vec!["Data", "Windows"], vec!["2TEUI", "Guoxue", "T2ERP"]],
    );
}

#[test]
fn drives_are_kept_apart_and_ranked_by_their_system_sort_key() {
    let second_usb = drive(
        Some("00coldplug/12removable/sd____b"),
        "External USB 3.0",
        "/dev/sdb",
    );
    let hot_plugged = drive(Some("01hotplug/1793861841"), "Stick", "/dev/sdc");
    let no_key = drive(None, "Phone", "/dev/bus/usb/001");
    let groups = group_by_drive(vec![
        row(&no_key, "Camera", "mtp"),
        row(&hot_plugged, "Stick", "/dev/sdc1"),
        row(&second_usb, "Second", "/dev/sdb1"),
        row(&usb(), "First", "/dev/sda1"),
        row(&nvme(), "System", "/dev/nvme0n1p1"),
    ]);

    assert_eq!(
        names(groups),
        [
            vec!["System"],
            vec!["First"],
            vec!["Second"],
            vec!["Stick"],
            vec!["Camera"],
        ],
    );
}

#[test]
fn rows_without_a_drive_follow_every_drive_and_network_shares_come_last() {
    let groups = group_by_drive(vec![
        row(&DeviceHome::Network, "Share", "smb://nas/share"),
        row(&DeviceHome::NoDrive, "Backup", "/mnt/backup"),
        row(&usb(), "Stick", "/dev/sda1"),
        row(&DeviceHome::NoDrive, "Archive", "/mnt/archive"),
    ]);

    assert_eq!(
        names(groups),
        [vec!["Stick"], vec!["Archive", "Backup"], vec!["Share"]],
    );
}

#[test]
fn volumes_sharing_a_name_keep_a_stable_natural_device_order() {
    let in_order = vec![
        row(&nvme(), "129 GB Volume", "/dev/nvme0n1p2"),
        row(&nvme(), "129 GB Volume", "/dev/nvme0n1p10"),
    ];
    let reversed = vec![
        row(&nvme(), "129 GB Volume", "/dev/nvme0n1p10"),
        row(&nvme(), "129 GB Volume", "/dev/nvme0n1p2"),
    ];

    let expected = [vec![
        "129 GB Volume /dev/nvme0n1p2".to_owned(),
        "129 GB Volume /dev/nvme0n1p10".to_owned(),
    ]];
    assert_eq!(group_by_drive(in_order), expected);
    assert_eq!(group_by_drive(reversed), expected);
}

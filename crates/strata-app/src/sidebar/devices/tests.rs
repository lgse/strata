// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use strata_core::model::Location;

use super::listing::{device_identity, is_network_share, is_system_loop, label_identity};
use super::order::{DeviceHome, DevicePlacement, DriveRank, group_by_drive};
use super::{MediaRelease, display_name, leaves, release_action};

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

// The ordering cases of GTK's `src/ui/window/device_order/tests.rs`.

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

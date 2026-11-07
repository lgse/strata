// SPDX-License-Identifier: MIT

use super::*;

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

// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn mounted_usage_does_not_require_directory_read_permission() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().expect("mounted usage fixture");
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o111))
        .expect("search-only directory");
    let usage = usage_for_path(root.path());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
        .expect("restore fixture permissions");
    let (total, available) = usage.expect("filesystem usage without directory enumeration");
    assert!(total > 0);
    assert!(available <= total);
}

#[test]
fn formatting_resolves_only_the_selected_volume_and_updates_partition_discovery() {
    for fs in [
        FilesystemType::Fat32,
        FilesystemType::Ntfs,
        FilesystemType::Exfat,
    ] {
        for quick in [true, false] {
            let mut methods = Vec::new();
            format_device(
                Path::new("/dev/test-partition"),
                fs,
                "Backup",
                quick,
                |path, interface, method, parameters| {
                    methods.push(method.to_owned());
                    match method {
                        "ResolveDevice" => {
                            assert_eq!(interface, "org.freedesktop.UDisks2.Manager");
                            assert_eq!(parameters.type_().as_str(), "(a{sv}a{sv})");
                            let spec = parameters.child_value(0);
                            let spec = glib::VariantDict::new(Some(&spec));
                            assert_eq!(
                                spec.lookup::<String>("path")
                                    .expect("device path variant")
                                    .as_deref(),
                                Some("/dev/test-partition")
                            );
                            Ok((vec![
                                glib::variant::ObjectPath::try_from(
                                    "/org/freedesktop/UDisks2/block_devices/test_partition",
                                )
                                .expect("valid UDisks object path"),
                            ],)
                                .to_variant())
                        }
                        "Format" => {
                            assert_eq!(
                                path,
                                "/org/freedesktop/UDisks2/block_devices/test_partition"
                            );
                            assert_eq!(interface, "org.freedesktop.UDisks2.Block");
                            assert_eq!(parameters.type_().as_str(), "(sa{sv})");
                            let kind = parameters
                                .child_value(0)
                                .get::<String>()
                                .expect("filesystem type");
                            let options = parameters.child_value(1);
                            assert_eq!(
                                kind,
                                match fs {
                                    FilesystemType::Fat32 => "vfat",
                                    FilesystemType::Ntfs => "ntfs",
                                    FilesystemType::Exfat => "exfat",
                                }
                            );
                            let options = glib::VariantDict::new(Some(&options));
                            assert_eq!(
                                options
                                    .lookup::<bool>("update-partition-type")
                                    .expect("partition-update option"),
                                Some(true)
                            );
                            assert_eq!(
                                options
                                    .lookup::<String>("label")
                                    .expect("label option")
                                    .as_deref(),
                                Some("Backup")
                            );
                            assert_eq!(
                                options
                                    .lookup::<String>("erase")
                                    .expect("erase option")
                                    .as_deref(),
                                if quick { None } else { Some("zero") }
                            );
                            assert_eq!(
                                options
                                    .lookup::<Vec<String>>("mkfs-args")
                                    .expect("mkfs arguments"),
                                if fs == FilesystemType::Fat32 {
                                    Some(vec!["-F".into(), "32".into()])
                                } else {
                                    None
                                }
                            );
                            Ok(().to_variant())
                        }
                        _ => panic!("unexpected storage operation"),
                    }
                },
            )
            .expect("format fixture succeeds");
            assert_eq!(methods, ["ResolveDevice", "Format"]);
        }
    }
}

#[test]
fn missing_or_ambiguous_device_never_formats() {
    for paths in [vec![], vec!["/one", "/two"]] {
        let result = format_device(
            Path::new("/dev/missing"),
            FilesystemType::Exfat,
            "",
            true,
            |_, _, method, _| {
                assert_eq!(method, "ResolveDevice");
                Ok((paths
                    .iter()
                    .map(|path| {
                        glib::variant::ObjectPath::try_from(*path).expect("fixture object path")
                    })
                    .collect::<Vec<_>>(),)
                    .to_variant())
            },
        );
        assert!(matches!(result, Err(DriveOpError::DeviceNotFound)));
    }
}

#[test]
fn format_failure_is_not_reported_as_success_or_retried() {
    let mut calls = 0;
    let result = format_device(
        Path::new("/dev/test"),
        FilesystemType::Exfat,
        "",
        true,
        |_, _, method, _| {
            calls += 1;
            if method == "ResolveDevice" {
                Ok((vec![
                    glib::variant::ObjectPath::try_from("/test").expect("fixture object path"),
                ],)
                    .to_variant())
            } else {
                Err(DriveOpError::CommandFailed("format failed".into()))
            }
        },
    );
    assert_eq!(calls, 2);
    assert!(
        matches!(result, Err(DriveOpError::CommandFailed(message)) if message == "format failed")
    );
}

const CONTAINER: &str = "/org/freedesktop/UDisks2/block_devices/sda1";
const MAPPING: &str = "/org/freedesktop/UDisks2/block_devices/dm_2d0";
const PLAIN: &str = "/org/freedesktop/UDisks2/block_devices/sda2";

#[derive(Clone, Copy)]
enum ContainerState {
    Open,
    Closed,
    Unreachable,
}

/// A UDisks fixture with one LUKS container, its mapping and a plain block.
/// Returns the object each `Lock` call targeted.
fn lock_with_fixture(
    device: Option<&str>,
    luks_uuid: Option<&str>,
    container: ContainerState,
) -> (Result<(), DriveOpError>, Vec<String>) {
    let mut locked = Vec::new();
    let object_path =
        |path: &str| glib::variant::ObjectPath::try_from(path).expect("fixture object path");
    let result =
        lock_encrypted_device(
            device,
            luks_uuid,
            |path, interface, method, parameters| match (interface, method) {
                ("org.freedesktop.UDisks2.Manager", "ResolveDevice") => {
                    let spec = glib::VariantDict::new(Some(&parameters.child_value(0)));
                    let by_path = spec.lookup::<String>("path").expect("path variant");
                    let by_uuid = spec.lookup::<String>("uuid").expect("uuid variant");
                    let found = match (by_path.as_deref(), by_uuid.as_deref()) {
                        (Some("/dev/sda1"), None) | (None, Some("luks-uuid")) => vec![CONTAINER],
                        (Some("/dev/dm-0"), None) => vec![MAPPING],
                        (Some("/dev/sda2"), None) => vec![PLAIN],
                        _ => vec![],
                    };
                    Ok((found.into_iter().map(object_path).collect::<Vec<_>>(),).to_variant())
                }
                ("org.freedesktop.DBus.Properties", "Get") => {
                    let (interface, property) = parameters
                        .get::<(String, String)>()
                        .expect("property request");
                    let value = match (path, interface.as_str(), property.as_str()) {
                        (MAPPING, "org.freedesktop.UDisks2.Block", "CryptoBackingDevice") => {
                            CONTAINER
                        }
                        (_, "org.freedesktop.UDisks2.Block", "CryptoBackingDevice") => "/",
                        (CONTAINER, "org.freedesktop.UDisks2.Encrypted", "CleartextDevice") => {
                            match container {
                                ContainerState::Open => MAPPING,
                                ContainerState::Closed => "/",
                                ContainerState::Unreachable => {
                                    return Err(DriveOpError::CommandFailed(
                                        "Timeout was reached".into(),
                                    ));
                                }
                            }
                        }
                        _ => {
                            return Err(DriveOpError::MissingInterface(
                                "No such interface on object".into(),
                            ));
                        }
                    };
                    Ok((object_path(value).to_variant(),).to_variant())
                }
                ("org.freedesktop.UDisks2.Encrypted", "Lock") => {
                    assert_eq!(parameters.type_().as_str(), "(a{sv})");
                    locked.push(path.to_owned());
                    Ok(().to_variant())
                }
                _ => panic!("unexpected storage operation {interface}.{method}"),
            },
        );
    (result, locked)
}

#[test]
fn lock_closes_the_container_whether_named_directly_or_through_its_mapping() {
    for (device, luks_uuid) in [
        (Some("/dev/sda1"), None),
        (Some("/dev/dm-0"), None),
        (None, Some("luks-uuid")),
        (Some("/dev/unknown"), Some("luks-uuid")),
        (Some("/dev/sda2"), Some("luks-uuid")),
    ] {
        let (result, locked) = lock_with_fixture(device, luks_uuid, ContainerState::Open);
        assert!(result.is_ok(), "{device:?} {luks_uuid:?}: {result:?}");
        assert_eq!(locked, [CONTAINER], "{device:?} {luks_uuid:?}");
    }
}

#[test]
fn lock_never_targets_a_block_without_an_encrypted_container() {
    for device in ["/dev/sda2", "/dev/unknown"] {
        let (result, locked) = lock_with_fixture(Some(device), None, ContainerState::Open);
        assert!(
            matches!(result, Err(DriveOpError::DeviceNotFound)),
            "{device}"
        );
        assert!(locked.is_empty(), "{device}");
    }
}

#[test]
fn lock_of_an_already_closed_container_succeeds_without_calling_udisks_lock() {
    let (result, locked) = lock_with_fixture(Some("/dev/sda1"), None, ContainerState::Closed);
    assert!(result.is_ok());
    assert!(locked.is_empty());
}

#[test]
fn lock_reports_a_failed_container_read_instead_of_a_missing_device() {
    for device in ["/dev/sda1", "/dev/dm-0"] {
        let (result, locked) = lock_with_fixture(Some(device), None, ContainerState::Unreachable);
        assert!(
            matches!(&result, Err(DriveOpError::CommandFailed(message)) if message == "Timeout was reached"),
            "{device}: {result:?}"
        );
        assert!(locked.is_empty(), "{device}");
    }
}

#[test]
fn dismissed_lock_authorization_is_quiet_and_other_failures_keep_their_reason() {
    assert!(matches!(
        lock_error(gio::DBusError::new_for_dbus_error(
            "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed",
            "Not authorized",
        )),
        DriveOpError::Cancelled
    ));
    assert!(matches!(
        lock_error(gio::DBusError::new_for_dbus_error(
            "org.freedesktop.UDisks2.Error.Failed",
            "Device /dev/sda1 is busy",
        )),
        DriveOpError::CommandFailed(message) if message == "Device /dev/sda1 is busy"
    ));
    assert!(matches!(
        lock_error(gio::DBusError::new_for_dbus_error(
            "org.freedesktop.DBus.Error.InvalidArgs",
            "No such interface “org.freedesktop.UDisks2.Encrypted”",
        )),
        DriveOpError::MissingInterface(message)
            if message == "No such interface “org.freedesktop.UDisks2.Encrypted”"
    ));
}

#[test]
fn tool_path_finds_shell_and_rejects_missing_tools() {
    assert!(tool_path("sh").is_some());
    assert!(tool_path("strata-definitely-missing-tool").is_none());
}

#[test]
fn filesystem_label_limits_match_backends() {
    assert_eq!(FilesystemType::Fat32.max_label_len(), 11);
    assert_eq!(FilesystemType::Ntfs.max_label_len(), 32);
    assert_eq!(FilesystemType::Exfat.max_label_len(), 11);
}

#[test]
fn labels_fit_in_the_units_each_backend_counts() {
    let emoji = |count| "😀".repeat(count);
    for (fs, label, fits) in [
        (FilesystemType::Exfat, "日本語のラベル".to_owned(), true),
        (FilesystemType::Exfat, emoji(5), true),
        (FilesystemType::Exfat, emoji(6), false),
        (FilesystemType::Fat32, "ARCH_202609".to_owned(), true),
        (FilesystemType::Fat32, "ÉTIQUETTE12".to_owned(), false),
        (FilesystemType::Ntfs, emoji(16), true),
        (FilesystemType::Ntfs, emoji(17), false),
    ] {
        assert_eq!(fs.label_fits(&label), fits, "{} {label}", fs.label());
    }
}

#[test]
fn ineligible_without_removable_drive() {
    assert!(!is_eligible(None));
    assert!(!show_mount(None));
}

#[test]
fn label_characters_follow_the_filesystem_rules() {
    for character in "*?.,;:/\\|+=<>[]\"".chars() {
        let label = format!("A{character}B");
        assert!(
            FilesystemType::Fat32
                .label_character_error(&label)
                .is_some(),
            "{label}"
        );
    }
    for character in "*?:/\\|<>\"".chars() {
        let label = format!("A{character}B");
        assert!(
            FilesystemType::Exfat
                .label_character_error(&label)
                .is_some(),
            "{label}"
        );
    }
    for filesystem in [
        FilesystemType::Fat32,
        FilesystemType::Exfat,
        FilesystemType::Ntfs,
    ] {
        assert!(filesystem.label_character_error("BACKUP").is_none());
        assert!(filesystem.label_character_error("A\u{0001}B").is_some());
        assert!(filesystem.label_character_error("A\0B").is_some());
    }
    assert!(
        FilesystemType::Exfat
            .label_character_error("BACKUP.2026")
            .is_none()
    );
    assert!(
        FilesystemType::Ntfs
            .label_character_error("BACKUP.2026")
            .is_none()
    );
}

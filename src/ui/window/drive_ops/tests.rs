// SPDX-License-Identifier: MIT

use super::*;

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
                                spec.lookup::<String>("path").unwrap().as_deref(),
                                Some("/dev/test-partition")
                            );
                            Ok((vec![
                                glib::variant::ObjectPath::try_from(
                                    "/org/freedesktop/UDisks2/block_devices/test_partition",
                                )
                                .unwrap(),
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
                            let kind = parameters.child_value(0).get::<String>().unwrap();
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
                                options.lookup::<bool>("update-partition-type").unwrap(),
                                Some(true)
                            );
                            assert_eq!(
                                options.lookup::<String>("label").unwrap().as_deref(),
                                Some("Backup")
                            );
                            assert_eq!(
                                options.lookup::<String>("erase").unwrap().as_deref(),
                                if quick { None } else { Some("zero") }
                            );
                            assert_eq!(
                                options.lookup::<Vec<String>>("mkfs-args").unwrap(),
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
            .unwrap();
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
                    .map(|path| glib::variant::ObjectPath::try_from(*path).unwrap())
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
                Ok((vec![glib::variant::ObjectPath::try_from("/test").unwrap()],).to_variant())
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

#[test]
fn tool_path_finds_shell_and_rejects_missing_tools() {
    assert!(tool_path("sh").is_some());
    assert!(tool_path("strata-definitely-missing-tool").is_none());
}

#[test]
fn filesystem_label_limits_match_backends() {
    assert_eq!(FilesystemType::Fat32.max_label_len(), 11);
    assert_eq!(FilesystemType::Ntfs.max_label_len(), 32);
    assert_eq!(FilesystemType::Exfat.max_label_len(), 32);
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

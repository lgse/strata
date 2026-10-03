// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn format_args_map_quick_and_full_per_filesystem() {
    assert_eq!(
        FilesystemType::Fat32.format_args("", true),
        vec!["-F", "32"]
    );
    assert_eq!(
        FilesystemType::Fat32.format_args("", false),
        vec!["-F", "32", "-c"]
    );
    assert_eq!(
        FilesystemType::Fat32.format_args("Stick", true),
        vec!["-F", "32", "-n", "Stick"]
    );
    assert_eq!(FilesystemType::Ntfs.format_args("", true), vec!["--quick"]);
    assert_eq!(
        FilesystemType::Ntfs.format_args("Stick", false),
        vec!["-L", "Stick"]
    );
    assert!(FilesystemType::Exfat.format_args("", true).is_empty());
    assert_eq!(FilesystemType::Exfat.format_args("", false), vec!["-f"]);
    assert_eq!(
        FilesystemType::Exfat.format_args("Stick", true),
        vec!["-n", "Stick"]
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

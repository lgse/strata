// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn os_release_selects_native_manager_before_ancestry() {
    for (release, expected) in [
        ("ID=arch\n", Some(PackageManager::Pacman)),
        ("ID=omarchy\nID_LIKE=arch\n", Some(PackageManager::Pacman)),
        ("ID=debian\n", Some(PackageManager::Apt)),
        ("ID=ubuntu\nID_LIKE=debian\n", Some(PackageManager::Apt)),
        ("ID=fedora\nID_LIKE=debian\n", Some(PackageManager::Dnf)),
        (
            "ID=opensuse-tumbleweed\nID_LIKE=\"suse opensuse\"\n",
            Some(PackageManager::Zypper),
        ),
        (
            "ID=custom\nID_LIKE='unknown debian'\n",
            Some(PackageManager::Apt),
        ),
        ("# ID=arch\nID=custom\nID_LIKE=other\n", None),
        ("NAME=Arch\n", None),
        ("", None),
    ] {
        assert_eq!(
            PackageManager::from_os_release(release),
            expected,
            "{release}"
        );
    }
}

#[test]
fn install_commands_deduplicate_packages_and_reject_shell_syntax() {
    for (manager, expected) in [
        (
            PackageManager::Pacman,
            "sudo pacman -S --needed dosfstools ntfs-3g",
        ),
        (PackageManager::Apt, "sudo apt install dosfstools ntfs-3g"),
        (PackageManager::Dnf, "sudo dnf install dosfstools ntfs-3g"),
        (
            PackageManager::Zypper,
            "sudo zypper install dosfstools ntfs-3g",
        ),
    ] {
        assert_eq!(
            manager
                .install_command(&["dosfstools", "ntfs-3g", "dosfstools"])
                .as_deref(),
            Some(expected)
        );
        assert!(manager.install_command(&[]).is_none());
        for package in ["", "--help", "a b", "a;reboot", "$(reboot)", "a\nb", "'a'"] {
            assert!(manager.install_command(&[package]).is_none(), "{package}");
        }
    }
}

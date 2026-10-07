// SPDX-License-Identifier: MIT

use std::fs;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PackageManager {
    Pacman,
    Apt,
    Dnf,
    Zypper,
}

impl PackageManager {
    pub(crate) fn detect() -> Option<Self> {
        let release = fs::read_to_string("/etc/os-release")
            .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
            .ok()?;
        Self::from_os_release(&release)
    }

    fn from_os_release(release: &str) -> Option<Self> {
        let value = |key: &str| {
            release.lines().find_map(|line| {
                let (name, value) = line.trim().split_once('=')?;
                (name == key).then(|| value.trim().trim_matches(['\"', '\'']))
            })
        };
        let manager_for = |id: &str| match id {
            "arch" | "omarchy" | "manjaro" | "endeavouros" => Some(Self::Pacman),
            "debian" | "ubuntu" | "linuxmint" | "pop" => Some(Self::Apt),
            "fedora" | "rhel" | "centos" | "rocky" | "almalinux" => Some(Self::Dnf),
            "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" | "suse" => {
                Some(Self::Zypper)
            }
            _ => None,
        };
        value("ID")
            .and_then(manager_for)
            .or_else(|| value("ID_LIKE")?.split_whitespace().find_map(manager_for))
    }

    pub(crate) fn install_command(self, packages: &[&str]) -> Option<String> {
        let mut unique = Vec::new();
        for package in packages {
            // Commands are copied to a shell, so reject options and shell syntax.
            if package.is_empty()
                || !package.starts_with(|c: char| c.is_ascii_alphanumeric())
                || !package
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".+_-".contains(c))
            {
                return None;
            }
            if !unique.contains(package) {
                unique.push(*package);
            }
        }
        if unique.is_empty() {
            return None;
        }
        let prefix = match self {
            Self::Pacman => "sudo pacman -S --needed",
            Self::Apt => "sudo apt install",
            Self::Dnf => "sudo dnf install",
            Self::Zypper => "sudo zypper install",
        };
        Some(format!("{prefix} {}", unique.join(" ")))
    }
}

#[cfg(test)]
mod tests;

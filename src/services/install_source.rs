// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::Deserialize;

/// Path of the packaging marker relative to the install prefix, so a package
/// installed somewhere other than `/usr` is still recognised.
const MARKER_RELATIVE_PATH: &str = "share/strata/install-source.toml";
const MARKER_SYSTEM_PATH: &str = "/usr/share/strata/install-source.toml";

const UNNAMED_MANAGER: &str = "your package manager";

/// How the running Strata was installed.
///
/// Distribution packages own `/usr/bin/strata`, so the in-app updater must not
/// replace it: `pacman -Qkk` would report the package as modified, and the next
/// package update would silently overwrite whatever the updater installed.
/// Packages declare themselves by installing a marker file; its absence means an
/// ordinary user-owned install that the updater is free to replace.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum InstallSource {
    /// User-owned. The in-app updater owns the binary.
    #[default]
    SelfManaged,
    /// Package-manager owned. The updater must defer to the package manager.
    Managed(ManagedInstall),
}

/// The packaging marker's contents.
///
/// Every field is optional and unknown keys are ignored, so a package built for
/// a newer Strata never fails to parse on an older binary -- it degrades to less
/// specific guidance instead.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ManagedInstall {
    manager: Option<String>,
    package: Option<String>,
    channel: Option<String>,
    update_command: Option<String>,
    alternate_package: Option<String>,
}

impl InstallSource {
    /// The install source of the running binary, resolved once per process.
    pub fn detect() -> &'static Self {
        static DETECTED: OnceLock<InstallSource> = OnceLock::new();
        DETECTED.get_or_init(|| match marker_path() {
            Some(path) => Self::load(&path),
            None => Self::SelfManaged,
        })
    }

    /// Reads `marker`, treating its presence as authoritative.
    ///
    /// A marker that exists but cannot be read or parsed still means a packaged
    /// install, so a corrupt file degrades to generic guidance rather than
    /// re-enabling an in-place update over package-owned files.
    fn load(marker: &Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(marker) else {
            return Self::SelfManaged;
        };
        match toml::from_str::<ManagedInstall>(&contents) {
            Ok(managed) => Self::Managed(managed.normalized()),
            Err(error) => {
                tracing::warn!("could not parse {}: {error}", marker.display());
                Self::Managed(ManagedInstall::default())
            }
        }
    }

    pub fn managed(&self) -> Option<&ManagedInstall> {
        match self {
            Self::SelfManaged => None,
            Self::Managed(managed) => Some(managed),
        }
    }

    pub fn is_managed(&self) -> bool {
        self.managed().is_some()
    }
}

impl ManagedInstall {
    /// The package manager's name, or a generic stand-in when the marker
    /// omitted it.
    pub fn manager(&self) -> &str {
        self.manager.as_deref().unwrap_or(UNNAMED_MANAGER)
    }

    pub fn package(&self) -> Option<&str> {
        self.package.as_deref()
    }

    pub fn channel(&self) -> Option<&str> {
        self.channel.as_deref()
    }

    pub fn alternate_package(&self) -> Option<&str> {
        self.alternate_package.as_deref()
    }

    /// One sentence naming who owns this install.
    pub fn ownership_summary(&self) -> String {
        match self.package() {
            Some(package) => format!("Installed by {} as {package}.", self.manager()),
            None => format!("Installed by {}.", self.manager()),
        }
    }

    /// How the user should update, preferring the packaged command over a
    /// generic instruction.
    pub fn update_instruction(&self) -> String {
        match self.update_command.as_deref() {
            Some(command) => format!("Update Strata with: {command}"),
            None => format!("Update Strata through {}.", self.manager()),
        }
    }

    /// How to reach the other release channel, when the package names a
    /// sibling that tracks it.
    pub fn alternate_instruction(&self) -> Option<String> {
        let alternate = self.alternate_package()?;
        Some(format!(
            "Other release channels are published as {alternate}."
        ))
    }

    /// Discards empty strings so a marker written with blank placeholders
    /// behaves the same as one that omitted the keys.
    fn normalized(self) -> Self {
        fn present(value: Option<String>) -> Option<String> {
            value.filter(|value| !value.trim().is_empty())
        }

        Self {
            manager: present(self.manager),
            package: present(self.package),
            channel: present(self.channel),
            update_command: present(self.update_command),
            alternate_package: present(self.alternate_package),
        }
    }
}

/// Locates the packaging marker for the running binary.
///
/// The prefix-relative path is checked first so a package installed under a
/// prefix other than `/usr` is still detected, with the system path as a
/// fallback for the case where the executable was reached through a symlink
/// outside its own prefix.
fn marker_path() -> Option<PathBuf> {
    let prefix_relative = std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(marker_path_for_executable)
        .filter(|path| path.is_file());
    if prefix_relative.is_some() {
        return prefix_relative;
    }

    let system = PathBuf::from(MARKER_SYSTEM_PATH);
    system.is_file().then_some(system)
}

/// Resolves `<prefix>/share/strata/install-source.toml` for an executable at
/// `<prefix>/bin/strata`.
fn marker_path_for_executable(executable: &Path) -> Option<PathBuf> {
    Some(executable.parent()?.parent()?.join(MARKER_RELATIVE_PATH))
}

/// Rejects an in-place install when a package manager owns the binary.
pub(crate) fn ensure_self_managed(source: &InstallSource) -> Result<(), String> {
    match source.managed() {
        None => Ok(()),
        Some(managed) => Err(format!(
            "{} {}",
            managed.ownership_summary(),
            managed.update_instruction()
        )),
    }
}

#[cfg(test)]
mod tests;

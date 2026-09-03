// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::Deserialize;

use crate::services::Channel;

/// Path of the packaging marker relative to the install prefix, so a package
/// installed somewhere other than `/usr` is still recognised.
const MARKER_RELATIVE_PATH: &str = "share/strata/install-source.toml";

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
    /// Candidate AUR helpers, most preferred first. pacman cannot update an
    /// AUR package -- `pacman -Syu strata-bin` fails with "target not found"
    /// because no configured repository carries it -- so the command depends
    /// on which helper the user actually has. The marker lists candidates
    /// rather than naming one, leaving the choice to whatever is installed.
    #[serde(default)]
    aur_helpers: Vec<String>,
    alternate_package: Option<String>,
}

impl InstallSource {
    /// The install source of the running binary, resolved once per process.
    pub fn detect() -> &'static Self {
        static DETECTED: OnceLock<InstallSource> = OnceLock::new();
        DETECTED.get_or_init(|| Self::from_marker_path(marker_path()))
    }

    /// Classifies an install from the marker `marker_path` located, if any.
    ///
    /// Absence is decided here rather than in [`Self::load`], which is only
    /// ever handed a path already confirmed to exist -- that is what lets an
    /// unreadable marker fail safe instead of being mistaken for no marker.
    fn from_marker_path(marker: Option<PathBuf>) -> Self {
        match marker {
            Some(path) => Self::load(&path),
            None => Self::SelfManaged,
        }
    }

    /// Reads an existing `marker`, treating its presence as authoritative.
    ///
    /// A marker that cannot be read or parsed still means a packaged install,
    /// so a corrupt file degrades to generic guidance rather than re-enabling
    /// an in-place update over package-owned files.
    fn load(marker: &Path) -> Self {
        let contents = match std::fs::read_to_string(marker) {
            Ok(contents) => contents,
            Err(error) => {
                // The caller already confirmed the file exists, so this is a
                // marker that is present but unreadable -- a truncated write
                // leaving invalid UTF-8, or a tightened mode. Treating that as
                // self-managed would re-enable a rename over package-owned
                // files, which is the one outcome this type exists to prevent.
                tracing::warn!("could not read {}: {error}", marker.display());
                return Self::Managed(ManagedInstall::default());
            }
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

    /// How the user should update: an explicit packaged command when the
    /// marker names one, otherwise the first listed AUR helper that is
    /// actually installed.
    pub fn update_instruction(&self) -> String {
        self.update_instruction_with(on_path)
    }

    /// [`Self::update_instruction`] against an injected PATH lookup, so the
    /// wording can be asserted without depending on what the test machine has
    /// installed.
    fn update_instruction_with(&self, available: impl Fn(&str) -> bool) -> String {
        if let Some(command) = self.update_command.as_deref() {
            return format!("Update Strata with: {command}");
        }
        if let Some(package) = self.package() {
            if let Some(helper) = self.aur_helpers.iter().find(|helper| available(helper)) {
                return format!("Update Strata with: {helper} -S {package}");
            }
            if let Some(helper) = self.aur_helpers.first() {
                return format!(
                    "Update Strata with an AUR helper, for example: {helper} -S {package}"
                );
            }
        }
        format!("Update Strata through {}.", self.manager())
    }

    /// The app release channel this package tracks.
    ///
    /// The marker names the packaging channel, which follows the AUR package
    /// names (`stable`, `rc`) and does not match the persisted [`Channel`]
    /// values, so the mapping is spelled out rather than delegated to
    /// [`Channel::parse`] -- which fails closed to Stable and would silently
    /// mis-map `rc`.
    pub fn tracked_channel(&self) -> Option<Channel> {
        match self.channel()? {
            "stable" => Some(Channel::Stable),
            "rc" | "preview" => Some(Channel::Preview),
            "nightly" => Some(Channel::Nightly),
            _ => None,
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
            aur_helpers: self
                .aur_helpers
                .into_iter()
                .filter(|helper| !helper.trim().is_empty())
                .collect(),
            alternate_package: present(self.alternate_package),
        }
    }
}

/// Locates the packaging marker for the running binary.
///
/// Resolved only relative to the running executable's own prefix, so a
/// package installed under a prefix other than `/usr` is still detected and,
/// more importantly, a manual `~/.local/bin/strata` on a machine that also
/// has the distribution package installed is not mistaken for the packaged
/// one. `current_exe` resolves through `/proc/self/exe` on Linux, so the
/// prefix here is always the real install prefix rather than a symlink's.
fn marker_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(marker_path_for_executable)
        .filter(|path| path.is_file())
}

/// Whether `program` is executable somewhere on `PATH`.
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| directory.join(program).is_file())
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

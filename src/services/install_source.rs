// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::Deserialize;

use crate::services::Channel;

const MARKER_RELATIVE_PATH: &str = "share/strata/install-source.toml";

const UNNAMED_MANAGER: &str = "your package manager";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum InstallSource {
    #[default]
    SelfManaged,
    Managed(ManagedInstall),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ManagedInstall {
    manager: Option<String>,
    package: Option<String>,
    channel: Option<String>,
    update_command: Option<String>,
    #[serde(default)]
    aur_helpers: Vec<String>,
    alternate_package: Option<String>,
}

impl InstallSource {
    pub fn detect() -> &'static Self {
        static DETECTED: OnceLock<InstallSource> = OnceLock::new();
        DETECTED.get_or_init(|| Self::from_marker_path(marker_path()))
    }

    fn from_marker_path(marker: Option<PathBuf>) -> Self {
        match marker {
            Some(path) => Self::load(&path),
            None => Self::SelfManaged,
        }
    }

    fn load(marker: &Path) -> Self {
        let contents = match std::fs::read_to_string(marker) {
            Ok(contents) => contents,
            Err(error) => {
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

    pub fn ownership_summary(&self) -> String {
        match self.package() {
            Some(package) => format!("Installed by {} as {package}.", self.manager()),
            None => format!("Installed by {}.", self.manager()),
        }
    }

    pub fn update_instruction(&self) -> String {
        self.update_instruction_with(on_path)
    }

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

    pub fn tracked_channel(&self) -> Option<Channel> {
        match self.channel()? {
            "stable" => Some(Channel::Stable),
            "rc" | "preview" => Some(Channel::Preview),
            "nightly" => Some(Channel::Nightly),
            _ => None,
        }
    }

    pub fn alternate_instruction(&self) -> Option<String> {
        let alternate = self.alternate_package()?;
        Some(format!(
            "Other release channels are published as {alternate}."
        ))
    }

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

fn marker_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(marker_path_for_executable)
        .filter(|path| path.is_file())
}

fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| directory.join(program).is_file())
}

fn marker_path_for_executable(executable: &Path) -> Option<PathBuf> {
    Some(executable.parent()?.parent()?.join(MARKER_RELATIVE_PATH))
}

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

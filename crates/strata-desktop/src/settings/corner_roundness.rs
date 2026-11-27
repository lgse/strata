// SPDX-License-Identifier: MIT

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer, Serialize};

const OS_RELEASE_PATH: &str = "/etc/os-release";
const SYSTEM_OMARCHY_VERSION_PATH: &str = "/usr/share/omarchy/version";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CornerRoundness(u32);

impl CornerRoundness {
    pub const MIN: u32 = 0;
    pub const MAX: u32 = 200;
    pub const STEP: u32 = 10;
    pub const SQUARE: Self = Self(0);
    pub const DEFAULT: Self = Self(60);

    pub fn new(percent: u32) -> Self {
        Self(percent.clamp(Self::MIN, Self::MAX))
    }

    pub fn percent(self) -> u32 {
        self.0
    }

    pub fn scale(self) -> f64 {
        f64::from(self.0) / 100.0
    }
}

impl Default for CornerRoundness {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Default corner roundness when omitted from `settings.toml`: square on Omarchy, rounded elsewhere.
pub fn default_corner_roundness() -> CornerRoundness {
    let user_version =
        env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/omarchy/version"));
    default_corner_roundness_for(
        Path::new(OS_RELEASE_PATH),
        Path::new(SYSTEM_OMARCHY_VERSION_PATH),
        user_version.as_deref(),
    )
}

fn default_corner_roundness_for(
    os_release: &Path,
    system_version: &Path,
    user_version: Option<&Path>,
) -> CornerRoundness {
    if is_omarchy(os_release, system_version, user_version) {
        CornerRoundness::SQUARE
    } else {
        CornerRoundness::DEFAULT
    }
}

fn is_omarchy(os_release: &Path, system_version: &Path, user_version: Option<&Path>) -> bool {
    os_release_is_omarchy(os_release)
        || system_version.is_file()
        || user_version.is_some_and(Path::is_file)
}

fn os_release_is_omarchy(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|contents| {
        contents.lines().any(|line| {
            line.strip_prefix("ID=").is_some_and(|value| {
                value
                    .trim()
                    .trim_matches(|character| character == '\'' || character == '"')
                    .eq_ignore_ascii_case("omarchy")
            })
        })
    })
}

impl<'de> Deserialize<'de> for CornerRoundness {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let percent = i64::deserialize(deserializer)?;
        Ok(Self::new(percent.clamp(0, i64::from(Self::MAX)) as u32))
    }
}

#[cfg(test)]
mod tests;

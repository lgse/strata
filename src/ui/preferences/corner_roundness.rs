// SPDX-License-Identifier: MIT

use serde::{Deserialize, Deserializer, Serialize};

/// Corner radius scale in percent of the stylesheet's radii; 0 squares every
/// rounded rectangle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CornerRoundness(u32);

impl CornerRoundness {
    pub const MIN: u32 = 0;
    pub const MAX: u32 = 200;
    pub const STEP: u32 = 10;
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

impl<'de> Deserialize<'de> for CornerRoundness {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let percent = i64::deserialize(deserializer)?;
        Ok(Self::new(percent.clamp(0, i64::from(Self::MAX)) as u32))
    }
}

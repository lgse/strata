// SPDX-License-Identifier: MIT

use serde::{Deserialize, Deserializer, Serialize};

pub const MIN_SCROLLBAR_HOVER_WIDTH: u32 = 6;
pub const MAX_SCROLLBAR_HOVER_WIDTH: u32 = 24;
pub const DEFAULT_SCROLLBAR_HOVER_WIDTH: u32 = 8;

/// Scrollbar thickness in logical pixels when hovered or dragged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ScrollbarHoverWidth(u32);

impl ScrollbarHoverWidth {
    pub const MIN: u32 = MIN_SCROLLBAR_HOVER_WIDTH;
    pub const MAX: u32 = MAX_SCROLLBAR_HOVER_WIDTH;
    pub const DEFAULT: Self = Self(DEFAULT_SCROLLBAR_HOVER_WIDTH);

    pub fn new(pixels: u32) -> Self {
        Self(pixels.clamp(Self::MIN, Self::MAX))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl Default for ScrollbarHoverWidth {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl<'de> Deserialize<'de> for ScrollbarHoverWidth {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pixels = i64::deserialize(deserializer)?;
        Ok(Self::new(
            pixels.clamp(i64::from(Self::MIN), i64::from(Self::MAX)) as u32,
        ))
    }
}

#[cfg(test)]
mod tests;

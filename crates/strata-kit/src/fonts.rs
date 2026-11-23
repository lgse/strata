// SPDX-License-Identifier: MIT

use crate::look::Families;

#[cfg(test)]
mod tests;

pub const JETBRAINS_MONO: &[u8] = include_bytes!("../fonts/JetBrainsMono[wght].ttf");

/// The family name inside the face: what Rune matches in a family list or `Family::Named`.
pub const FAMILY: &str = "JetBrains Mono";

/// For `AppSpec::fonts`. They replace Rune's built-in Inter.
pub const FONTS: &[&[u8]] = &[JETBRAINS_MONO];

/// For `AppSpec::families`, as (sans, mono): Strata's default families, for both slots.
pub const FAMILIES: (&str, &str) = (FAMILY, FAMILY);

pub fn families() -> Families {
    Families::new(FAMILIES.0, FAMILIES.1)
}

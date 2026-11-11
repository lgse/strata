//! The bundled font: JetBrains Mono, Strata's interface font and its previews' monospace.
//!
//! Work package: K6, `docs/work/phase-1-leaves/K6-bundled-font.md`.
//!
//! The face and its families are data, as rune-kit's theme crates export theirs: a host passes
//! [`FONTS`] to `AppSpec::fonts` and [`FAMILIES`] to `AppSpec::families`, together. Fonts in
//! `AppSpec::fonts` replace Rune's built-in Inter, so [`FAMILIES`] without [`FONTS`] names a
//! face that isn't there, and text draws in whatever system face Rune falls back to.
//! [`LookParams::default`](crate::shape::LookParams) takes [`families`] as the app's own, so a
//! chosen interface font goes in front of it and Default puts it back (K2, `typography.rs`).
//!
//! The face is the variable `JetBrainsMono[wght].ttf` (100–800). Parley sets the `wght` axis
//! from `TextStyle::weight`, so Kit's weights need no mapping or variations.
//!
//! # Monospace surfaces
//!
//! Strata sets these in monospace whatever the interface font. A component draws them in the
//! mono slot (`kit.ty.mono`, `.mono()`, or rune-kit's mono table cells and Markdown code),
//! never a named or system family, so they stay in JetBrains Mono when the user picks another
//! interface font:
//!
//! - text and source previews and their line numbers (`.preview-text`,
//!   `.preview-source-lines`): C10b;
//! - document code spans and math source (`document-monospace`): C11;
//! - the preview command entry (`entry.preview-command-entry`);
//! - the density and thumbnail-size readouts in Appearance (`.appearance-density-value`,
//!   `.icons-thumbnail-value`): C20;
//! - diagnostics details and job logs (`.diagnostics-detail`, `.job-log`): C16.
//!
//! # Deviations from Strata
//!
//! - Strata writes the font to `$XDG_CACHE_HOME/strata/fonts/2.304/` and registers the file
//!   with fontconfig for GTK. Rune takes the bytes, so nothing is written and the system's
//!   fonts are untouched.
//! - Strata's family list is `"JetBrains Mono", "JetBrainsMono Nerd Font", monospace`. Both
//!   slots here are `JetBrains Mono` alone. Rune loads an installed family by name (since Rune
//!   `99c123d`), so the Nerd Font entry would resolve only where it is installed, and Rune's
//!   stand-in for the generic `monospace` (`ui-monospace`) would load the system's monospace
//!   files: either way drawn text would depend on the machine. The single bundled family keeps
//!   it the same everywhere. A glyph JetBrains Mono lacks falls back to Rune's symbol and CJK
//!   families, as before. A single name also keeps Rune treating the face as the slot's own
//!   font rather than as a script fallback.
//! - Where Strata names the generic `monospace` (document code, the command entry, diagnostics,
//!   job logs), the components use the mono slot, so those draw in JetBrains Mono, not the
//!   system's monospace face.

use crate::look::Families;

#[cfg(test)]
mod tests;

/// JetBrains Mono 2.304, the variable `wght` face (100–800), unmodified (SIL OFL 1.1; see
/// `fonts/README.md`).
pub const JETBRAINS_MONO: &[u8] = include_bytes!("../fonts/JetBrainsMono[wght].ttf");

/// The family name inside the face: what Rune matches in a family list or `Family::Named`.
pub const FAMILY: &str = "JetBrains Mono";

/// For `AppSpec::fonts`. They replace Rune's built-in Inter.
pub const FONTS: &[&[u8]] = &[JETBRAINS_MONO];

/// For `AppSpec::families`, as (sans, mono): Strata's default families, for both slots.
pub const FAMILIES: (&str, &str) = (FAMILY, FAMILY);

/// [`FAMILIES`] as [`Families`], the app's own families in
/// [`LookParams`](crate::shape::LookParams).
pub fn families() -> Families {
    Families::new(FAMILIES.0, FAMILIES.1)
}

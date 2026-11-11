//! Corner roundness, and [`LookParams`]: every K2 setting (roundness, density, text size and
//! the interface font), plus the element glow and animations switches, applied to a [`Look`]
//! in one step.
//!
//! Work package: K2, `docs/work/phase-1-leaves/K2-shape-density-type.md`.
//!
//! Values are Strata's, read at `lgse/strata` `upstream/main` 35ee677ab. Roundness:
//! `src/ui/preferences/corner_roundness.rs` (0–200 %, steps of 10, default 60) and
//! `src/ui/theme/roundness.rs` (every px radius scaled, rounded to 0.01 px).
//!
//! # Radii
//!
//! [`BASE_RADIUS`] is Strata's stylesheet at 100 %, by Kit step: 3 px for checks and thin
//! bars, 5 px for buttons, entries and breadcrumbs, 6 px for rows, cards and sidebar rows,
//! 9 px for dialogs, menus and popovers, 14 px for the settings panel. A radius between steps
//! (Strata also uses 4, 8, 10 and 12 px) goes through [`Look::round`] with its 100 % value.
//!
//! Round ends come in two kinds, as in Strata's stylesheet:
//!
//! - A **circle** (an avatar, a spinner, a radio dot: Strata's `50%` radii) stays a circle at
//!   every roundness: `radius(h / 2)`, never a Kit radius or [`Look::round`].
//! - A **pill** (a folder colour dot, a swatch, a badge, a chip, a slider trough: Strata's
//!   `999px` and `9999px` radii) follows the roundness and squares off at 0 %:
//!   `radius(look.round(999.0).min(h / 2.0))`.
//!
//! # Building order
//!
//! [`Look::apply_params`] is step 2 of the look's build (`look.rs`): after the theme, before
//! the user's overrides. It sets absolute values from fixed bases, so it owns the fields it
//! writes: `kit.radius`, `roundness`, `kit.ty`, `ty`, `icon_size`, `frame`, `content`, `row`,
//! `font`, `glow.on` and `animations`. It never reads what an earlier call left in them, so
//! applying the same parameters twice changes nothing, and new parameters replace old ones
//! completely. It leaves `space`, `tiles`, `thumbnails` and the glow's colour, radius and
//! opacity alone: Strata's paddings are fixed pixels, thumbnails don't follow the text size,
//! and the theme (K1) colours the glow.
//!
//! The two switches live here, not on the theme, so a host that rebuilds the look from a new
//! theme and the same parameters keeps them (W2c).

use rune_kit::Radius;

use crate::look::{Families, Look};
use crate::{density, fonts, typography};

#[cfg(test)]
mod tests;

/// A whole-number setting's range: its ends, the step its control moves by, and its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Steps {
    pub min: u16,
    pub max: u16,
    pub step: u16,
    pub default: u16,
}

impl Steps {
    /// `value` moved into the range.
    pub fn clamp(&self, value: u16) -> u16 {
        value.clamp(self.min, self.max)
    }

    /// `value` moved `delta` steps, clamped to the range.
    pub fn stepped(&self, value: u16, delta: i32) -> u16 {
        let moved = i64::from(self.clamp(value)) + i64::from(delta) * i64::from(self.step);
        moved.clamp(i64::from(self.min), i64::from(self.max)) as u16
    }
}

/// Corner roundness in percent of [`BASE_RADIUS`]: 0 squares every corner, 200 doubles it.
pub const ROUNDNESS: Steps = Steps {
    min: 0,
    max: 200,
    step: 10,
    default: 60,
};

/// Strata's radii at 100 % roundness, by Kit step.
pub const BASE_RADIUS: Radius = Radius {
    xs: 3.0,
    sm: 5.0,
    md: 6.0,
    lg: 9.0,
    xl: 14.0,
};

/// The factor a roundness of `percent` puts on every radius: 1.0 is 100 %.
pub fn factor(percent: u16) -> f32 {
    f32::from(ROUNDNESS.clamp(percent)) / 100.0
}

/// `base` at roundness `factor`, rounded to 0.01 pt as Strata rounds its scaled radii.
pub fn scale_radius(base: f32, factor: f32) -> f32 {
    (base * factor * 100.0).round() / 100.0
}

fn scale_radii(base: Radius, factor: f32) -> Radius {
    Radius {
        xs: scale_radius(base.xs, factor),
        sm: scale_radius(base.sm, factor),
        md: scale_radius(base.md, factor),
        lg: scale_radius(base.lg, factor),
        xl: scale_radius(base.xl, factor),
    }
}

/// The user's shape, density and type settings, from which [`Look::apply_params`] fills in
/// the look's tokens. Out-of-range values are clamped as Strata clamps a saved value.
///
/// Fields are public so Strata can build one from its settings:
/// `LookParams { text_size: 15, ..LookParams::default() }`.
#[derive(Clone, Debug, PartialEq)]
pub struct LookParams {
    /// Corner roundness in percent ([`ROUNDNESS`]).
    pub roundness: u16,
    /// Frame density in px ([`density::FRAME`]): the extra inset of the window's bars.
    pub frame_density: u8,
    /// Content density in px ([`density::CONTENT`]): the space above and below a row's
    /// content.
    pub content_density: u8,
    /// The interface text size in px ([`typography::TEXT_SIZE`]).
    pub text_size: u16,
    /// The desktop's text scaling factor (P5), which multiplies the text size once.
    pub text_scale: f32,
    /// The chosen interface family, or `None` for the app's own.
    pub font: Option<String>,
    /// The app's own families (`AppSpec::families`): the fallback behind a chosen family,
    /// and the mono slot, which a chosen family never changes. Defaults to the bundled
    /// JetBrains Mono for both ([`fonts::FAMILIES`]); the host registers [`fonts::FONTS`].
    pub app_families: Families,
    /// Strata's "Element glow" setting: `glow.on`.
    pub element_glow: bool,
    /// False when the user or the desktop turned animations off (P5's switch, as data):
    /// `animations`.
    pub animations: bool,
}

impl Default for LookParams {
    /// Strata's defaults, on the bundled JetBrains Mono ([`fonts::families`]).
    fn default() -> LookParams {
        LookParams {
            roundness: ROUNDNESS.default,
            frame_density: density::FRAME.compact,
            content_density: density::CONTENT.compact,
            text_size: typography::TEXT_SIZE.default,
            text_scale: 1.0,
            font: None,
            app_families: fonts::families(),
            element_glow: true,
            animations: true,
        }
    }
}

impl Look {
    /// Sets the tokens K2 owns from `params`: the type ramp and icon sizes for the text size,
    /// the bars and rows for the densities (and the text size), the radii for the roundness,
    /// the interface families, and the glow and animations switches. Applying the same
    /// parameters again changes nothing.
    pub fn apply_params(&mut self, params: &LookParams) {
        let root = typography::root_px(params.text_size, params.text_scale);
        typography::apply(self, root, params.font.as_deref(), &params.app_families);
        // Rows are sized from the scaled ramp and icons, so this follows the type step.
        density::apply(
            self,
            params.frame_density,
            params.content_density,
            typography::factor(root),
        );
        let factor = factor(params.roundness);
        self.roundness = factor;
        self.kit.radius = scale_radii(BASE_RADIUS, factor);
        self.glow.on = params.element_glow;
        self.animations = params.animations;
    }

    /// `self` with [`Look::apply_params`] applied.
    pub fn with_params(mut self, params: &LookParams) -> Look {
        self.apply_params(params);
        self
    }
}

// SPDX-License-Identifier: MIT

use rune_kit::Radius;

use crate::look::{Families, Look};
use crate::transparency::{self, TransparencyParams};
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

#[derive(Clone, Debug, PartialEq)]
pub struct LookParams {
    /// Corner roundness in percent ([`ROUNDNESS`]).
    pub roundness: u16,
    /// Frame density in px ([`density::FRAME`]): the extra inset of the window's bars.
    pub frame_density: u8,
    pub content_density: u8,
    /// The interface text size in px ([`typography::TEXT_SIZE`]).
    pub text_size: u16,
    pub text_scale: f32,
    /// The chosen interface family, or `None` for the app's own.
    pub font: Option<String>,
    pub app_families: Families,
    /// Strata's "Element glow" setting: `glow.on`.
    pub element_glow: bool,
    pub animations: bool,
    /// Surface transparency and frosted glass parameters.
    pub transparency: TransparencyParams,
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
            transparency: TransparencyParams::default(),
        }
    }
}

impl Look {
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
        transparency::apply(self, params.transparency);
    }

    /// `self` with [`Look::apply_params`] applied.
    pub fn with_params(mut self, params: &LookParams) -> Look {
        self.apply_params(params);
        self
    }
}

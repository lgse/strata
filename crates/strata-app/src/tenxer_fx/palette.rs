// SPDX-License-Identifier: MIT

//! Theme-derived colour palette and easing math for the 10x plasma splash FX.

use std::f32::consts::PI;

use rune::Color;
use strata_kit::look::Look;

/// Theme-derived colours for a single FX frame — zero hardcoded colours or theme names.
#[derive(Clone, Copy, Debug)]
pub(super) struct FxPalette {
    /// Primary saturated theme accent (`kit.c.bg_accent`).
    pub(super) accent: Color,
    /// High-clarity focus accent (`kit.c.border_focus`).
    pub(super) focus: Color,
    /// Luminous highlight core blended from `accent` toward its contrasting ink.
    pub(super) bright: Color,
    /// Theme base surface (`kit.c.bg_base`) for splash card grounding.
    pub(super) surface: Color,
    /// Alpha boost factor for crisp readability on light themes.
    pub(super) contrast_boost: f32,
}

impl FxPalette {
    pub(super) fn from_look(look: &Look) -> Self {
        let c = &look.kit.c;
        let accent = c.bg_accent;
        let focus = c.border_focus;
        let ink = look.ink_on(accent);
        let is_dark = c.bg_base.luminance() < 0.2;
        let bright = accent.mix(ink, if is_dark { 0.48 } else { 0.28 });
        Self {
            accent,
            focus,
            bright,
            surface: c.bg_base,
            contrast_boost: if is_dark { 1.0 } else { 1.18 },
        }
    }

    /// Returns `color` with alpha scaled for theme contrast and clamped to `0.0..=1.0`.
    pub(super) fn with_alpha(self, color: Color, alpha: f32) -> Color {
        color.alpha((alpha * self.contrast_boost).clamp(0.0, 1.0))
    }
}

/// Per-frame parameters for the 10x plasma splash animation.
#[derive(Clone, Copy)]
pub(super) struct FxFrame<'a> {
    pub(super) look: &'a Look,
    pub(super) width: f32,
    pub(super) height: f32,
    /// Normalized animation progress in `0.0..=1.0`.
    pub(super) t: f32,
    /// Monotonic elapsed playback time in seconds (for real-time GPU shader animation).
    pub(super) elapsed: f32,
    /// Per-trigger deterministic seed so repeated plays produce distinct arc choreography.
    pub(super) seed: u32,
    pub(super) palette: FxPalette,
}

impl<'a> FxFrame<'a> {
    pub(super) fn new(look: &'a Look, width: f32, height: f32, t: f32) -> Self {
        let t = clamp01(t);
        Self {
            look,
            width: width.max(1.0),
            height: height.max(1.0),
            t,
            elapsed: t,
            seed: 1,
            palette: FxPalette::from_look(look),
        }
    }

    pub(super) fn with_timing(mut self, elapsed: f32, seed: u32) -> Self {
        self.elapsed = elapsed.max(0.0);
        self.seed = seed.max(1);
        self
    }

    /// Window corner radius scaled by the active `CornerRoundness`.
    pub(super) fn window_radius(&self) -> f32 {
        self.look.round(10.0)
    }

    /// Accent color with alpha adjusted for theme luminance.
    pub(super) fn accent(&self, alpha: f32) -> Color {
        self.palette.with_alpha(self.palette.accent, alpha)
    }

    /// Focus accent color with alpha adjusted for theme luminance.
    pub(super) fn focus(&self, alpha: f32) -> Color {
        self.palette.with_alpha(self.palette.focus, alpha)
    }

    /// Bright luminous core color with alpha adjusted for theme luminance.
    pub(super) fn bright(&self, alpha: f32) -> Color {
        self.palette.with_alpha(self.palette.bright, alpha)
    }

    /// Encodes `[elapsed, intensity, opacity, signed_seed]` for `El::effect_data`.
    ///
    /// A negative seed tells `FxCompositor` that the active theme is light so additive shaders
    /// raise occlusion for crisp contrast on bright surfaces.
    pub(super) fn effect_slot_data(
        &self,
        intensity: f32,
        opacity: f32,
        seed_offset: u32,
    ) -> [f32; 4] {
        let is_light = self.look.kit.c.bg_base.luminance() >= 0.2;
        let seed = (self.seed.wrapping_add(seed_offset) % 65_535).max(1) as f32;
        let signed_seed = if is_light { -seed } else { seed };
        [self.elapsed, intensity, opacity, signed_seed]
    }
}

pub(super) fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Maps `t` linearly across `[start, end]` into `0.0..=1.0`.
pub(super) fn segment(t: f32, start: f32, end: f32) -> f32 {
    if end <= start {
        return if t >= end { 1.0 } else { 0.0 };
    }
    clamp01((t - start) / (end - start))
}

pub(super) fn ease_out_cubic(t: f32) -> f32 {
    let inv = 1.0 - clamp01(t);
    1.0 - inv * inv * inv
}

pub(super) fn ease_in_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
    }
}

/// Smooth underdamped spring curve (`0.0 -> ~1.09 -> 1.0`) for crisp overshoot lock-ins.
pub(super) fn spring_overshoot(t: f32) -> f32 {
    let t = clamp01(t);
    if t >= 1.0 {
        return 1.0;
    }
    let decay = (-6.5 * t).exp();
    1.0 - decay * (2.6 * PI * t).cos()
}

/// Smooth bell envelope (`0.0 -> 1.0 -> 0.0`) peaking at `peak` inside `[start, end]`.
pub(super) fn envelope(t: f32, start: f32, peak: f32, end: f32) -> f32 {
    if t <= start || t >= end {
        return 0.0;
    }
    if t < peak {
        ease_out_cubic(segment(t, start, peak))
    } else {
        1.0 - ease_in_out_cubic(segment(t, peak, end))
    }
}

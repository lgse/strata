// SPDX-License-Identifier: MIT

//! Variable surface transparency and frosted glass tokens for [`crate::look::Look`].
//!
//! When transparency is disabled (`TransparencyParams::enabled == false`), every surface color
//! remains untouched at its theme alpha (`1.0` for standard themes) and `backdrop_blur` is `0.0`.
//! When enabled, background surfaces (`bg_base`, `page`), raised chrome surfaces (`bg_deep`,
//! `surface_raised`, `surface_raised_base`, `surface_weak`, `layer_02`–`layer_04`), and floating
//! overlays (`layer_01`) are scaled independently while foreground text, icons, accents, borders,
//! and syntax colors stay at full opacity.

use std::hash::{Hash, Hasher};

use rune::{Color, El};
use rune_kit::Palette;

use crate::look::Look;

#[cfg(test)]
mod tests;

/// Gaussian blur sigma (in points) for frosted glass surfaces (`El::backdrop_blur`).
pub const FROSTED_BLUR_SIGMA: f32 = 14.0;

/// Minimum opacity kept on floating overlays (popovers, context menus, dialogs) when frosted
/// glass is active so overlapping text stays legible over translucent panes.
pub const OVERLAY_ALPHA_FLOOR_FROSTED: f32 = 0.72;

/// Minimum opacity kept on floating overlays when transparency is active without frosted blur.
pub const OVERLAY_ALPHA_FLOOR_PLAIN: f32 = 0.82;

/// User transparency settings and live window focus state handed to [`Look::apply_params`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransparencyParams {
    /// Master toggle. When `false`, all surface alphas stay at their base theme values (`1.0`).
    pub enabled: bool,
    /// Transparency of main background surfaces (`0..=100`; `0` is opaque, `100` is fully clear).
    pub background_percent: u8,
    /// Independent transparency of raised chrome surfaces (`0..=100`).
    pub raised_percent: u8,
    /// Applies GPU backdrop blur and a subtle frosted sheen to translucent overlays/chrome.
    pub frosted_glass: bool,
    /// Multiplier applied to transparency percentages when `!window_active` (`0.0..=2.0`).
    pub unfocused_multiplier: f32,
    /// Whether the window currently holds keyboard focus (`app.window.active`).
    pub window_active: bool,
}

impl Default for TransparencyParams {
    fn default() -> Self {
        Self {
            enabled: false,
            background_percent: 15,
            raised_percent: 10,
            frosted_glass: true,
            unfocused_multiplier: 1.25,
            window_active: true,
        }
    }
}

/// Resolved surface transparency and frosted glass state carried by [`Look`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Transparency {
    /// True when surface transparency effects are enabled.
    pub enabled: bool,
    /// Effective alpha (`0.0..=1.0`) applied to main background surfaces (`bg_base`, `page`).
    pub background_alpha: f32,
    /// Effective alpha (`0.0..=1.0`) applied to raised chrome surfaces (`bg_deep`,
    /// `surface_raised`, headers, sidebar, tab strip, footer).
    pub raised_alpha: f32,
    /// Effective alpha (`0.0..=1.0`) applied to floating overlays (popovers, menus, dialogs),
    /// clamped to a legibility floor.
    pub overlay_alpha: f32,
    /// True when both `enabled` and the frosted glass toggle are on.
    pub frosted_glass: bool,
    /// Blur sigma passed to `El::backdrop_blur` (`0.0` when `!frosted_glass`).
    pub backdrop_blur: f32,
    /// Subtle frosted rim highlight color (`Color::TRANSPARENT` when `!frosted_glass`).
    pub frost_border: Color,
}

impl Default for Transparency {
    fn default() -> Self {
        Self::disabled()
    }
}

impl Hash for Transparency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let Self {
            enabled,
            background_alpha,
            raised_alpha,
            overlay_alpha,
            frosted_glass,
            backdrop_blur,
            frost_border,
        } = self;
        enabled.hash(state);
        f32_bits(*background_alpha).hash(state);
        f32_bits(*raised_alpha).hash(state);
        f32_bits(*overlay_alpha).hash(state);
        frosted_glass.hash(state);
        f32_bits(*backdrop_blur).hash(state);
        frost_border.hash(state);
    }
}

fn f32_bits(x: f32) -> u32 {
    if x == 0.0 {
        0
    } else if x.is_nan() {
        f32::NAN.to_bits()
    } else {
        x.to_bits()
    }
}

impl Transparency {
    /// Completely opaque, frosted glass off.
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            background_alpha: 1.0,
            raised_alpha: 1.0,
            overlay_alpha: 1.0,
            frosted_glass: false,
            backdrop_blur: 0.0,
            frost_border: Color::TRANSPARENT,
        }
    }

    /// Resolves effective alphas and frosted glass parameters from `params` and theme polarity.
    pub fn resolve(params: TransparencyParams, dark: bool) -> Self {
        if !params.enabled {
            return Self::disabled();
        }
        let focus_factor = if params.window_active {
            1.0
        } else if params.unfocused_multiplier.is_finite() {
            params.unfocused_multiplier.clamp(0.0, 2.0)
        } else {
            1.0
        };
        let bg_transparency =
            (f32::from(params.background_percent.min(100)) / 100.0 * focus_factor).clamp(0.0, 1.0);
        let raised_transparency =
            (f32::from(params.raised_percent.min(100)) / 100.0 * focus_factor).clamp(0.0, 1.0);
        let background_alpha = (1.0 - bg_transparency).clamp(0.0, 1.0);
        let raised_alpha = (1.0 - raised_transparency).clamp(0.0, 1.0);
        let floor = if params.frosted_glass {
            OVERLAY_ALPHA_FLOOR_FROSTED
        } else {
            OVERLAY_ALPHA_FLOOR_PLAIN
        };
        let overlay_alpha = raised_alpha.max(floor).clamp(0.0, 1.0);
        let (backdrop_blur, frost_border) = if params.frosted_glass {
            let rim = Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if dark { 0.16 } else { 0.45 },
            };
            (FROSTED_BLUR_SIGMA, rim)
        } else {
            (0.0, Color::TRANSPARENT)
        };
        Self {
            enabled: true,
            background_alpha,
            raised_alpha,
            overlay_alpha,
            frosted_glass: params.frosted_glass,
            backdrop_blur,
            frost_border,
        }
    }

    /// Returns `color` with its alpha set to `overlay_alpha` when transparency is enabled,
    /// or unchanged when disabled.
    pub const fn overlay_color(self, color: Color) -> Color {
        if self.enabled {
            Color {
                a: self.overlay_alpha,
                ..color
            }
        } else {
            color
        }
    }

    /// Returns `color` at full opacity (`a = 1.0`) when transparency is enabled, or unchanged
    /// when disabled (for foreground spans, slider knobs, and media frames that derive from a
    /// surface token).
    pub const fn opaque(self, color: Color) -> Color {
        if self.enabled {
            Color { a: 1.0, ..color }
        } else {
            color
        }
    }

    /// Blends a subtle frosted sheen into `border` when frosted glass is active, or returns
    /// `border` unchanged otherwise.
    pub fn frosted_border(self, border: Color) -> Color {
        if !self.frosted_glass {
            return border;
        }
        let white = Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        let lift = border.mix(white, 0.18);
        Color {
            a: border.a.max(self.frost_border.a),
            ..lift
        }
    }
}

/// Wraps `body` in `rune_kit::popover::raised`, applying Strata's floating-overlay legibility
/// floor, frosted border sheen, and conditional GPU backdrop blur when transparency is enabled.
pub fn raised_popover(look: &Look, body: El, radius: f32) -> El {
    let kit = &look.kit;
    let mut surface = rune_kit::popover::raised(kit, body, radius);
    if look.transparency.enabled {
        surface = surface
            .fill(look.transparency.overlay_color(kit.c.surface_raised))
            .border(
                1.0,
                look.transparency
                    .frosted_border(kit.alpha(kit.c.std_border, 0.5)),
            );
    }
    if look.transparency.frosted_glass {
        surface = surface.backdrop_blur(look.transparency.backdrop_blur);
    }
    surface
}

/// Snapshot of the theme's original surface colors captured in [`Look::from_kit`], allowing
/// [`Look::apply_params`] to stay idempotent across repeated parameter updates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BaseSurfaces {
    page: Color,
    bg_base: Color,
    bg_deep: Color,
    surface_raised: Color,
    surface_raised_base: Color,
    surface_weak: Color,
    layer_01: Color,
    layer_02: Color,
    layer_03: Color,
    layer_04: Color,
}

impl BaseSurfaces {
    pub(crate) fn from_palette(p: &Palette) -> Self {
        Self {
            page: p.page,
            bg_base: p.bg_base,
            bg_deep: p.bg_deep,
            surface_raised: p.surface_raised,
            surface_raised_base: p.surface_raised_base,
            surface_weak: p.surface_weak,
            layer_01: p.layer_01,
            layer_02: p.layer_02,
            layer_03: p.layer_03,
            layer_04: p.layer_04,
        }
    }

    fn restore(&self, p: &mut Palette) {
        p.page = self.page;
        p.bg_base = self.bg_base;
        p.bg_deep = self.bg_deep;
        p.surface_raised = self.surface_raised;
        p.surface_raised_base = self.surface_raised_base;
        p.surface_weak = self.surface_weak;
        p.layer_01 = self.layer_01;
        p.layer_02 = self.layer_02;
        p.layer_03 = self.layer_03;
        p.layer_04 = self.layer_04;
    }
}

fn scale_alpha(color: Color, factor: f32) -> Color {
    Color {
        a: (color.a * factor).clamp(0.0, 1.0),
        ..color
    }
}

fn frosted_tint(color: Color, dark: bool) -> Color {
    let mix = if dark { 0.04 } else { 0.06 };
    let white = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: color.a,
    };
    let lifted = color.mix(white, mix);
    Color {
        a: color.a,
        ..lifted
    }
}

/// Applies `params` to `look` idempotently from `look.base_surfaces`.
pub(crate) fn apply(look: &mut Look, params: TransparencyParams) {
    look.base_surfaces.restore(&mut look.kit.c);
    let resolved = Transparency::resolve(params, look.kit.dark);
    look.transparency = resolved;
    if !resolved.enabled {
        return;
    }

    let dark = look.kit.dark;
    let p = &mut look.kit.c;
    p.page = scale_alpha(p.page, resolved.background_alpha);
    p.bg_base = scale_alpha(p.bg_base, resolved.background_alpha);

    let paint_raised = |c: Color, alpha: f32| {
        let base = if resolved.frosted_glass {
            frosted_tint(c, dark)
        } else {
            c
        };
        scale_alpha(base, alpha)
    };

    p.bg_deep = paint_raised(p.bg_deep, resolved.raised_alpha);
    p.surface_raised = paint_raised(p.surface_raised, resolved.raised_alpha);
    p.surface_raised_base = paint_raised(p.surface_raised_base, resolved.raised_alpha);
    p.surface_weak = paint_raised(p.surface_weak, resolved.raised_alpha);
    p.layer_02 = paint_raised(p.layer_02, resolved.raised_alpha);
    p.layer_03 = paint_raised(p.layer_03, resolved.raised_alpha);
    p.layer_04 = paint_raised(p.layer_04, resolved.raised_alpha);

    p.layer_01 = paint_raised(p.layer_01, resolved.overlay_alpha);
}

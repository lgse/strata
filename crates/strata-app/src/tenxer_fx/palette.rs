// SPDX-License-Identifier: MIT

//! Shared theme-derived colour palette, easing math, and vector geometry helpers for 10x FX.

use std::f32::consts::PI;

use rune::shape::Path;
use rune::{Color, El, Shadow, TextStyle, el, row, text};
use strata_kit::look::Look;

/// Theme-derived colours for a single FX frame — zero hardcoded colours or theme names.
#[derive(Clone, Copy, Debug)]
pub(super) struct FxPalette {
    /// Primary saturated theme accent (`kit.c.bg_accent`).
    pub(super) accent: Color,
    /// High-clarity focus accent (`kit.c.border_focus`).
    pub(super) focus: Color,
    /// Accent text ink (`kit.c.text_accent`).
    pub(super) text_accent: Color,
    /// Luminous highlight core blended from `accent` toward its contrasting ink.
    pub(super) bright: Color,
    /// Contrasting foreground ink over `accent`.
    pub(super) ink: Color,
    /// Theme base surface (`kit.c.bg_base`) for subtle emblem grounding.
    pub(super) surface: Color,
    /// Alpha boost factor for crisp readability on light themes.
    pub(super) contrast_boost: f32,
}

impl FxPalette {
    pub(super) fn from_look(look: &Look) -> Self {
        let c = &look.kit.c;
        let accent = c.bg_accent;
        let focus = c.border_focus;
        let text_accent = c.text_accent;
        let ink = look.ink_on(accent);
        let is_dark = c.bg_base.luminance() < 0.2;
        let bright = accent.mix(ink, if is_dark { 0.48 } else { 0.28 });
        Self {
            accent,
            focus,
            text_accent,
            bright,
            ink,
            surface: c.bg_base,
            contrast_boost: if is_dark { 1.0 } else { 1.18 },
        }
    }

    /// Returns `color` with alpha scaled for theme contrast and clamped to `0.0..=1.0`.
    pub(super) fn with_alpha(self, color: Color, alpha: f32) -> Color {
        color.alpha((alpha * self.contrast_boost).clamp(0.0, 1.0))
    }
}

/// Per-frame parameters shared across all 10x visual FX concepts.
#[derive(Clone, Copy)]
pub(super) struct FxFrame<'a> {
    pub(super) look: &'a Look,
    pub(super) width: f32,
    pub(super) height: f32,
    /// Normalized animation progress in `0.0..=1.0`.
    pub(super) t: f32,
    pub(super) palette: FxPalette,
}

impl<'a> FxFrame<'a> {
    pub(super) fn new(look: &'a Look, width: f32, height: f32, t: f32) -> Self {
        Self {
            look,
            width: width.max(1.0),
            height: height.max(1.0),
            t: clamp01(t),
            palette: FxPalette::from_look(look),
        }
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

pub(super) fn ease_out_expo(t: f32) -> f32 {
    let t = clamp01(t);
    if t >= 1.0 {
        1.0
    } else {
        1.0 - 2.0_f32.powf(-8.0 * t)
    }
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

/// Inset accent glow frame pulse around the entire window perimeter.
pub(super) fn frame_border_pulse(frame: &FxFrame<'_>, strength: f32) -> Option<El> {
    if strength <= 0.004 {
        return None;
    }
    let radius = frame.window_radius();
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(radius)
            .border(2.0, frame.focus(0.75 * strength))
            .box_shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 24.0,
                spread: 2.0,
                color: frame.accent(0.45 * strength),
                inset: true,
            })
            .box_shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 6.0,
                spread: 0.0,
                color: frame.bright(0.32 * strength),
                inset: true,
            }),
    )
}

/// Builds a glowing centered `10X` badge with analytic scale, opacity, and Gaussian blur.
pub(super) struct EmblemSpec {
    pub(super) opacity: f32,
    pub(super) scale: f32,
    pub(super) blur: f32,
    pub(super) glow_alpha: f32,
    pub(super) accent_fill: bool,
}

pub(super) fn centered_emblem(frame: &FxFrame<'_>, spec: EmblemSpec) -> Option<El> {
    if spec.opacity <= 0.005 {
        return None;
    }
    let look = frame.look;
    let p = frame.palette;
    let radius = look.round(12.0);
    let font = TextStyle {
        size: 30.0,
        weight: 800.0,
        tracking: 3.0,
        ..look.kit.ty.ui_medium
    };
    let (bg, fg) = if spec.accent_fill {
        (p.accent.alpha(0.92), p.ink)
    } else {
        (p.surface.alpha(0.84), p.text_accent)
    };
    let badge = row()
        .px(24.0)
        .py(10.0)
        .center()
        .radius(radius)
        .fill(bg)
        .border(1.5, frame.focus(0.85))
        .box_shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 32.0,
            spread: 2.0,
            color: frame.accent(spec.glow_alpha),
            inset: false,
        })
        .box_shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 10.0,
            spread: 0.0,
            color: frame.bright(spec.glow_alpha * 0.55),
            inset: true,
        })
        .child(text("10X").font(font).color(fg).a11y_hidden())
        .scale(spec.scale)
        .blur(spec.blur)
        .layer_opacity(spec.opacity);
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .center()
            .child(badge),
    )
}

/// Builds the two equal-length halves of a rounded window perimeter starting at the bottom-right
/// `10X` badge region (`[w - inset, h - inset]`) and colliding at top-left (`[inset, inset]`).
pub(super) fn perimeter_halves(frame: &FxFrame<'_>, inset: f32) -> (Path, Path) {
    let x0 = inset;
    let y0 = inset;
    let x1 = (frame.width - inset).max(x0 + 4.0);
    let y1 = (frame.height - inset).max(y0 + 4.0);
    let r = frame
        .window_radius()
        .clamp(0.0, ((x1 - x0).min(y1 - y0)) * 0.45);

    let mut lower_left = Path::new().move_to([x1 - r, y1]).line_to([x0 + r, y1]);
    if r > 0.5 {
        lower_left = lower_left.quad_to([x0, y1], [x0, y1 - r]);
    }
    lower_left = lower_left.line_to([x0, y0 + r]);
    if r > 0.5 {
        lower_left = lower_left.quad_to([x0, y0], [x0 + r, y0]);
    }

    let mut upper_right = Path::new().move_to([x1 - r, y1]);
    if r > 0.5 {
        upper_right = upper_right.quad_to([x1, y1], [x1, y1 - r]);
    }
    upper_right = upper_right.line_to([x1, y0 + r]);
    if r > 0.5 {
        upper_right = upper_right.quad_to([x1, y0], [x1 - r, y0]);
    }
    upper_right = upper_right.line_to([x0 + r, y0]);

    (lower_left, upper_right)
}

/// Approximates the 2D point at normalized arc-length `u` (`0.0..=1.0`) along a corner-to-corner
/// half-perimeter path (`w` edge + `h` edge) for placing a glowing spark head flare.
pub(super) fn perimeter_head_point(
    frame: &FxFrame<'_>,
    inset: f32,
    u: f32,
    upper_right: bool,
) -> [f32; 2] {
    let u = clamp01(u);
    let span_x = (frame.width - 2.0 * inset).max(1.0);
    let span_y = (frame.height - 2.0 * inset).max(1.0);
    let dist = u * (span_x + span_y);
    if upper_right {
        if dist <= span_y {
            [frame.width - inset, (frame.height - inset) - dist]
        } else {
            [(frame.width - inset) - (dist - span_y), inset]
        }
    } else if dist <= span_x {
        [(frame.width - inset) - dist, frame.height - inset]
    } else {
        [inset, (frame.height - inset) - (dist - span_x)]
    }
}

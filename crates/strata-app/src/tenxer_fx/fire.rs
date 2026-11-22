// SPDX-License-Identifier: MIT

//! Concept 6: **Interface on fire** — real GPU fluid-simulated blackbody fire, rising embers,
//! curling smoke, shimmering backdrop heat haze, a blazing perimeter filament border, and a
//! flaming `10X` lock-in emblem rendered through `rune-fx` effect slots.

use std::f32::consts::PI;

use rune::{Color, El, Shadow, TextStyle, effect, el, row, text};

use crate::fx::{
    MARK_FIRE_BADGE_FLAME, MARK_FIRE_BADGE_HAZE, MARK_FIRE_BORDER, MARK_FIRE_BOTTOM, MARK_FIRE_HAZE,
};

use super::palette::{
    FxFrame, clamp01, ease_in_out_cubic, ease_out_cubic, segment, spring_overshoot,
};

/// Theme-derived thermal tones for the subtle ambient wash and `10X` emblem badge.
#[derive(Clone, Copy)]
struct FireAccent {
    crimson: Color,
    flame: Color,
    amber: Color,
    emblem_bg: Color,
    emblem_ink: Color,
    boost: f32,
}

impl FireAccent {
    fn from_frame(frame: &FxFrame<'_>) -> Self {
        let c = &frame.look.kit.c;
        let is_dark = c.bg_base.luminance() < 0.2;
        let danger = c.danger.fg;
        let warning = c.warning.fg;
        let crimson = danger.mix(c.bg_accent, 0.08);
        let flame = danger.mix(warning, 0.46);
        let amber = warning.mix(c.white, if is_dark { 0.24 } else { 0.10 });
        let emblem_bg = danger.mix(warning, 0.28);
        let emblem_ink = frame.look.ink_on(emblem_bg);
        Self {
            crimson,
            flame,
            amber,
            emblem_bg,
            emblem_ink,
            boost: frame.palette.contrast_boost,
        }
    }

    fn with_alpha(self, color: Color, alpha: f32) -> Color {
        color.alpha((alpha * self.boost).clamp(0.0, 1.0))
    }
}

/// Ignition (`0.0..0.16`), full roaring blaze (`0.16..0.68`), and smooth burn-down (`0.68..1.0`).
#[derive(Clone, Copy)]
struct FireLifecycle {
    blaze_opacity: f32,
    blaze_intensity: f32,
    haze_intensity: f32,
    border_opacity: f32,
}

impl FireLifecycle {
    fn at(t: f32) -> Self {
        let ignite = ease_out_cubic(segment(t, 0.0, 0.16));
        let burn_down = 1.0 - ease_in_out_cubic(segment(t, 0.68, 1.0));
        let blaze_opacity = clamp01(ignite * burn_down);
        let surge = 0.85 + 0.25 * ease_out_cubic(segment(t, 0.0, 0.24));
        let cool = 1.0 - 0.35 * ease_in_out_cubic(segment(t, 0.66, 1.0));
        let blaze_intensity = clamp01(ignite * burn_down) * surge * cool;

        let haze_in = ease_out_cubic(segment(t, 0.04, 0.22));
        let haze_out = 1.0 - ease_in_out_cubic(segment(t, 0.66, 0.98));
        let haze_intensity = clamp01(haze_in * haze_out);

        let border_in = ease_out_cubic(segment(t, 0.02, 0.20));
        let border_out = 1.0 - ease_in_out_cubic(segment(t, 0.64, 0.96));
        let border_opacity = clamp01(border_in * border_out);

        Self {
            blaze_opacity,
            blaze_intensity,
            haze_intensity,
            border_opacity,
        }
    }
}

pub(super) fn build(frame: &FxFrame<'_>) -> El {
    let accent = FireAccent::from_frame(frame);
    let life = FireLifecycle::at(frame.t);

    let mut root = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if let Some(wash) = heat_wash(frame, accent, life) {
        root = root.child(wash);
    }
    if let Some(haze) = window_heat_haze(frame, life) {
        root = root.child(haze);
    }
    if let Some(border) = perimeter_fire_border(frame, life) {
        root = root.child(border);
    }
    if let Some(blaze) = bottom_blaze(frame, life) {
        root = root.child(blaze);
    }
    if let Some(emblem) = fire_emblem(frame, accent) {
        root = root.child(emblem);
    }
    root
}

fn slot_data(frame: &FxFrame<'_>, intensity: f32, opacity: f32) -> [f32; 4] {
    let is_light = frame.look.kit.c.bg_base.luminance() >= 0.2;
    let signed_seed = if is_light {
        -(frame.seed as f32)
    } else {
        frame.seed as f32
    };
    [frame.elapsed, intensity, opacity, signed_seed]
}

fn heat_wash(frame: &FxFrame<'_>, accent: FireAccent, life: FireLifecycle) -> Option<El> {
    if life.blaze_opacity <= 0.005 {
        return None;
    }
    let linear_stops = [
        (0.0, accent.with_alpha(accent.crimson, 0.0)),
        (
            0.55,
            accent.with_alpha(accent.crimson, 0.04 * life.blaze_opacity),
        ),
        (
            0.84,
            accent.with_alpha(accent.flame, 0.14 * life.blaze_opacity),
        ),
        (
            1.0,
            accent.with_alpha(accent.amber, 0.24 * life.blaze_opacity),
        ),
    ];
    let linear = el()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .linear_gradient(PI, linear_stops);

    let rx = frame.width * 0.62;
    let ry = frame.height * 0.42;
    let radial_stops = [
        (
            0.0,
            accent.with_alpha(accent.amber, 0.22 * life.blaze_opacity),
        ),
        (
            0.45,
            accent.with_alpha(accent.flame, 0.12 * life.blaze_opacity),
        ),
        (
            0.82,
            accent.with_alpha(accent.crimson, 0.04 * life.blaze_opacity),
        ),
        (1.0, accent.with_alpha(accent.crimson, 0.0)),
    ];
    let radial = el()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .radial_gradient([0.5, 1.0], [rx, ry], radial_stops);

    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(linear)
            .child(radial),
    )
}

fn window_heat_haze(frame: &FxFrame<'_>, life: FireLifecycle) -> Option<El> {
    if life.haze_intensity <= 0.005 {
        return None;
    }
    Some(
        effect(MARK_FIRE_HAZE)
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .effect_backdrop(true)
            .effect_data(slot_data(frame, life.haze_intensity, life.haze_intensity)),
    )
}

fn perimeter_fire_border(frame: &FxFrame<'_>, life: FireLifecycle) -> Option<El> {
    if life.border_opacity <= 0.005 {
        return None;
    }
    Some(
        effect(MARK_FIRE_BORDER)
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(frame.window_radius())
            .effect_data(slot_data(frame, life.border_opacity, life.border_opacity)),
    )
}

fn bottom_blaze(frame: &FxFrame<'_>, life: FireLifecycle) -> Option<El> {
    if life.blaze_opacity <= 0.005 {
        return None;
    }
    Some(
        effect(MARK_FIRE_BOTTOM)
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .effect_data(slot_data(frame, life.blaze_intensity, life.blaze_opacity)),
    )
}

fn fire_emblem(frame: &FxFrame<'_>, accent: FireAccent) -> Option<El> {
    let enter_t = segment(frame.t, 0.06, 0.24);
    let exit_t = segment(frame.t, 0.62, 0.90);
    let opacity = ease_out_cubic(enter_t) * (1.0 - ease_in_out_cubic(exit_t));
    if opacity <= 0.005 {
        return None;
    }

    let look = frame.look;
    let radius = look.round(14.0);
    let scale = (0.76 + 0.24 * spring_overshoot(enter_t)) + 0.08 * ease_in_out_cubic(exit_t);
    let blur_in = 7.0 * (1.0 - ease_out_cubic(enter_t));
    let blur_out = 10.0 * ease_in_out_cubic(exit_t);
    let font = TextStyle {
        size: 30.0,
        weight: 800.0,
        tracking: 3.0,
        ..look.kit.ty.ui_medium
    };

    let flame_opacity =
        ease_out_cubic(segment(frame.t, 0.08, 0.26)) * (1.0 - ease_in_out_cubic(exit_t));

    let badge = row()
        .px(26.0)
        .py(11.0)
        .center()
        .radius(radius)
        .fill(accent.emblem_bg.alpha(0.94))
        .border(1.75, accent.with_alpha(accent.amber, 0.92))
        .box_shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 32.0,
            spread: 2.0,
            color: accent.with_alpha(accent.crimson, 0.68 * opacity),
            inset: false,
        })
        .box_shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 12.0,
            spread: 1.0,
            color: accent.with_alpha(accent.amber, 0.48 * opacity),
            inset: true,
        })
        .child(
            text("10X")
                .font(font)
                .color(accent.emblem_ink)
                .a11y_hidden(),
        )
        .scale(scale)
        .blur(blur_in + blur_out)
        .layer_opacity(opacity);

    let mut holder = el();
    if flame_opacity > 0.005 {
        holder = holder.child(
            effect(MARK_FIRE_BADGE_FLAME)
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(radius)
                .effect_bleed(96.0)
                .effect_data(slot_data(frame, flame_opacity, flame_opacity)),
        );
    }
    holder = holder.child(badge);
    if flame_opacity > 0.005 {
        holder = holder.child(
            effect(MARK_FIRE_BADGE_HAZE)
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(radius)
                .effect_bleed(56.0)
                .effect_backdrop(true)
                .effect_data(slot_data(frame, flame_opacity, flame_opacity)),
        );
    }

    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .center()
            .child(holder),
    )
}

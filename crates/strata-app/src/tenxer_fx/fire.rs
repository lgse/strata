// SPDX-License-Identifier: MIT

//! Concept 6: **Interface on fire** — theme-derived thermal flame walls roaring along the bottom
//! and lower side edges, inset perimeter heat-bloom glow, rising drifting ember sparks, and a
//! blazing `10X` lock-in emblem.

use std::f32::consts::{PI, TAU};

use rune::{
    Color, El, Shadow, TextStyle, canvas, el, row,
    shape::{LineCap, Shape},
    text,
};

use super::palette::{
    FxFrame, clamp01, ease_in_out_cubic, ease_out_cubic, envelope, segment, spring_overshoot,
};

const BOTTOM_SAMPLES: usize = 56;
const SIDE_SAMPLES: usize = 24;
const EMBER_COUNT: usize = 32;

/// Theme-derived thermal spectrum — zero hardcoded colours or theme names.
#[derive(Clone, Copy)]
struct FirePalette {
    crimson: Color,
    flame: Color,
    amber: Color,
    core: Color,
    emblem_bg: Color,
    emblem_ink: Color,
    boost: f32,
}

impl FirePalette {
    fn from_frame(frame: &FxFrame<'_>) -> Self {
        let c = &frame.look.kit.c;
        let is_dark = c.bg_base.luminance() < 0.2;
        let danger = c.danger.fg;
        let warning = c.warning.fg;
        let crimson = danger.mix(c.bg_accent, 0.08);
        let flame = danger.mix(warning, 0.46);
        let amber = warning.mix(c.white, if is_dark { 0.24 } else { 0.10 });
        let core = warning.mix(c.white, if is_dark { 0.72 } else { 0.52 });
        let emblem_bg = danger.mix(c.bg_base, 0.16);
        let emblem_ink = frame.look.ink_on(emblem_bg);
        Self {
            crimson,
            flame,
            amber,
            core,
            emblem_bg,
            emblem_ink,
            boost: frame.palette.contrast_boost,
        }
    }

    fn with_alpha(self, color: Color, alpha: f32) -> Color {
        color.alpha((alpha * self.boost).clamp(0.0, 1.0))
    }
}

/// Lifecycle intensity curves shared across flame walls, heat wash, and perimeter glow.
#[derive(Clone, Copy)]
struct FireLifecycle {
    /// Overall surge height (`0 -> 1` in `0.0..0.22`, roaring in `0.22..0.68`, `1 -> 0` in `0.68..1.0`).
    surge: f32,
    /// Alpha intensity of the flame body.
    alpha: f32,
    /// Subtle roaring flicker multiplier around `1.0`.
    flicker: f32,
}

impl FireLifecycle {
    fn at(t: f32) -> Self {
        let ignite = ease_out_cubic(segment(t, 0.0, 0.22));
        let burn_down = 1.0 - ease_in_out_cubic(segment(t, 0.68, 1.0));
        let surge = clamp01(ignite * burn_down);
        let flicker = 1.0 + 0.065 * (t * 26.0).sin() + 0.035 * (t * 43.0 + 1.1).sin();
        let alpha = clamp01(envelope(t, 0.0, 0.18, 1.0));
        Self {
            surge,
            alpha,
            flicker,
        }
    }
}

struct FlameLayerSpec {
    max_height_frac: f32,
    side_width_frac: f32,
    side_reach_frac: f32,
    phase_speed: f32,
    seed: f32,
    sharpness: f32,
    color: Color,
}

pub(super) fn build(frame: &FxFrame<'_>) -> El {
    let fire = FirePalette::from_frame(frame);
    let life = FireLifecycle::at(frame.t);

    let mut root = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if let Some(wash) = heat_wash(frame, fire, life) {
        root = root.child(wash);
    }
    if let Some(rim) = perimeter_heat_glow(frame, fire, life) {
        root = root.child(rim);
    }
    if let Some(outer) = outer_flame_layer(frame, fire, life) {
        root = root.child(outer);
    }
    if let Some(mid) = mid_flame_layer(frame, fire, life) {
        root = root.child(mid);
    }
    if let Some(core) = core_flame_layer(frame, fire, life) {
        root = root.child(core);
    }
    if let Some(embers) = ember_sparks(frame, fire) {
        root = root.child(embers);
    }
    if let Some(badge) = fire_emblem(frame, fire) {
        root = root.child(badge);
    }
    root
}

fn heat_wash(frame: &FxFrame<'_>, fire: FirePalette, life: FireLifecycle) -> Option<El> {
    if life.alpha <= 0.005 {
        return None;
    }
    let linear_stops = [
        (0.0, fire.with_alpha(fire.crimson, 0.0)),
        (0.46, fire.with_alpha(fire.crimson, 0.05 * life.alpha)),
        (0.78, fire.with_alpha(fire.flame, 0.18 * life.alpha)),
        (1.0, fire.with_alpha(fire.amber, 0.34 * life.alpha)),
    ];
    let linear = el()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .linear_gradient(PI, linear_stops);

    let rx = frame.width * (0.48 + 0.22 * life.surge);
    let ry = frame.height * (0.34 + 0.24 * life.surge);
    let radial_stops = [
        (0.0, fire.with_alpha(fire.amber, 0.36 * life.alpha)),
        (0.42, fire.with_alpha(fire.flame, 0.20 * life.alpha)),
        (0.78, fire.with_alpha(fire.crimson, 0.08 * life.alpha)),
        (1.0, fire.with_alpha(fire.crimson, 0.0)),
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

fn perimeter_heat_glow(frame: &FxFrame<'_>, fire: FirePalette, life: FireLifecycle) -> Option<El> {
    let strength = life.alpha * (0.85 + 0.15 * life.flicker);
    if strength <= 0.005 {
        return None;
    }
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(frame.window_radius())
            .border(2.0, fire.with_alpha(fire.amber, 0.68 * strength))
            .box_shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 36.0,
                spread: 4.0,
                color: fire.with_alpha(fire.crimson, 0.56 * strength),
                inset: true,
            })
            .box_shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 14.0,
                spread: 1.0,
                color: fire.with_alpha(fire.flame, 0.48 * strength),
                inset: true,
            })
            .box_shadow(Shadow {
                x: 0.0,
                y: -10.0,
                blur: 28.0,
                spread: 2.0,
                color: fire.with_alpha(fire.amber, 0.32 * strength),
                inset: true,
            }),
    )
}

fn outer_flame_layer(frame: &FxFrame<'_>, fire: FirePalette, life: FireLifecycle) -> Option<El> {
    if life.surge <= 0.005 {
        return None;
    }
    let spec = FlameLayerSpec {
        max_height_frac: 0.34,
        side_width_frac: 0.11,
        side_reach_frac: 0.72,
        phase_speed: 6.2,
        seed: 0.4,
        sharpness: 1.25,
        color: fire.with_alpha(fire.crimson, 0.68 * life.alpha),
    };
    let shape = build_flame_wall_shape(frame, life, &spec, true);
    Some(full_canvas(shape).blur(14.0))
}

fn mid_flame_layer(frame: &FxFrame<'_>, fire: FirePalette, life: FireLifecycle) -> Option<El> {
    if life.surge <= 0.005 {
        return None;
    }
    let flame_spec = FlameLayerSpec {
        max_height_frac: 0.25,
        side_width_frac: 0.075,
        side_reach_frac: 0.56,
        phase_speed: 8.6,
        seed: 2.1,
        sharpness: 1.45,
        color: fire.with_alpha(fire.flame, 0.78 * life.alpha),
    };
    let amber_spec = FlameLayerSpec {
        max_height_frac: 0.18,
        side_width_frac: 0.048,
        side_reach_frac: 0.40,
        phase_speed: 10.4,
        seed: 4.3,
        sharpness: 1.65,
        color: fire.with_alpha(fire.amber, 0.64 * life.alpha),
    };
    let mut shape = build_flame_wall_shape(frame, life, &flame_spec, true);
    let inner = build_flame_wall_shape(frame, life, &amber_spec, true);
    shape.ops.extend(inner.ops);
    Some(full_canvas(shape).blur(5.0))
}

fn core_flame_layer(frame: &FxFrame<'_>, fire: FirePalette, life: FireLifecycle) -> Option<El> {
    let core_surge = life.surge * envelope(frame.t, 0.02, 0.24, 0.90);
    if core_surge <= 0.005 {
        return None;
    }
    let core_life = FireLifecycle {
        surge: core_surge,
        alpha: life.alpha,
        flicker: life.flicker,
    };
    let spec = FlameLayerSpec {
        max_height_frac: 0.105,
        side_width_frac: 0.028,
        side_reach_frac: 0.24,
        phase_speed: 12.8,
        seed: 5.7,
        sharpness: 1.8,
        color: fire.with_alpha(fire.core, 0.88 * core_surge),
    };
    let shape = build_flame_wall_shape(frame, core_life, &spec, false);
    Some(full_canvas(shape).blur(1.5))
}

fn build_flame_wall_shape(
    frame: &FxFrame<'_>,
    life: FireLifecycle,
    spec: &FlameLayerSpec,
    include_sides: bool,
) -> Shape {
    let mut shape = Shape::new().view_box(frame.width, frame.height);
    let bottom = bottom_flame_polygon(frame, life, spec);
    shape = shape.fill(bottom, spec.color);
    if include_sides {
        shape = shape
            .fill(side_flame_polygon(frame, life, spec, true), spec.color)
            .fill(side_flame_polygon(frame, life, spec, false), spec.color);
    }
    shape
}

fn bottom_flame_polygon(
    frame: &FxFrame<'_>,
    life: FireLifecycle,
    spec: &FlameLayerSpec,
) -> Vec<[f32; 2]> {
    let phase = frame.t * spec.phase_speed;
    let base_amp = frame.height * spec.max_height_frac * life.surge * life.flicker;
    let max_h = (frame.height * 0.82).max(1.0);
    let mut pts = Vec::with_capacity(BOTTOM_SAMPLES + 3);
    for i in 0..=BOTTOM_SAMPLES {
        let u = i as f32 / BOTTOM_SAMPLES as f32;
        let x = u * frame.width;
        // Cup upward along left & right edges while leaving center clearer for content & emblem.
        let centered = 2.0 * u - 1.0;
        let edge_cup = 0.72 + 0.58 * centered * centered;
        let tongue = flame_wave(u, phase, spec.seed, spec.sharpness);
        let h = (base_amp * edge_cup * tongue).clamp(1.0, max_h);
        pts.push([x, (frame.height - h).max(0.0)]);
    }
    pts.push([frame.width, frame.height]);
    pts.push([0.0, frame.height]);
    pts
}

fn side_flame_polygon(
    frame: &FxFrame<'_>,
    life: FireLifecycle,
    spec: &FlameLayerSpec,
    left_edge: bool,
) -> Vec<[f32; 2]> {
    if frame.width < 4.0 || frame.height < 4.0 {
        return Vec::new();
    }
    let phase = frame.t * (spec.phase_speed * 1.15);
    let seed = spec.seed + if left_edge { 1.7 } else { 3.9 };
    let max_reach = (frame.height * 0.92).max(2.0);
    let reach = (frame.height * spec.side_reach_frac * life.surge).clamp(2.0, max_reach);
    let max_depth = frame.width * spec.side_width_frac * life.surge * life.flicker;
    let max_cap = (frame.width * 0.36).max(1.0);
    let top_y = (frame.height - reach).max(0.0);
    let edge_x = if left_edge { 0.0 } else { frame.width };
    let inward = if left_edge { 1.0 } else { -1.0 };

    let mut pts = Vec::with_capacity(SIDE_SAMPLES + 2);
    pts.push([edge_x, frame.height]);
    for i in 0..SIDE_SAMPLES {
        let v = i as f32 / SIDE_SAMPLES as f32;
        let y = frame.height - v * reach;
        // Taper cleanly toward the edge as `v` approaches `1.0`.
        let taper = (1.0 - v).powf(1.35);
        let wave = flame_wave(v * 0.85, phase, seed, spec.sharpness);
        let depth = (max_depth * taper * wave).clamp(0.5, max_cap);
        pts.push([edge_x + inward * depth, y]);
    }
    pts.push([edge_x, top_y]);
    pts
}

/// Multi-harmonic licking flame wave in `~[0.24, 1.18]` with sharp upward tongues.
fn flame_wave(u: f32, phase: f32, seed: f32, sharpness: f32) -> f32 {
    let billow = 0.5 + 0.5 * (u * TAU * 3.0 + phase * 0.85 + seed).sin();
    let tongue = (u * TAU * 7.0 - phase * 1.55 + seed * 1.9)
        .sin()
        .abs()
        .powf(sharpness);
    let flicker = 0.5 + 0.5 * (u * TAU * 13.0 + phase * 2.7 + seed * 2.7).sin();
    let tip = 0.5 + 0.5 * (u * TAU * 23.0 - phase * 4.1 + seed * 3.5).sin();
    0.26 + 0.28 * billow + 0.34 * tongue + 0.16 * flicker + 0.08 * tip
}

fn ember_sparks(frame: &FxFrame<'_>, fire: FirePalette) -> Option<El> {
    let mut glow = Shape::new().view_box(frame.width, frame.height);
    let mut crisp = Shape::new().view_box(frame.width, frame.height);

    for i in 0..EMBER_COUNT {
        if let Some(spark) = sample_ember(frame, fire, i) {
            glow = glow.line(
                spark.tail,
                spark.head,
                spark.width + 3.2,
                LineCap::Round,
                spark.glow_color,
            );
            crisp = crisp.line(
                spark.tail,
                spark.head,
                spark.width,
                LineCap::Round,
                spark.core_color,
            );
        }
    }
    if crisp.is_empty() {
        return None;
    }
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(full_canvas(glow).blur(2.5))
            .child(full_canvas(crisp)),
    )
}

struct EmberSample {
    tail: [f32; 2],
    head: [f32; 2],
    width: f32,
    core_color: Color,
    glow_color: Color,
}

fn sample_ember(frame: &FxFrame<'_>, fire: FirePalette, index: usize) -> Option<EmberSample> {
    let h1 = hash01(index, 1);
    let h2 = hash01(index, 2);
    let h3 = hash01(index, 3);
    let h4 = hash01(index, 4);

    let spawn_t = 0.03 + h1 * 0.42;
    let life = 0.38 + h2 * 0.16;
    let age = segment(frame.t, spawn_t, spawn_t + life);
    if age <= 0.01 || age >= 0.99 {
        return None;
    }

    let alpha = envelope(age, 0.0, 0.16, 1.0);
    if alpha <= 0.01 {
        return None;
    }

    let rise = ease_out_cubic(age) * frame.height * (0.42 + 0.38 * h3);
    let start_y = frame.height * (0.86 + 0.12 * h4);
    let y = (start_y - rise).clamp(0.0, frame.height);

    let base_x = (0.04 + 0.92 * h2) * frame.width;
    let sway_amp = 14.0 + 26.0 * h4;
    let sway_phase = h1 * TAU;
    let dx = (age * (4.0 + 3.0 * h3) + sway_phase).sin() * sway_amp;
    let x = (base_x + dx).clamp(0.0, frame.width);

    let streak_len = (5.0 + 13.0 * h3) * (1.0 - 0.55 * age);
    let drift_slope = (age * (4.0 + 3.0 * h3) + sway_phase).cos() * 0.25;
    let tail = [
        x - drift_slope * streak_len,
        (y + streak_len).min(frame.height),
    ];

    // Cool from incandescent core -> amber -> crimson as the spark rises.
    let cool = if age < 0.45 {
        fire.core.mix(fire.amber, age / 0.45)
    } else {
        fire.amber.mix(fire.crimson, (age - 0.45) / 0.55)
    };
    let width = (1.6 + 1.8 * h4) * (1.0 - 0.35 * age);

    Some(EmberSample {
        tail,
        head: [x, y],
        width,
        core_color: fire.with_alpha(cool, 0.95 * alpha),
        glow_color: fire.with_alpha(fire.flame, 0.48 * alpha),
    })
}

/// Deterministic pseudo-random float in `0.0..1.0` from `(index, channel)`.
fn hash01(index: usize, channel: usize) -> f32 {
    let seed = (index as u32)
        .wrapping_mul(747_796_405)
        .wrapping_add((channel as u32).wrapping_mul(2_891_336_453))
        .wrapping_add(1_013_904_223);
    let mixed = ((seed >> ((seed >> 28).wrapping_add(4))) ^ seed).wrapping_mul(277_803_737);
    let final_bits = (mixed >> 22) ^ mixed;
    (final_bits & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn fire_emblem(frame: &FxFrame<'_>, fire: FirePalette) -> Option<El> {
    let enter_t = segment(frame.t, 0.06, 0.30);
    let exit_t = segment(frame.t, 0.60, 0.96);
    let opacity = ease_out_cubic(enter_t) * (1.0 - ease_in_out_cubic(exit_t));
    if opacity <= 0.005 {
        return None;
    }
    let look = frame.look;
    let scale = (0.72 + 0.28 * spring_overshoot(enter_t)) + 0.10 * ease_in_out_cubic(exit_t);
    let blur_in = 8.0 * (1.0 - ease_out_cubic(enter_t));
    let blur_out = 11.0 * ease_in_out_cubic(exit_t);
    let font = TextStyle {
        size: 30.0,
        weight: 800.0,
        tracking: 3.0,
        ..look.kit.ty.ui_medium
    };
    let badge = row()
        .px(24.0)
        .py(10.0)
        .center()
        .radius(look.round(12.0))
        .fill(fire.emblem_bg.alpha(0.92))
        .border(1.75, fire.with_alpha(fire.amber, 0.90))
        .box_shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 34.0,
            spread: 3.0,
            color: fire.with_alpha(fire.crimson, 0.72 * opacity),
            inset: false,
        })
        .box_shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 14.0,
            spread: 1.0,
            color: fire.with_alpha(fire.amber, 0.52 * opacity),
            inset: true,
        })
        .child(text("10X").font(font).color(fire.emblem_ink).a11y_hidden())
        .scale(scale)
        .blur(blur_in + blur_out)
        .layer_opacity(opacity);

    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .center()
            .child(badge),
    )
}

fn full_canvas(shape: Shape) -> El {
    canvas(shape)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full()
}

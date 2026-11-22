// SPDX-License-Identifier: MIT

//! Concept 2: **Velocity Warp** — radiating GPU vector speed-lines converging from window edges,
//! an expanding radial shockwave glow, and a spring-snapped glowing `10X` emblem that dissolves
//! via analytic Gaussian blur and fade.

use std::f32::consts::TAU;

use rune::{
    El, canvas, el,
    shape::{LineCap, Shape},
};

use super::palette::{
    EmblemSpec, FxFrame, centered_emblem, ease_in_out_cubic, ease_out_cubic, ease_out_expo,
    envelope, segment, spring_overshoot,
};

const RAY_COUNT: usize = 32;

pub(super) fn build(frame: &FxFrame<'_>) -> El {
    let mut root = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if let Some(wave) = radial_shockwave(frame) {
        root = root.child(wave);
    }
    if let Some(lines) = speed_lines(frame) {
        root = root.child(lines);
    }
    if let Some(badge) = warp_emblem(frame) {
        root = root.child(badge);
    }
    root
}

fn radial_shockwave(frame: &FxFrame<'_>) -> Option<El> {
    let alpha = envelope(frame.t, 0.0, 0.22, 0.88);
    if alpha <= 0.005 {
        return None;
    }
    let diag = frame.width.hypot(frame.height);
    let expand = ease_out_expo(segment(frame.t, 0.0, 0.84));
    let rx = (0.08 + 0.56 * expand) * diag.max(frame.width);
    let ry = (0.08 + 0.56 * expand) * diag.max(frame.height);
    let stops = [
        (0.0, frame.accent(0.0)),
        (0.52, frame.accent(0.05 * alpha)),
        (0.76, frame.focus(0.32 * alpha)),
        (0.88, frame.bright(0.42 * alpha)),
        (1.0, frame.accent(0.0)),
    ];
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radial_gradient([0.5, 0.5], [rx * 0.5, ry * 0.5], stops),
    )
}

fn speed_lines(frame: &FxFrame<'_>) -> Option<El> {
    let alpha = envelope(frame.t, 0.0, 0.18, 0.76);
    if alpha <= 0.005 {
        return None;
    }
    let (primary, glow) = build_ray_shapes(frame, alpha);
    if primary.is_empty() {
        return None;
    }
    let glow_layer = canvas(glow)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full()
        .blur(3.0);
    let crisp_layer = canvas(primary)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full();
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(glow_layer)
            .child(crisp_layer),
    )
}

fn build_ray_shapes(frame: &FxFrame<'_>, alpha: f32) -> (Shape, Shape) {
    let cx = frame.width * 0.5;
    let cy = frame.height * 0.5;
    let max_r = cx.hypot(cy) * 1.06;
    let mut primary = Shape::new().view_box(frame.width, frame.height);
    let mut glow = Shape::new().view_box(frame.width, frame.height);
    let travel = ease_out_cubic(segment(frame.t, 0.0, 0.72));

    for i in 0..RAY_COUNT {
        let idx = i as f32;
        let angle = (idx / RAY_COUNT as f32) * TAU + ((i % 3) as f32) * 0.025;
        let phase = ((i * 7) % 11) as f32 / 11.0;
        let ray_t = segment(travel, phase * 0.24, 0.68 + phase * 0.32);
        if ray_t <= 0.01 {
            continue;
        }
        let inner = max_r * (0.98 - 0.78 * ray_t).max(0.16);
        let streak_len = max_r * (0.14 + 0.24 * (1.0 - ray_t)) * (0.75 + 0.4 * (i % 2) as f32);
        let outer = (inner + streak_len).min(max_r);
        let (sin, cos) = angle.sin_cos();
        let a = [cx + cos * inner, cy + sin * inner];
        let b = [cx + cos * outer, cy + sin * outer];
        let is_major = i % 4 == 0;
        let color = if is_major {
            frame.bright(0.85 * alpha)
        } else {
            frame.focus(0.62 * alpha)
        };
        let width = if is_major { 2.4 } else { 1.4 };
        primary = primary.line(a, b, width, LineCap::Round, color);
        glow = glow.line(
            a,
            b,
            width + 3.0,
            LineCap::Round,
            frame.accent(0.38 * alpha),
        );
    }
    (primary, glow)
}

fn warp_emblem(frame: &FxFrame<'_>) -> Option<El> {
    let enter_t = segment(frame.t, 0.06, 0.36);
    let exit_t = segment(frame.t, 0.56, 0.96);
    let opacity = ease_out_cubic(enter_t) * (1.0 - ease_in_out_cubic(exit_t));
    if opacity <= 0.005 {
        return None;
    }
    let scale = (0.64 + 0.36 * spring_overshoot(enter_t)) + 0.14 * ease_in_out_cubic(exit_t);
    let blur = 11.0 * ease_in_out_cubic(exit_t);
    centered_emblem(
        frame,
        EmblemSpec {
            opacity,
            scale,
            blur,
            glow_alpha: 0.65 * opacity,
            accent_fill: true,
        },
    )
}

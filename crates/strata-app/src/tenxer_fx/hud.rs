// SPDX-License-Identifier: MIT

//! Concept 5: **Tactical HUD** — spring-overshoot animated corner targeting reticles, faint
//! transient coordinate micro-grid, and a high-contrast `10X` lock-in emblem with analytic
//! Gaussian blur-in and blur-out.

use rune::{
    El, canvas, el,
    shape::{LineCap, Shape},
};

use super::palette::{
    EmblemSpec, FxFrame, centered_emblem, ease_in_out_cubic, ease_out_cubic, envelope, segment,
    spring_overshoot,
};

const RETICLE_INSET_TARGET: f32 = 22.0;
const RETICLE_INSET_START: f32 = 66.0;
const MICRO_GRID_STEP: f32 = 56.0;

pub(super) fn build(frame: &FxFrame<'_>) -> El {
    let mut root = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if let Some(grid) = center_micro_grid(frame) {
        root = root.child(grid);
    }
    if let Some(reticles) = targeting_reticles(frame) {
        root = root.child(reticles);
    }
    if let Some(badge) = hud_emblem(frame) {
        root = root.child(badge);
    }
    root
}

struct ReticleStyle {
    inset: f32,
    arm: f32,
    color: rune::Color,
    tick: rune::Color,
}

fn targeting_reticles(frame: &FxFrame<'_>) -> Option<El> {
    let alpha = envelope(frame.t, 0.0, 0.22, 0.96);
    if alpha <= 0.005 {
        return None;
    }
    let lock = spring_overshoot(segment(frame.t, 0.0, 0.44));
    let style = ReticleStyle {
        inset: (RETICLE_INSET_START + (RETICLE_INSET_TARGET - RETICLE_INSET_START) * lock).max(8.0),
        arm: 30.0 + 6.0 * lock,
        color: frame.focus(0.92 * alpha),
        tick: frame.accent(0.55 * alpha),
    };

    let mut shape = Shape::new().view_box(frame.width, frame.height);
    shape = add_corner_reticles(shape, frame, &style);
    shape = add_mid_edge_ticks(shape, frame, style.inset, style.tick);
    shape = add_emblem_wings(shape, frame, lock, alpha);

    let glow = canvas(shape.clone())
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full()
        .blur(2.5)
        .layer_opacity(0.55);
    let crisp = canvas(shape)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full();
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(glow)
            .child(crisp),
    )
}

fn add_corner_reticles(mut shape: Shape, frame: &FxFrame<'_>, style: &ReticleStyle) -> Shape {
    let x0 = style.inset;
    let y0 = style.inset;
    let x1 = (frame.width - style.inset).max(x0 + 16.0);
    let y1 = (frame.height - style.inset).max(y0 + 16.0);
    let corners = [
        (x0, y0, 1.0, 1.0),
        (x1, y0, -1.0, 1.0),
        (x1, y1, -1.0, -1.0),
        (x0, y1, 1.0, -1.0),
    ];
    for (cx, cy, dx, dy) in corners {
        shape = shape.stroke_with(
            vec![
                [cx, cy + dy * style.arm],
                [cx, cy],
                [cx + dx * style.arm, cy],
            ],
            false,
            2.2,
            LineCap::Round,
            style.color,
        );
        shape = shape.line(
            [cx + dx * 8.0, cy + dy * 8.0],
            [cx + dx * 16.0, cy + dy * 16.0],
            1.4,
            LineCap::Round,
            style.tick,
        );
    }
    shape
}

fn add_mid_edge_ticks(
    mut shape: Shape,
    frame: &FxFrame<'_>,
    inset: f32,
    color: rune::Color,
) -> Shape {
    let mx = frame.width * 0.5;
    let my = frame.height * 0.5;
    let span = 14.0;
    shape = shape
        .line(
            [mx - span, inset],
            [mx + span, inset],
            1.5,
            LineCap::Round,
            color,
        )
        .line(
            [mx - span, frame.height - inset],
            [mx + span, frame.height - inset],
            1.5,
            LineCap::Round,
            color,
        )
        .line(
            [inset, my - span],
            [inset, my + span],
            1.5,
            LineCap::Round,
            color,
        )
        .line(
            [frame.width - inset, my - span],
            [frame.width - inset, my + span],
            1.5,
            LineCap::Round,
            color,
        );
    shape
}

fn add_emblem_wings(mut shape: Shape, frame: &FxFrame<'_>, lock: f32, alpha: f32) -> Shape {
    let mx = frame.width * 0.5;
    let my = frame.height * 0.5;
    let inner = 74.0;
    let wing = 44.0 + 38.0 * lock;
    let color = frame.focus(0.68 * alpha);
    shape = shape
        .line(
            [mx - inner - wing, my],
            [mx - inner, my],
            1.5,
            LineCap::Round,
            color,
        )
        .line(
            [mx + inner, my],
            [mx + inner + wing, my],
            1.5,
            LineCap::Round,
            color,
        );
    shape
}

fn center_micro_grid(frame: &FxFrame<'_>) -> Option<El> {
    let alpha = envelope(frame.t, 0.04, 0.24, 0.86);
    if alpha <= 0.01 {
        return None;
    }
    let cx = frame.width * 0.5;
    let cy = frame.height * 0.5;
    let rx = (frame.width * 0.34).max(120.0);
    let ry = (frame.height * 0.32).max(90.0);
    let mut shape = Shape::new().view_box(frame.width, frame.height);

    let cols = (rx / MICRO_GRID_STEP).floor() as i32;
    let rows = (ry / MICRO_GRID_STEP).floor() as i32;
    for gx in -cols..=cols {
        for gy in -rows..=rows {
            let px = cx + gx as f32 * MICRO_GRID_STEP;
            let py = cy + gy as f32 * MICRO_GRID_STEP;
            let nx = (px - cx) / rx;
            let ny = (py - cy) / ry;
            let falloff = (1.0 - (nx * nx + ny * ny).sqrt()).clamp(0.0, 1.0);
            if falloff <= 0.05 {
                continue;
            }
            let c = frame.accent(0.28 * alpha * falloff);
            shape = shape
                .line([px - 3.5, py], [px + 3.5, py], 1.0, LineCap::Butt, c)
                .line([px, py - 3.5], [px, py + 3.5], 1.0, LineCap::Butt, c);
        }
    }
    (!shape.is_empty()).then(|| {
        canvas(shape)
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .w_full()
            .h_full()
    })
}

fn hud_emblem(frame: &FxFrame<'_>) -> Option<El> {
    let enter_t = segment(frame.t, 0.0, 0.26);
    let exit_t = segment(frame.t, 0.58, 0.98);
    let opacity = ease_out_cubic(enter_t) * (1.0 - ease_in_out_cubic(exit_t));
    if opacity <= 0.005 {
        return None;
    }
    let blur_in = 12.0 * (1.0 - ease_out_cubic(enter_t));
    let blur_out = 10.0 * ease_in_out_cubic(exit_t);
    let scale = 0.92 + 0.08 * spring_overshoot(enter_t);
    centered_emblem(
        frame,
        EmblemSpec {
            opacity,
            scale,
            blur: blur_in + blur_out,
            glow_alpha: 0.48 * opacity,
            accent_fill: false,
        },
    )
}

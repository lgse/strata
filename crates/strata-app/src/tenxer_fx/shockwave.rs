// SPDX-License-Identifier: MIT

//! Concept 3: **Shockwave Ring** — concentric theme-accent frame ripples expanding from the
//! center (respecting `look.round(...)` corner roundness dynamically) with snapping accent
//! corner brackets.

use rune::{
    El, Shadow, canvas, el,
    shape::{LineCap, Shape},
};

use super::palette::{FxFrame, ease_out_expo, envelope, segment, spring_overshoot};

struct RingWave {
    start: f32,
    end: f32,
    border_width: f32,
    alpha_scale: f32,
    base_radius: f32,
}

const WAVES: [RingWave; 3] = [
    RingWave {
        start: 0.0,
        end: 0.72,
        border_width: 2.5,
        alpha_scale: 0.92,
        base_radius: 18.0,
    },
    RingWave {
        start: 0.11,
        end: 0.85,
        border_width: 1.75,
        alpha_scale: 0.65,
        base_radius: 14.0,
    },
    RingWave {
        start: 0.22,
        end: 0.98,
        border_width: 1.25,
        alpha_scale: 0.42,
        base_radius: 10.0,
    },
];

pub(super) fn build(frame: &FxFrame<'_>) -> El {
    let mut root = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if let Some(pulse) = center_pulse(frame) {
        root = root.child(pulse);
    }
    for wave in &WAVES {
        if let Some(ring) = frame_ripple(frame, wave) {
            root = root.child(ring);
        }
    }
    if let Some(brackets) = corner_brackets(frame) {
        root = root.child(brackets);
    }
    root
}

fn center_pulse(frame: &FxFrame<'_>) -> Option<El> {
    let alpha = envelope(frame.t, 0.0, 0.08, 0.42);
    if alpha <= 0.005 {
        return None;
    }
    let radius =
        frame.width.min(frame.height) * (0.14 + 0.34 * ease_out_expo(segment(frame.t, 0.0, 0.42)));
    let stops = [
        (0.0, frame.bright(0.32 * alpha)),
        (0.45, frame.accent(0.16 * alpha)),
        (1.0, frame.accent(0.0)),
    ];
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radial_gradient([0.5, 0.5], radius, stops),
    )
}

fn frame_ripple(frame: &FxFrame<'_>, wave: &RingWave) -> Option<El> {
    let local_t = segment(frame.t, wave.start, wave.end);
    if local_t <= 0.002 || local_t >= 0.998 {
        return None;
    }
    let expand = ease_out_expo(local_t);
    let fade = (1.0 - local_t).powf(1.35) * wave.alpha_scale;
    if fade <= 0.005 {
        return None;
    }
    let frac = 0.14 + 0.88 * expand;
    let w = frame.width * frac;
    let h = frame.height * frac;
    let x = (frame.width - w) * 0.5;
    let y = (frame.height - h) * 0.5;
    // Honors CornerRoundness: 0% yields crisp sharp rectangles, rounded themes yield smooth ripples.
    let corner = frame.look.round(wave.base_radius * (1.15 - 0.35 * expand));
    Some(
        el().absolute(x, y)
            .size(w, h)
            .radius(corner)
            .border(wave.border_width, frame.focus(0.88 * fade))
            .box_shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 18.0,
                spread: 1.0,
                color: frame.accent(0.42 * fade),
                inset: false,
            })
            .box_shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 10.0,
                spread: 0.0,
                color: frame.bright(0.24 * fade),
                inset: true,
            }),
    )
}

struct BracketSpec {
    inset: f32,
    arm: f32,
    color: rune::Color,
    glow: rune::Color,
}

fn corner_brackets(frame: &FxFrame<'_>) -> Option<El> {
    let alpha = envelope(frame.t, 0.06, 0.28, 0.96);
    if alpha <= 0.005 {
        return None;
    }
    let snap = spring_overshoot(segment(frame.t, 0.06, 0.46));
    let spec = BracketSpec {
        inset: 54.0 - 42.0 * snap,
        arm: 26.0 + 10.0 * snap,
        color: frame.focus(0.92 * alpha),
        glow: frame.accent(0.46 * alpha),
    };
    let (crisp, glow) = build_bracket_shapes(frame, &spec);
    let glow_el = canvas(glow)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full()
        .blur(3.5);
    let crisp_el = canvas(crisp)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full();
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(glow_el)
            .child(crisp_el),
    )
}

fn build_bracket_shapes(frame: &FxFrame<'_>, spec: &BracketSpec) -> (Shape, Shape) {
    let x0 = spec.inset.max(6.0);
    let y0 = spec.inset.max(6.0);
    let x1 = (frame.width - x0).max(x0 + 8.0);
    let y1 = (frame.height - y0).max(y0 + 8.0);
    let arm = spec.arm;
    let corners = [
        ([x0, y0 + arm], [x0, y0], [x0 + arm, y0]),
        ([x1 - arm, y0], [x1, y0], [x1, y0 + arm]),
        ([x1, y1 - arm], [x1, y1], [x1 - arm, y1]),
        ([x0 + arm, y1], [x0, y1], [x0, y1 - arm]),
    ];
    let mut crisp = Shape::new().view_box(frame.width, frame.height);
    let mut glow = Shape::new().view_box(frame.width, frame.height);
    for (a, b, c) in corners {
        crisp = crisp.stroke_with(vec![a, b, c], false, 2.5, LineCap::Round, spec.color);
        glow = glow.stroke_with(vec![a, b, c], false, 6.0, LineCap::Round, spec.glow);
    }
    (crisp, glow)
}

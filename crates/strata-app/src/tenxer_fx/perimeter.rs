// SPDX-License-Identifier: MIT

//! Concept 4: **Perimeter Ignite** — dual glowing accent sparks launching from the bottom-right
//! `10X` footer badge region and racing in opposite directions along rounded window perimeter
//! vector paths (`Shape::trimmed`) with glowing trails until colliding at top-left and flashing
//! the border.

use rune::{
    El, canvas, el,
    shape::{LineCap, Shape},
};

use super::palette::{
    FxFrame, ease_in_out_cubic, ease_out_expo, envelope, frame_border_pulse, perimeter_halves,
    perimeter_head_point, segment,
};

const EDGE_INSET: f32 = 3.0;
const TRAIL_SPAN: f32 = 0.28;
const CORE_SPAN: f32 = 0.065;

pub(super) fn build(frame: &FxFrame<'_>) -> El {
    let race_t = ease_in_out_cubic(segment(frame.t, 0.0, 0.62));
    let trail_alpha = envelope(frame.t, 0.0, 0.08, 0.78);
    let collision_alpha = envelope(frame.t, 0.56, 0.65, 1.0);
    let border_alpha = envelope(frame.t, 0.54, 0.66, 1.0);

    let mut root = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if let Some(border) = frame_border_pulse(frame, border_alpha) {
        root = root.child(border);
    }
    if trail_alpha > 0.005 {
        root = root.child(racing_trails(frame, race_t, trail_alpha));
        root = root.child(spark_flares(frame, race_t, trail_alpha));
    }
    if let Some(launch) = point_flare(
        frame,
        [frame.width - EDGE_INSET, frame.height - EDGE_INSET],
        envelope(frame.t, 0.0, 0.06, 0.28),
        68.0,
    ) {
        root = root.child(launch);
    }
    if let Some(clash) = point_flare(
        frame,
        [EDGE_INSET, EDGE_INSET],
        collision_alpha,
        96.0 + 72.0 * ease_out_expo(segment(frame.t, 0.56, 0.92)),
    ) {
        root = root.child(clash);
    }
    root
}

struct StrokeSpec {
    span: (f32, f32),
    width: f32,
    color: rune::Color,
}

fn racing_trails(frame: &FxFrame<'_>, head: f32, alpha: f32) -> El {
    let tail = (head - TRAIL_SPAN).max(0.0);
    let core_tail = (head - CORE_SPAN).max(0.0);
    let paths = perimeter_halves(frame, EDGE_INSET);

    let bloom = trim_dual_paths(
        &paths,
        frame,
        StrokeSpec {
            span: (tail, head),
            width: 10.0,
            color: frame.accent(0.45 * alpha),
        },
    );
    let trail = trim_dual_paths(
        &paths,
        frame,
        StrokeSpec {
            span: (tail, head),
            width: 4.0,
            color: frame.focus(0.84 * alpha),
        },
    );
    let core = trim_dual_paths(
        &paths,
        frame,
        StrokeSpec {
            span: (core_tail, head),
            width: 2.2,
            color: frame.bright(0.98 * alpha),
        },
    );

    let bloom_el = canvas(bloom)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full()
        .blur(4.0);
    let trail_el = canvas(trail)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full();
    let core_el = canvas(core)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .w_full()
        .h_full();

    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(bloom_el)
        .child(trail_el)
        .child(core_el)
}

fn trim_dual_paths(
    (first, second): &(rune::shape::Path, rune::shape::Path),
    frame: &FxFrame<'_>,
    spec: StrokeSpec,
) -> Shape {
    let part_a = Shape::new()
        .view_box(frame.width, frame.height)
        .stroke_path(first, spec.width, LineCap::Round, spec.color)
        .trimmed(spec.span.0, spec.span.1);
    let part_b = Shape::new()
        .view_box(frame.width, frame.height)
        .stroke_path(second, spec.width, LineCap::Round, spec.color)
        .trimmed(spec.span.0, spec.span.1);
    let mut combined = Shape::new().view_box(frame.width, frame.height);
    combined.ops.extend(part_a.ops);
    combined.ops.extend(part_b.ops);
    combined
}

fn spark_flares(frame: &FxFrame<'_>, head: f32, alpha: f32) -> El {
    let p_a = perimeter_head_point(frame, EDGE_INSET, head, false);
    let p_b = perimeter_head_point(frame, EDGE_INSET, head, true);
    let mut layer = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if let Some(flare) = point_flare(frame, p_a, alpha, 44.0) {
        layer = layer.child(flare);
    }
    if let Some(flare) = point_flare(frame, p_b, alpha, 44.0) {
        layer = layer.child(flare);
    }
    layer
}

fn point_flare(frame: &FxFrame<'_>, pos: [f32; 2], alpha: f32, diameter: f32) -> Option<El> {
    if alpha <= 0.005 || diameter <= 1.0 {
        return None;
    }
    let half = diameter * 0.5;
    let stops = [
        (0.0, frame.bright(0.95 * alpha)),
        (0.28, frame.focus(0.62 * alpha)),
        (0.62, frame.accent(0.22 * alpha)),
        (1.0, frame.accent(0.0)),
    ];
    Some(
        el().absolute(pos[0] - half, pos[1] - half)
            .size(diameter, diameter)
            .radial_gradient([0.5, 0.5], half, stops),
    )
}

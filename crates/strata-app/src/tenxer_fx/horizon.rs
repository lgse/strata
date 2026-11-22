// SPDX-License-Identifier: MIT

//! Concept 1: **Horizon Scan** — a theme-accent laser scanline with trailing linear gradient
//! sweep, transient laser-illuminated telemetry micro-grid, and glowing accent frame lock pulse.

use std::f32::consts::PI;

use rune::{
    El, Shadow, canvas, el,
    shape::{LineCap, Shape},
};

use super::palette::{FxFrame, ease_out_cubic, envelope, frame_border_pulse, segment};

const WAKE_HEIGHT: f32 = 156.0;
const GRID_STEP: f32 = 64.0;
const GRID_BAND: f32 = 132.0;

pub(super) fn build(frame: &FxFrame<'_>) -> El {
    let sweep_t = segment(frame.t, 0.0, 0.78);
    let scan_y = ease_out_cubic(sweep_t) * (frame.height + 28.0) - 14.0;
    let beam_alpha = envelope(frame.t, 0.0, 0.10, 0.82);
    let frame_alpha = envelope(frame.t, 0.44, 0.68, 1.0);

    let mut root = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if let Some(grid) = telemetry_grid(frame, scan_y, beam_alpha) {
        root = root.child(grid);
    }
    if beam_alpha > 0.005 {
        root = root
            .child(laser_wake(frame, scan_y, beam_alpha))
            .child(laser_beam(frame, scan_y, beam_alpha));
    }
    if let Some(border) = frame_border_pulse(frame, frame_alpha) {
        root = root.child(border);
    }
    root
}

fn laser_wake(frame: &FxFrame<'_>, scan_y: f32, alpha: f32) -> El {
    let top = scan_y - WAKE_HEIGHT;
    let stops = [
        (0.0, frame.accent(0.0)),
        (0.45, frame.accent(0.06 * alpha)),
        (0.82, frame.accent(0.18 * alpha)),
        (1.0, frame.focus(0.38 * alpha)),
    ];
    el().absolute(0.0, top)
        .size(frame.width, WAKE_HEIGHT)
        .linear_gradient(PI, stops)
}

fn laser_beam(frame: &FxFrame<'_>, scan_y: f32, alpha: f32) -> El {
    let bloom = el()
        .absolute(0.0, scan_y - 4.0)
        .size(frame.width, 8.0)
        .fill(frame.accent(0.55 * alpha))
        .blur(4.0);
    let core = el()
        .absolute(0.0, scan_y - 1.25)
        .size(frame.width, 2.5)
        .fill(frame.bright(0.95 * alpha))
        .box_shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 18.0,
            spread: 1.0,
            color: frame.accent(0.75 * alpha),
            inset: false,
        });
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(bloom)
        .child(core)
}

struct GridBand {
    scan_y: f32,
    top: f32,
    bottom: f32,
    alpha: f32,
}

fn telemetry_grid(frame: &FxFrame<'_>, scan_y: f32, alpha: f32) -> Option<El> {
    if alpha <= 0.01 {
        return None;
    }
    let top = (scan_y - GRID_BAND).max(0.0);
    let bottom = (scan_y + 12.0).min(frame.height);
    if bottom <= top {
        return None;
    }
    let band = GridBand {
        scan_y,
        top,
        bottom,
        alpha,
    };
    let mut shape = Shape::new().view_box(frame.width, frame.height);
    shape = add_horizontal_lines(shape, frame, &band);
    shape = add_vertical_ticks(shape, frame, &band);
    (!shape.is_empty()).then(|| {
        canvas(shape)
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .w_full()
            .h_full()
    })
}

fn add_horizontal_lines(mut shape: Shape, frame: &FxFrame<'_>, band: &GridBand) -> Shape {
    let first = (band.top / GRID_STEP).ceil().max(1.0) * GRID_STEP;
    let mut y = first;
    while y < band.bottom {
        let proximity = 1.0 - ((band.scan_y - y).abs() / GRID_BAND).clamp(0.0, 1.0);
        let line_alpha = 0.22 * band.alpha * proximity * proximity;
        if line_alpha > 0.01 {
            shape = shape.line(
                [0.0, y],
                [frame.width, y],
                1.0,
                LineCap::Butt,
                frame.focus(line_alpha),
            );
        }
        y += GRID_STEP;
    }
    shape
}

fn add_vertical_ticks(mut shape: Shape, frame: &FxFrame<'_>, band: &GridBand) -> Shape {
    let mut x = GRID_STEP;
    while x < frame.width {
        let line_alpha = 0.14 * band.alpha;
        shape = shape.line(
            [x, band.top],
            [x, band.bottom],
            1.0,
            LineCap::Butt,
            frame.accent(line_alpha),
        );
        let cross_y = (band.scan_y / GRID_STEP).floor() * GRID_STEP;
        if cross_y > band.top && cross_y < band.bottom {
            let color = frame.bright(0.38 * band.alpha);
            shape = shape
                .line(
                    [x - 4.0, cross_y],
                    [x + 4.0, cross_y],
                    1.25,
                    LineCap::Round,
                    color,
                )
                .line(
                    [x, cross_y - 4.0],
                    [x, cross_y + 4.0],
                    1.25,
                    LineCap::Round,
                    color,
                );
        }
        x += GRID_STEP;
    }
    shape
}

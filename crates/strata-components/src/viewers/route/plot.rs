//! The elevation profile, drawn with `canvas` shapes from the samples' heights, and the markers
//! that ride both pictures.
//!
//! **Deviation from Strata** (coordinator ruling): Strata's helper draws the profile into an
//! 800 × 70 PNG in the theme's accent, and the drawer stretches it to the pane
//! (`ContentFit::Fill`). Here the same line is drawn from the samples, in the same 800 × 70
//! space stretched to the element (a `Shape::view_box`, whose strokes stay in points), from Kit
//! tokens, so a theme switch recolours it without a new render and it stays sharp at any width.
//! The helper's profile PNG is left unused; the sandbox protocol is unchanged.

use rune::{El, Shape, canvas, el};
use strata_kit::look::Look;

use super::state::RouteData;
use super::trace;

/// The profile's height in points (Strata's 45).
pub const PROFILE_HEIGHT: f32 = 45.0;
/// The profile line's width, in points (the helper's 2 px stroke).
const LINE: f32 = 2.0;
/// The helper skips points closer than this to the last one it drew, in profile pixels.
const THIN: f32 = 0.6;
/// The marker's radius and its ring (Strata's 4.5 and 2).
const MARKER: f32 = 4.5;
const RING: f32 = 2.0;

/// The profile's polylines in profile pixels: one per run of connected samples with heights,
/// thinned as the helper thins them, so a 250,000-point route draws a few thousand points.
pub(super) fn profile_lines(data: &RouteData) -> Vec<Vec<[f32; 2]>> {
    let width = f64::from(data.profile_px[0]);
    let mut lines = Vec::new();
    let mut line: Vec<[f32; 2]> = Vec::new();
    let mut flush = |line: &mut Vec<[f32; 2]>| {
        match line.len() {
            0 => {}
            // A lone point still draws, as a dot.
            1 => {
                let p = line[0];
                lines.push(vec![p, [p[0] + 0.01, p[1]]]);
            }
            _ => lines.push(std::mem::take(line)),
        }
        line.clear();
    };
    for s in &data.samples {
        let point = s.elevation_y.map(|y| {
            [
                trace::profile_x(s.distance_m, data.distance_m, width) as f32,
                y,
            ]
        });
        if s.starts_segment || point.is_none() {
            flush(&mut line);
        }
        if let Some(p) = point {
            let far = line
                .last()
                .is_none_or(|q| (p[0] - q[0]).hypot(p[1] - q[1]) >= THIN);
            if far {
                line.push(p);
            } else if let Some(last) = line.last_mut() {
                // Keep the run's true end.
                *last = p;
            }
        }
    }
    flush(&mut line);
    lines
}

/// The profile line as a shape over the profile's pixel space.
pub(super) fn profile_shape(lines: &[Vec<[f32; 2]>], px: [f32; 2], look: &Look) -> El {
    let shape = lines
        .iter()
        .fold(Shape::new(), |s, l| {
            s.stroke(l.clone(), LINE, look.kit.c.text_accent)
        })
        .view_box(px[0], px[1]);
    canvas(shape).w_full().h_full().a11y_hidden()
}

/// A marker dot: the accent with a ring of the background, centred on its live offset.
pub(super) fn marker(key: String, at: &rune::Live<[f32; 2]>, look: &Look) -> El {
    let k = &look.kit;
    let side = (MARKER + RING) * 2.0;
    el().key(key)
        .pin(Some(-side / 2.0), Some(-side / 2.0), None, None)
        .size(side, side)
        .radius(side / 2.0)
        .fill(k.c.text_accent)
        .border(RING, k.c.bg_base)
        .live_offset(at)
        .a11y_hidden()
}

/// The profile's cursor: a hairline down its height at the marker's distance.
pub(super) fn cursor_line(key: String, at: &rune::Live<[f32; 2]>, look: &Look) -> El {
    el().key(key)
        .pin(Some(-0.5), Some(0.0), None, Some(0.0))
        .w(1.0)
        .fill(look.kit.c.border_strong)
        .live_offset(at)
        .a11y_hidden()
}

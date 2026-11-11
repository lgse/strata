//! The camera indicator between the camera buttons: a map from above, with a hollow circle for
//! the model in the middle and a dot on the outer ring for the camera (Strata's
//! `camera_position` and `draw_camera_position`).
//!
//! The dot sits in a box the size of the map, which turns about its centre (`El::rotate`) to the
//! camera's angle on a 240 ms emphasized-decelerate tween. The state keeps the angle unwrapped,
//! so a turn takes the shorter way. The turn lands at once when the indicator first mounts, when
//! a new file opens (`sweep` false: the dot keeps its key, so without that it would sweep back
//! from the last file's view), under reduced motion (Rune's own), and with the look's animations
//! off.

use std::f32::consts::TAU;

use rune::{Anim, Color, CubicBezier, Curve, Easing, El, Shape, canvas, el};
use strata_kit::look::Look;

/// The map's side, in points (Strata's 64 px drawing area).
pub const SIZE: f32 = 64.0;
/// The hollow centre circle's radius.
const CENTRE: f32 = 8.0;
/// The dot's radius.
const DOT: f32 = 3.5;
/// Room for the dot to straddle the ring without clipping.
const INSET: f32 = 4.0;
/// Strata's sweep: 240 ms on `emphasized_deceleration`.
const SWEEP: f32 = 0.24;

/// The indicator, keyed `key`: an image named `label` and described by `view` (no visible
/// caption), with the dot at `angle` (radians, clockwise from the right). A change of angle
/// sweeps when `sweep` and the look's animations are on, and lands at once otherwise.
pub fn indicator(
    key: String,
    label: &str,
    view: String,
    angle: f32,
    sweep: bool,
    look: &Look,
) -> El {
    let k = &look.kit;
    let c = SIZE / 2.0;
    let ring = c - INSET;
    let with_alpha = |color: Color, a: f32| Color {
        a: color.a * a,
        ..color
    };
    let map = Shape::new()
        .fill(circle([c, c], ring, 48), with_alpha(k.c.bg_base, 0.72))
        .stroke(
            closed(circle([c, c], ring, 48)),
            1.0,
            with_alpha(k.c.border_base, 0.42),
        )
        .stroke(
            closed(circle([c, c], CENTRE, 24)),
            1.5,
            with_alpha(k.c.text_accent, 0.8),
        );
    let motion: Anim = if sweep && look.animations {
        Curve::tween(
            SWEEP,
            Easing::CubicBezier(CubicBezier::new(0.16, 1.0, 0.3, 1.0)),
        )
        .into()
    } else {
        Curve::Instant.into()
    };
    el().key(key.clone())
        .role("image")
        .label(label.to_owned())
        .description(view)
        .size(SIZE, SIZE)
        .child(canvas(map).size(SIZE, SIZE).a11y_hidden())
        .child(
            el().key(format!("{key}-dot"))
                .pin(Some(0.0), Some(0.0), None, None)
                .size(SIZE, SIZE)
                .rotate(angle)
                .rotate_motion(motion)
                .a11y_hidden()
                .child(
                    el().pin(Some(c + ring - DOT), Some(c - DOT), None, None)
                        .size(DOT * 2.0, DOT * 2.0)
                        .radius(DOT)
                        .fill(k.c.text_accent),
                ),
        )
}

/// `n` points round a circle.
fn circle(centre: [f32; 2], r: f32, n: usize) -> Vec<[f32; 2]> {
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * TAU;
            [centre[0] + r * a.cos(), centre[1] + r * a.sin()]
        })
        .collect()
}

/// The same points with the first repeated at the end: a closed stroke.
fn closed(mut points: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    if let Some(&first) = points.first() {
        points.push(first);
    }
    points
}

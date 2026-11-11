// SPDX-License-Identifier: MIT

//! Boxes and points in screen coordinates, and the whole-pixel tests the predicates make on them.

use std::collections::HashMap;

use crate::{At, PointAxis};

/// The boxes `settle`, `remember` and `keep` keep under a name.
pub type Boxes = HashMap<String, Rect>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn centre(&self) -> (f64, f64) {
        (
            self.x + (self.width / 2.0).floor(),
            self.y + (self.height / 2.0).floor(),
        )
    }

    pub fn point(&self, at: Option<&At>) -> (f64, f64) {
        match at {
            None => self.centre(),
            Some(at) => (
                at.x.resolve(self.x, self.width),
                at.y.resolve(self.y, self.height),
            ),
        }
    }

    /// Whole pixels, as AT-SPI reports extents; sub-pixel drift is not movement.
    pub fn pixels(&self) -> [i64; 4] {
        [self.x, self.y, self.width, self.height].map(|value| value.floor() as i64)
    }
}

/// `Missing` is accepted by `absent` and the patient lookups; `Broken` is always an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupError {
    Missing(String),
    Broken(String),
}

impl LookupError {
    pub fn message(&self) -> &str {
        match self {
            Self::Missing(message) | Self::Broken(message) => message,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aim {
    Point,
    DragOrigin,
}

/// A screen coordinate the X protocol can carry.
pub fn coordinate(value: f64) -> Result<i16, String> {
    let value = value.floor();
    if value < f64::from(i16::MIN) || value > f64::from(i16::MAX) {
        return Err(format!("the target lies outside the screen at {value}"));
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the range was checked just above"
    )]
    Ok(value as i16)
}

/// Python's floor division, so both harnesses visit the same pixels.
pub fn step_towards(from: i16, to: i16, step: u32, steps: u32) -> i16 {
    let travelled =
        ((i64::from(to) - i64::from(from)) * i64::from(step)).div_euclid(i64::from(steps));
    i16::try_from(i64::from(from) + travelled).unwrap_or(to)
}

pub fn within_viewport(editor: [i64; 4], pane_x: i64, window_width: i64) -> bool {
    let [x, _, width, height] = editor;
    height > 0 && pane_x <= x && x < window_width && 0 < width && width <= window_width - pane_x
}

pub fn within_window(editor: [i64; 4], pane_x: i64, window_width: i64) -> bool {
    let [x, _, width, _] = editor;
    width > 1 && x >= pane_x.max(0) && x + width <= window_width
}

pub fn inside(inner: [i64; 4], outer: [i64; 4], axis: Option<PointAxis>) -> bool {
    let [x, y, width, height] = inner;
    let [outer_x, outer_y, outer_width, outer_height] = outer;
    let across = x >= outer_x && x + width <= outer_x + outer_width;
    let down = y >= outer_y && y + height <= outer_y + outer_height;
    width > 0
        && height > 0
        && match axis {
            None => across && down,
            Some(PointAxis::X) => across,
            Some(PointAxis::Y) => down,
        }
}

pub fn on_screen(bounds: [i64; 4], window: [i64; 4]) -> bool {
    let [_, y, _, height] = bounds;
    height > 0 && y >= window[1] && y + height <= window[1] + window[3]
}

pub fn contains(bounds: [i64; 4], point: (i64, i64)) -> bool {
    let [x, y, width, height] = bounds;
    x <= point.0 && point.0 < x + width && y <= point.1 && point.1 < y + height
}

pub fn middle_visible(
    entries: &[(String, [i64; 4])],
    viewport: [i64; 4],
    unless_first: &str,
) -> Option<String> {
    let [_, top, _, height] = viewport;
    let visible: Vec<&String> = entries
        .iter()
        .filter(|(_, bounds)| top < bounds[1] && bounds[1] < top + height - bounds[3])
        .map(|(name, _)| name)
        .collect();
    visible
        .first()
        .filter(|first| first.as_str() != unless_first)
        .map(|_| visible[visible.len() / 2].clone())
}

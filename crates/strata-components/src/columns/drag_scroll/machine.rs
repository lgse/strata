//! The drag autoscroll's state machine: where the held drag is, how long it has dwelt near an
//! edge, and what each frame scrolls. Plain data with no Rune runtime; the view applies its
//! [`Step`].

use std::time::Duration;

use rune::Spring;

use super::speed::{
    MAX_HORIZONTAL_SPEED, MAX_VERTICAL_SPEED, band_direction, edge_direction, scroll_speed,
};
use crate::collection::marquee::glide_lead;
use crate::contract::{Point, Rect};

/// The first frame's length, before one is measured.
const FRAME: Duration = Duration::from_millis(16);

/// The longest frame a step counts: a stall doesn't jump the view (Strata's `MAX_FRAME`).
const MAX_FRAME: Duration = Duration::from_millis(50);

/// The glide each frame retargets: Rune's, as the marquee's edge scroll uses.
pub(crate) const GLIDE: Spring = rune::tok::motion::GLIDE;

/// Where the strip and its columns are, in window points, for one frame.
#[derive(Clone, Debug, Default)]
pub struct StripGeometry {
    /// The strip's viewport.
    pub strip: Rect,
    /// How far it is scrolled sideways.
    pub offset: f32,
    /// How far it can scroll sideways.
    pub max: f32,
    /// Each column, root first.
    pub columns: Vec<ColumnGeometry>,
}

/// One column, in window points.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColumnGeometry {
    /// Its left edge.
    pub left: f32,
    /// Its right edge.
    pub right: f32,
    /// Its listing's viewport.
    pub listing: Rect,
    /// How far the listing is scrolled.
    pub offset: f32,
    /// How far it can scroll.
    pub max: f32,
}

/// A scroll a frame asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    /// Glide to this offset (a lead ahead of where the view is).
    Glide(f32),
    /// Jump here: a step with animations off, or a dead stop where the view is.
    Instant(f32),
}

/// What a frame does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    /// The strip, sideways.
    pub strip: Option<Scroll>,
    /// A column's listing, down: the column and the scroll.
    pub listing: Option<(usize, Scroll)>,
    /// Another frame is wanted.
    pub wake: bool,
}

/// The column strip's drag autoscroll, and the column a held drag is over.
#[derive(Clone, Debug, Default)]
pub struct DragScroll {
    /// The drag, in window points, while an accepted drag is over the strip.
    pointer: Option<Point>,
    /// When the strip started scrolling in the direction it scrolls, and which.
    burst_x: Option<(f64, i8)>,
    /// When a listing started scrolling, which way, and which column.
    burst_y: Option<(f64, i8, usize)>,
    ticked: Option<f64>,
    /// The column under the drag, as last worked out.
    hovered: Option<usize>,
}

impl DragScroll {
    /// Nothing held.
    pub fn new() -> Self {
        Self::default()
    }

    /// Where an accepted drag is now (`Some`, window points), or that it left the strip, was
    /// dropped, ended or was refused (`None`). True when a frame is wanted: it reached the
    /// strip, or left it while scrolling.
    pub fn over(&mut self, at: Option<Point>) -> bool {
        let was = self.pointer.is_some() || self.scrolling();
        self.pointer = at;
        at.is_some() || was
    }

    /// Where the drag is, while one is over the strip.
    pub fn pointer(&self) -> Option<Point> {
        self.pointer
    }

    /// The column under the drag, as of the last frame (or [`DragScroll::hover`]).
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// Whether a frame scrolled something.
    pub fn scrolling(&self) -> bool {
        self.burst_x.is_some() || self.burst_y.is_some()
    }

    /// Works out the column under the drag in `geo`. True when it changed.
    pub fn hover(&mut self, geo: &StripGeometry) -> bool {
        let hovered = self.pointer.and_then(|p| column_at(geo, p));
        let changed = hovered != self.hovered;
        self.hovered = hovered;
        changed
    }

    /// Forgets the drag, scrolling nothing more (a folder change, the view gone).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// One frame at `now` (seconds): what to scroll, gliding (or stepping, with `animations`
    /// off) toward the edge the drag is near, and stopping dead where the view is when it
    /// leaves the edge, can't go further, or the drag goes.
    pub fn step(&mut self, now: f64, geo: &StripGeometry, animations: bool) -> Step {
        let dt = self
            .ticked
            .map(|t| now - t)
            .filter(|dt| *dt > 0.0)
            .map_or(FRAME, |dt| Duration::from_secs_f64(dt).min(MAX_FRAME));
        self.ticked = Some(now);
        self.hover(geo);
        let mut step = Step::default();
        let Some(p) = self.pointer else {
            if self.burst_x.take().is_some() {
                step.strip = Some(Scroll::Instant(geo.offset));
            }
            if let Some((_, _, depth)) = self.burst_y.take()
                && let Some(c) = geo.columns.get(depth)
            {
                step.listing = Some((depth, Scroll::Instant(c.offset)));
            }
            self.ticked = None;
            return step;
        };
        let along = |speed: f32, offset: f32| {
            if animations {
                Scroll::Glide(offset + glide_lead(speed, GLIDE, dt))
            } else {
                Scroll::Instant(offset + speed * dt.as_secs_f32())
            }
        };

        // Sideways: near the strip's left or right edge, while it can go that way.
        let inside = geo.strip.contains(p);
        let dx = if inside {
            edge_direction(p.x - geo.strip.x, geo.strip.w)
        } else {
            0.0
        };
        let dx = if advanceable(geo.offset, geo.max, dx) {
            dx
        } else {
            0.0
        };
        match sign(dx) {
            0 => {
                if self.burst_x.take().is_some() {
                    step.strip = Some(Scroll::Instant(geo.offset));
                }
            }
            s => {
                let start = match self.burst_x {
                    Some((t, was)) if was == s => t,
                    _ => now,
                };
                self.burst_x = Some((start, s));
                let dwell = Duration::from_secs_f64((now - start).max(0.0));
                let speed = scroll_speed(dx, dwell, MAX_HORIZONTAL_SPEED);
                step.strip = Some(along(speed, geo.offset));
                step.wake = true;
            }
        }

        // Down: near the top or bottom of the listing under the drag.
        let vertical = inside
            .then(|| column_at(geo, p))
            .flatten()
            .and_then(|depth| {
                let c = geo.columns[depth];
                let dy = band_direction(p.y, c.listing.y, c.listing.h);
                advanceable(c.offset, c.max, dy).then_some((depth, dy))
            })
            .filter(|(_, dy)| sign(*dy) != 0);
        match vertical {
            None => {
                if let Some((_, _, depth)) = self.burst_y.take()
                    && let Some(c) = geo.columns.get(depth)
                {
                    step.listing = Some((depth, Scroll::Instant(c.offset)));
                }
            }
            Some((depth, dy)) => {
                let s = sign(dy);
                if let Some((_, _, was)) = self.burst_y
                    && was != depth
                    && let Some(c) = geo.columns.get(was)
                {
                    // Moved to another column's edge: the one it left stops where it is.
                    // (One listing scrolls a frame; the new one starts on the next.)
                    self.burst_y = None;
                    step.listing = Some((was, Scroll::Instant(c.offset)));
                    step.wake = true;
                    return step;
                }
                let start = match self.burst_y {
                    Some((t, was, _)) if was == s => t,
                    _ => now,
                };
                self.burst_y = Some((start, s, depth));
                let dwell = Duration::from_secs_f64((now - start).max(0.0));
                let speed = scroll_speed(dy, dwell, MAX_VERTICAL_SPEED);
                step.listing = Some((depth, along(speed, geo.columns[depth].offset)));
                step.wake = true;
            }
        }
        if !step.wake {
            // At rest near no edge: no frames until the drag moves.
            self.ticked = None;
        }
        step
    }
}

/// The column whose span holds `p`'s x, while `p` is in the strip.
pub(crate) fn column_at(geo: &StripGeometry, p: Point) -> Option<usize> {
    if !geo.strip.contains(p) {
        return None;
    }
    geo.columns
        .iter()
        .position(|c| p.x >= c.left && p.x < c.right)
}

fn sign(x: f32) -> i8 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

/// Whether a view at `offset` of `max` can go the way `direction` points.
fn advanceable(offset: f32, max: f32, direction: f32) -> bool {
    if direction < 0.0 {
        offset > 0.5
    } else if direction > 0.0 {
        offset < max - 0.5
    } else {
        false
    }
}

// SPDX-License-Identifier: MIT

use rune::WheelEvent;

use super::range::Range;

/// What a change to the value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderChange {
    /// A drag moved it: apply it, but don't save yet.
    Live,
    Commit,
    Cancel,
}

/// A drag's start: the value it began at and where on the knob it was grabbed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Grab {
    start: f64,
    offset: f32,
}

/// A slider's state, held by the host and reached through accessors.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderState {
    range: Range,
    value: f64,
    grab: Option<Grab>,
    /// The track's laid-out width, in points (0 until the first layout).
    width: f32,
    /// Wheel travel not yet a whole step (precise deltas).
    wheel: f32,
}

const WHEEL_PITCH: f32 = 40.0;

pub(crate) fn takes_wheel(w: &WheelEvent) -> bool {
    let m = w.mods;
    w.dy != 0.0 && !(m.ctrl || m.alt || m.cmd || m.logo)
}

pub(crate) fn wheel_notches(pending: &mut f32, dy: f32, precise: bool) -> f32 {
    if dy == 0.0 {
        return 0.0;
    }
    if !precise {
        return dy.signum();
    }
    *pending += dy / WHEEL_PITCH;
    let whole = pending.trunc();
    *pending -= whole;
    whole
}

impl SliderState {
    /// A slider over `range` at `value` (clamped).
    pub fn new(value: f64, range: Range) -> SliderState {
        SliderState {
            range,
            value: range.clamp(value),
            grab: None,
            width: 0.0,
            wheel: 0.0,
        }
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn range(&self) -> Range {
        self.range
    }

    pub fn set_value(&mut self, value: f64) {
        self.value = self.range.clamp(value);
    }

    /// While a drag is in progress.
    pub fn dragging(&self) -> bool {
        self.grab.is_some()
    }

    #[doc(hidden)]
    pub fn hold(&mut self) {
        self.grab = Some(Grab {
            start: self.value,
            offset: 0.0,
        });
    }

    pub(super) fn width(&self) -> f32 {
        self.width
    }

    /// Records the track's width; true when it changed.
    pub(super) fn set_width(&mut self, w: f32) -> bool {
        let moved = (self.width - w).abs() > 0.01;
        self.width = w;
        moved
    }

    pub(super) fn x_of(&self, value: f64, knob: f32) -> f32 {
        let travel = (self.width - knob).max(0.0);
        knob / 2.0 + self.range.fraction(value) as f32 * travel
    }

    /// The raw value under a knob centred at `x`.
    fn value_at(&self, x: f32, knob: f32) -> f64 {
        let travel = (self.width - knob).max(1.0);
        self.range.at(f64::from((x - knob / 2.0) / travel))
    }

    fn drag_value(&self, x: f32, knob: f32, snaps: &[f64], snap_within: f32, free: bool) -> f64 {
        let raw = self.value_at(x, knob);
        let near = snaps
            .iter()
            .map(|&s| (s, (self.x_of(s, knob) - self.x_of(raw, knob)).abs()))
            .filter(|&(_, d)| d < snap_within)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match near {
            Some((s, _)) => self.range.clamp(s),
            None if free => self.range.clamp(raw),
            None => self.range.snap_to_grid(raw),
        }
    }

    pub(super) fn press(
        &mut self,
        x: f32,
        knob: f32,
        snaps: &[f64],
        snap_within: f32,
        free: bool,
    ) -> Option<SliderChange> {
        let centre = self.x_of(self.value, knob);
        let on_knob = (x - centre).abs() <= knob / 2.0;
        self.grab = Some(Grab {
            start: self.value,
            offset: if on_knob { x - centre } else { 0.0 },
        });
        if on_knob {
            return None;
        }
        self.move_to(self.drag_value(x, knob, snaps, snap_within, free))
            .then_some(SliderChange::Live)
    }

    /// The pointer moved to track point `x` while dragging.
    pub(super) fn drag(
        &mut self,
        x: f32,
        knob: f32,
        snaps: &[f64],
        snap_within: f32,
        free: bool,
    ) -> Option<SliderChange> {
        let grab = self.grab?;
        self.move_to(self.drag_value(x - grab.offset, knob, snaps, snap_within, free))
            .then_some(SliderChange::Live)
    }

    pub(super) fn release(&mut self, cancelled: bool) -> Option<SliderChange> {
        let grab = self.grab.take()?;
        let moved = self.value != grab.start;
        if cancelled {
            self.value = grab.start;
            return moved.then_some(SliderChange::Cancel);
        }
        moved.then_some(SliderChange::Commit)
    }

    pub(super) fn step(&mut self, steps: i32) -> bool {
        self.move_to(self.range.stepped(self.value, steps))
    }

    /// To the minimum (`false`) or the maximum. True when the value moved.
    pub(super) fn jump_to_end(&mut self, max: bool) -> bool {
        let r = self.range;
        self.move_to(if max { r.max() } else { r.min() })
    }

    /// Straight to `value` (a snap label). True when the value moved.
    pub(super) fn set(&mut self, value: f64) -> bool {
        self.move_to(self.range.clamp(value))
    }

    pub(super) fn wheel(&mut self, dy: f32, precise: bool, amount: f64) -> bool {
        let notches = wheel_notches(&mut self.wheel, dy, precise);
        if notches == 0.0 {
            return false;
        }
        self.move_to(self.range.clamp(self.value + f64::from(notches) * amount))
    }

    fn move_to(&mut self, v: f64) -> bool {
        let moved = v != self.value;
        self.value = v;
        moved
    }
}

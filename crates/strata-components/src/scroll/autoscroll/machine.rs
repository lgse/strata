//! The autoscroll state machine: idle, running from an anchor, or stopping. Plain data with no
//! Rune runtime; what it asks of Rune goes through [`Out`].

use rune::{PointerButton, Spring, Tx};

use super::speed::{AutoscrollSettings, speed};
use crate::collection::marquee::glide_lead;
use crate::contract::{Point, Rect};
use crate::scroll::ScrollFeed;

/// Why an autoscroll stopped, as the host hears it.
///
/// Used by: hosts (an Escape also cancels an armed 10xer chord).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stop {
    /// Escape.
    Escape,
    /// A press of this button, anywhere in the window.
    Press(PointerButton),
    /// The listing went: another folder, hidden or closed.
    Gone,
}

/// A viewport autoscroll moves, the axes it moves it on, and where it reads the viewport's
/// position from.
///
/// Used by: C24's collection adapter, C4 (its sideways viewport), hosts.
#[derive(Clone, Debug)]
pub struct Target {
    pub(crate) path: String,
    pub(crate) feed: ScrollFeed,
    pub(crate) x: bool,
    pub(crate) y: bool,
}

impl Target {
    /// The vertical viewport keyed `path` (a key or key path), whose position `feed` holds.
    pub fn vertical(path: impl Into<String>, feed: ScrollFeed) -> Self {
        Self::new(path, feed, false, true)
    }

    /// The sideways viewport keyed `path` (`El::scroll_x`).
    pub fn horizontal(path: impl Into<String>, feed: ScrollFeed) -> Self {
        Self::new(path, feed, true, false)
    }

    /// The viewport keyed `path` that scrolls both ways (`El::scroll_xy`).
    pub fn both(path: impl Into<String>, feed: ScrollFeed) -> Self {
        Self::new(path, feed, true, true)
    }

    fn new(path: impl Into<String>, feed: ScrollFeed, x: bool, y: bool) -> Self {
        Self {
            path: path.into(),
            feed,
            x,
            y,
        }
    }

    /// Whether the viewport can scroll on an axis this target moves.
    fn scrollable(&self) -> bool {
        let max = self.feed.get().max;
        (self.x && max[0] > 0.5) || (self.y && max[1] > 0.5)
    }
}

/// What starts an autoscroll.
///
/// Used by: C24's collection adapter, hosts with a plain viewport.
#[derive(Clone, Debug)]
pub struct Start {
    /// Who started it: the collection's key, say. [`super::Autoscroll::stop_gone`] stops only
    /// its owner's.
    pub owner: String,
    /// The press, in window points: where the anchor shows.
    pub anchor: Point,
    /// The viewport's rect in window points: the anchor is clipped to it and the pointer
    /// shows the scroll cursor over it.
    pub viewport: Rect,
    /// The viewports to move. Autoscroll starts only if one of them can scroll.
    pub targets: Vec<Target>,
}

/// What the machine asks of Rune: a [`Tx`] in the app, a recorder in tests.
pub(crate) trait Out {
    /// Glide the viewport at `path` to (`x`, `y`) on `spring`.
    fn glide(&mut self, path: &str, x: f32, y: f32, spring: Spring);
    /// Jump the viewport at `path` to (`x`, `y`): also stops a glide in progress.
    fn jump(&mut self, path: &str, x: f32, y: f32);
    /// Run the next frame.
    fn wake(&mut self);
}

impl Out for Tx {
    fn glide(&mut self, path: &str, x: f32, y: f32, spring: Spring) {
        self.scroll_to_with(path.to_owned(), x, y, spring);
    }

    fn jump(&mut self, path: &str, x: f32, y: f32) {
        self.scroll_to_instant(path.to_owned(), x, y);
    }

    fn wake(&mut self) {
        self.wake_after(std::time::Duration::ZERO);
    }
}

#[derive(Debug)]
pub(super) struct Run {
    pub(super) owner: String,
    pub(super) anchor: Point,
    pub(super) pointer: Point,
    pub(super) viewport: Rect,
    targets: Vec<Target>,
    settings: AutoscrollSettings,
    /// When the last step ran, while the machine keeps waking; `None` once it sleeps.
    last: Option<f64>,
    /// The last step moved a viewport (and asked for the next frame).
    moving: bool,
    /// Which targets the last step moved, one per target: a target that stopped (at its end, or
    /// in the dead zone on its axes) is stopped once, not on every frame another moves.
    moved: Vec<bool>,
}

#[derive(Debug, Default)]
pub(super) enum State {
    #[default]
    Idle,
    Running(Run),
    /// A stop that has no transaction yet (Escape, through the modal's dismiss): the next step
    /// stops the viewports where they are.
    Stopping(Run),
}

/// The machine. One runs per window (Strata's "only one autoscroll at a time").
#[derive(Debug, Default)]
pub(crate) struct Machine {
    pub(super) state: State,
}

impl Machine {
    /// Starts from `start`, unless one is running or nothing it names can scroll: a listing
    /// that fits keeps the press for other handlers. True when it started.
    pub(crate) fn start(&mut self, start: Start, settings: AutoscrollSettings) -> bool {
        if !matches!(self.state, State::Idle) || !start.targets.iter().any(Target::scrollable) {
            return false;
        }
        let moved = vec![false; start.targets.len()];
        self.state = State::Running(Run {
            moved,
            owner: start.owner,
            anchor: start.anchor,
            pointer: start.anchor,
            viewport: start.viewport,
            targets: start.targets,
            settings,
            last: None,
            moving: false,
        });
        true
    }

    /// The running autoscroll, while one runs (not while it stops).
    pub(super) fn running(&self) -> Option<&Run> {
        match &self.state {
            State::Running(run) => Some(run),
            _ => None,
        }
    }

    /// Whether an autoscroll runs or is stopping.
    pub(crate) fn active(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    /// Whose autoscroll runs or stops.
    pub(crate) fn owner(&self) -> Option<&str> {
        match &self.state {
            State::Running(run) | State::Stopping(run) => Some(&run.owner),
            State::Idle => None,
        }
    }

    /// The pointer moved to `p` (window points). True when the machine sleeps and should wake:
    /// the pointer left the dead zone, or turned back from an end it had reached.
    pub(crate) fn track(&mut self, p: Point) -> bool {
        let State::Running(run) = &mut self.state else {
            return false;
        };
        run.pointer = p;
        !run.moving && run.speeds().iter().any(|v| v.0 != 0.0 || v.1 != 0.0)
    }

    /// Stops. With nothing moving it is idle at once; else it stops at the next
    /// [`Self::step`], which has the transaction to stop the glide with.
    pub(crate) fn request_stop(&mut self) {
        match std::mem::take(&mut self.state) {
            State::Running(run) if run.moving => self.state = State::Stopping(run),
            State::Stopping(run) => self.state = State::Stopping(run),
            _ => {}
        }
    }

    /// Stops now, the viewports where they are (Strata's timer stops dead). True when one ran.
    pub(crate) fn stop_now(&mut self, out: &mut dyn Out) -> bool {
        match std::mem::take(&mut self.state) {
            State::Running(run) | State::Stopping(run) => {
                run.halt(out);
                true
            }
            State::Idle => false,
        }
    }

    /// Stops without moving anything (the listing went, or another scroll request replaces the
    /// glide). `owner` limits it to that owner's; `None` stops any. True when one stopped.
    pub(crate) fn stop_gone(&mut self, owner: Option<&str>) -> bool {
        if self.owner().is_none_or(|o| owner.is_some_and(|w| w != o)) {
            return false;
        }
        self.state = State::Idle;
        true
    }

    /// One frame at `now` (seconds): glides each viewport toward the pointer, or steps it
    /// when animations are off. True when what the host draws changed (a stop finished).
    pub(crate) fn step(&mut self, now: f64, animations: bool, out: &mut dyn Out) -> bool {
        match &mut self.state {
            State::Idle => false,
            State::Stopping(_) => self.stop_now(out),
            State::Running(run) => {
                run.step(now, animations, out);
                false
            }
        }
    }
}

impl Run {
    /// Each target's speed on its axes, (x, y) in points a second: zero on an axis it doesn't
    /// move, and toward an end it has reached.
    fn speeds(&self) -> Vec<(f32, f32)> {
        let dx = speed(self.pointer.x - self.anchor.x, &self.settings);
        let dy = speed(self.pointer.y - self.anchor.y, &self.settings);
        self.targets
            .iter()
            .map(|t| {
                let p = t.feed.get();
                let along = |on: bool, v: f32, i: usize| {
                    let at_end = (v > 0.0 && p.offset[i] >= p.max[i] - 0.5)
                        || (v < 0.0 && p.offset[i] <= 0.5);
                    if on && !at_end { v } else { 0.0 }
                };
                (along(t.x, dx, 0), along(t.y, dy, 1))
            })
            .collect()
    }

    fn step(&mut self, now: f64, animations: bool, out: &mut dyn Out) {
        let frame = self.settings.frame.as_secs_f32();
        let dt = self
            .last
            .map(|l| (now - l) as f32)
            .filter(|d| *d > 0.0)
            .unwrap_or(frame)
            .clamp(1.0 / 240.0, 0.1);
        let spring = self.settings.glide;
        let mut moved = false;
        let speeds = self.speeds();
        for ((t, (vx, vy)), was) in self.targets.iter().zip(speeds).zip(self.moved.iter_mut()) {
            let p = t.feed.get();
            let [ox, oy] = p.offset;
            if vx == 0.0 && vy == 0.0 {
                // Into the dead zone or at an end: stop the glide where the view is, as
                // Strata's step does nothing then; once, as it stops.
                if std::mem::take(was) {
                    out.jump(&t.path, ox, oy);
                }
                continue;
            }
            *was = true;
            moved = true;
            let to = |o: f32, v: f32, max: f32| {
                let ahead = if animations {
                    // A glide aimed this far ahead each frame settles at `v` (C1d's probe).
                    glide_lead(v, spring, std::time::Duration::from_secs_f32(dt))
                } else {
                    // Rune lands a glide at once with motion off: aim one frame's travel ahead.
                    v * dt
                };
                (o + ahead).clamp(0.0, max.max(0.0))
            };
            let (x, y) = (to(ox, vx, p.max[0]), to(oy, vy, p.max[1]));
            if animations {
                out.glide(&t.path, x, y, spring);
            } else {
                out.jump(&t.path, x, y);
            }
        }
        self.moving = moved;
        if moved {
            self.last = Some(now);
            out.wake();
        } else {
            // Asleep: the pointer's next move out of the dead zone wakes it, and its first step
            // then assumes one frame rather than the time it slept.
            self.last = None;
        }
    }

    /// Stops every viewport that is moving where it is.
    fn halt(&self, out: &mut dyn Out) {
        for (t, _) in self.targets.iter().zip(&self.moved).filter(|(_, m)| **m) {
            let [x, y] = t.feed.get().offset;
            out.jump(&t.path, x, y);
        }
    }
}

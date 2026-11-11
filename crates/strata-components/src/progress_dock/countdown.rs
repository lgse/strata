//! [`Countdown`]: a finished card's time before it closes itself, paused while the pointer is
//! over it or it is pinned, then a short fade (Strata's `progress_dock/completion.rs`).

/// How long a completed card stays (Strata's 5 s).
pub const COMPLETE_FOR: f64 = 5.0;

/// The fade before a card that ran out goes, with animations on (Strata's 180 ms).
pub const FADE: f64 = 0.18;

/// Due times within this of the clock count as reached.
const EPS: f64 = 1e-6;

/// A time stamped at the next tick. The dock's cards start from handlers that have a `Tx`, so
/// `Tx::clock` (Rune c650fe0c) could stamp them, except C15's dialog buttons, which carry no
/// `Tx` ([`super::Dock::cancel_untimed`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Since {
    Pending,
    At(f64),
}

impl Since {
    /// The stamped time, stamping it at `now` if it is still pending.
    pub(crate) fn at(&mut self, now: f64) -> f64 {
        match *self {
            Since::Pending => {
                *self = Since::At(now);
                now
            }
            Since::At(t) => t,
        }
    }
}

/// What a tick changed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Step {
    /// The view shows something else: the words, the bar's pose, the fade.
    pub changed: bool,
    /// Counting began again from `left`: the bar's tween starts over.
    pub restarted: bool,
    /// The fade began.
    pub leaving: bool,
    /// The card is due to go.
    pub done: bool,
    /// When the next tick is due, in seconds from now.
    pub wake: Option<f64>,
}

/// A countdown with Strata's rules:
///
/// - It counts down from `duration`, and closes after a fade of [`FADE`] (none with animations
///   off) once it reaches zero.
/// - Hover or the pin pauses it, keeping the time left; leaving or unpinning resumes from there.
///   A pause during the fade stops the fade, and resuming starts it again (no time is left).
/// - Changes from handlers land at the next tick, which stamps them with the clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Countdown {
    duration: f64,
    /// The time left when counting last stopped (or began).
    remaining: f64,
    /// Counting since; `None` while paused.
    counting: Option<Since>,
    /// Fading since.
    leaving: Option<Since>,
    hovered: bool,
    pinned: bool,
    fade: f64,
    /// The time left at the last tick, for the words.
    shown_left: f64,
}

impl Countdown {
    /// A countdown of `duration` seconds from the next tick. `animations` off drops the fade.
    pub fn new(duration: f64, animations: bool) -> Self {
        Countdown {
            duration,
            remaining: duration,
            counting: Some(Since::Pending),
            leaving: None,
            hovered: false,
            pinned: false,
            fade: if animations { FADE } else { 0.0 },
            shown_left: duration,
        }
    }

    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// The pointer is over the card. True when that changes anything.
    pub fn set_hovered(&mut self, hovered: bool) -> bool {
        std::mem::replace(&mut self.hovered, hovered) != hovered
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    /// Toggles the pin.
    pub fn toggle_pin(&mut self) {
        self.pinned = !self.pinned;
    }

    /// Paused by hover or the pin.
    pub fn paused(&self) -> bool {
        self.hovered || self.pinned
    }

    /// Fading out.
    pub fn leaving(&self) -> bool {
        self.leaving.is_some()
    }

    /// The time left at the last tick, in seconds.
    pub fn left(&self) -> f64 {
        self.shown_left
    }

    /// How long the fade takes.
    pub fn fade(&self) -> f64 {
        self.fade
    }

    /// One tick at `now` seconds.
    pub(crate) fn tick(&mut self, now: f64) -> Step {
        let mut step = Step::default();
        if self.paused() {
            if let Some(mut since) = self.counting.take() {
                let at = since.at(now);
                self.remaining = (self.remaining - (now - at)).max(0.0);
                step.changed = true;
            }
            if self.leaving.take().is_some() {
                step.changed = true;
            }
            if self.shown_left != self.remaining {
                self.shown_left = self.remaining;
                step.changed = true;
            }
            return step;
        }
        let start = match &mut self.counting {
            Some(since) => {
                let pending = *since == Since::Pending;
                let at = since.at(now);
                if pending {
                    step.restarted = true;
                    step.changed = true;
                }
                at
            }
            None => {
                self.counting = Some(Since::At(now));
                step.restarted = true;
                step.changed = true;
                now
            }
        };
        let left = (self.remaining - (now - start)).max(0.0);
        if self.leaving.is_none() {
            if left <= EPS {
                self.leaving = Some(Since::At(now));
                step.leaving = true;
                step.changed = true;
                self.shown_left = 0.0;
            } else {
                if left.ceil() != self.shown_left.ceil() {
                    step.changed = true;
                }
                self.shown_left = left;
                // The words show whole seconds: wake when the next one starts.
                let whole = left.ceil() - 1.0;
                step.wake = Some((left - whole).max(EPS));
                return step;
            }
        }
        if let Some(mut since) = self.leaving {
            let at = since.at(now);
            let end = at + self.fade;
            if now + EPS >= end {
                step.done = true;
                step.changed = true;
            } else {
                step.wake = Some(end - now);
            }
        }
        step
    }
}

//! [`Effects`]: the host's state for the motion effects, their ids, timers and events.

use std::time::Duration;

use rune::Tx;
use strata_kit::look::Look;

use super::dissolve::{Dissolve, DissolveRun};
use super::flight::{Flight, FlightRun};
pub(super) use super::pulse::Pulses;

/// One effect, as [`Effects::fly`] and [`Effects::dissolve`] return it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectId(pub(super) u64);

/// What an effect tells its host, from [`Effects::tick`] and [`Effects::settle`].
///
/// A flight that [`Effects::fly`] started sends `Launched`, `Landed` and `Done`, once each
/// and in that order, whether or not anything flew (reduced motion, no layer, every leg or the
/// target out of sight), and however it ends (on time, its target gone, or settled). Two or
/// all three can come in one tick. A dissolve sends `Done` only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A flight's icons have left their sources: remove the rows they left now, in the commit
    /// this poll makes (and not an instant one: see the module docs).
    Launched(EffectId),
    /// A flight's icons have landed, or nothing flew: show the rows a restore flew onto.
    Landed(EffectId),
    /// The effect is over: remove a dissolve's rows now. Every effect ends with it.
    Done(EffectId),
}

/// The motion effects playing in one window: the host holds it, and its view mounts a
/// [`MotionLayer`](super::MotionLayer) over everything.
///
/// The host calls [`Effects::tick`] from its poll every frame and acts on the events. Effects
/// ask for the wakes they need, so a window at rest wakes at those times only.
#[derive(Default)]
pub struct Effects {
    next: u64,
    pub(super) flights: Vec<FlightRun>,
    pub(super) dissolve: Option<DissolveRun>,
    pub(super) pulses: Pulses,
}

impl std::fmt::Debug for Effects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effects")
            .field("flights", &self.flights.len())
            .field("dissolving", &self.dissolve.is_some())
            .finish_non_exhaustive()
    }
}

impl Effects {
    /// Nothing playing.
    pub fn new() -> Self {
        Self::default()
    }

    fn id(&mut self) -> EffectId {
        self.next += 1;
        EffectId(self.next)
    }

    /// Starts `flight`. `None` when nothing plays (animations are off, or it has no legs): apply
    /// the end state now.
    ///
    /// Call it while the source rows still show, and remove them at [`Event::Launched`].
    pub fn fly(&mut self, flight: Flight, look: &Look, tx: &mut Tx) -> Option<EffectId> {
        if !look.animations || flight.legs.is_empty() {
            return None;
        }
        let id = self.id();
        self.flights.push(FlightRun::new(id, flight));
        tx.wake_after(Duration::ZERO);
        Some(id)
    }

    /// Starts `dissolve`. `None` when nothing plays (animations are off, no rows, or a dissolve
    /// is already playing): apply the end state now.
    ///
    /// In the same handler, mark its rows vanished (`RowMarks::vanished`) and keep them listed
    /// until [`Event::Done`], so the rows around them hold still while it plays.
    pub fn dissolve(&mut self, dissolve: Dissolve, look: &Look, tx: &mut Tx) -> Option<EffectId> {
        if !look.animations || dissolve.rows.is_empty() || self.dissolve.is_some() {
            return None;
        }
        let id = self.id();
        self.dissolve = Some(DissolveRun::new(id, dissolve));
        tx.wake_after(Duration::ZERO);
        Some(id)
    }

    /// One frame at `now` seconds on the host's clock: moves the effects on, asks `tx` for the
    /// next wake they need, and returns what happened, in order.
    pub fn tick(&mut self, now: f64, tx: &mut Tx) -> Vec<Event> {
        let mut events = Vec::new();
        let mut next = None;
        let pulses = &mut self.pulses;
        self.flights
            .retain_mut(|f| f.step(now, &mut events, &mut next, pulses));
        if let Some(d) = &mut self.dissolve
            && !d.step(now, &mut events, &mut next)
        {
            self.dissolve = None;
        }
        if let Some(at) = next {
            tx.wake_after(Duration::from_secs_f64((at - now).max(0.0)));
        }
        events
    }

    /// Ends every effect now (a tab or view-mode switch under them), with the events each
    /// still owed. Their targets' pulses end too: [`dress_target`](super::dress_target) keeps
    /// its wrapper, playing nothing.
    pub fn settle(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for f in self.flights.drain(..) {
            f.spend_pulse(&mut self.pulses);
            if matches!(f.phase, super::flight::Phase::Mount { .. }) {
                events.push(Event::Launched(f.id));
            }
            if !matches!(
                f.phase,
                super::flight::Phase::Fly {
                    landed: Some(_),
                    ..
                }
            ) {
                events.push(Event::Landed(f.id));
            }
            events.push(Event::Done(f.id));
        }
        if let Some(d) = self.dissolve.take() {
            events.push(Event::Done(d.id));
        }
        events
    }

    /// Whether any effect plays.
    pub fn playing(&self) -> bool {
        !self.flights.is_empty() || self.dissolve.is_some()
    }

    /// Whether a dissolve plays (a second one would not).
    pub fn dissolving(&self) -> bool {
        self.dissolve.is_some()
    }
}

// SPDX-License-Identifier: MIT

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Launched(EffectId),
    /// A flight's icons have landed, or nothing flew: show the rows a restore flew onto.
    Landed(EffectId),
    /// The effect is over: remove a dissolve's rows now. Every effect ends with it.
    Done(EffectId),
}

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

pub(super) struct FlightTick<'a> {
    pub events: &'a mut Vec<Event>,
    pub next: &'a mut Option<f64>,
    pub pulses: &'a mut Pulses,
    pub animate: bool,
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

    pub fn fly(&mut self, flight: Flight, look: &Look, tx: &mut Tx) -> Option<EffectId> {
        if !look.animations || flight.legs.is_empty() {
            return None;
        }
        let id = self.id();
        self.flights.push(FlightRun::new(id, flight));
        tx.wake_after(Duration::ZERO);
        Some(id)
    }

    pub fn dissolve(&mut self, dissolve: Dissolve, look: &Look, tx: &mut Tx) -> Option<EffectId> {
        if !look.animations || dissolve.rows.is_empty() || self.dissolve.is_some() {
            return None;
        }
        let id = self.id();
        self.dissolve = Some(DissolveRun::new(id, dissolve));
        tx.wake_after(Duration::ZERO);
        Some(id)
    }

    pub fn tick(&mut self, now: f64, tx: &mut Tx) -> Vec<Event> {
        let mut events = Vec::new();
        let mut next = None;
        let mut flight_tick = FlightTick {
            events: &mut events,
            next: &mut next,
            pulses: &mut self.pulses,
            animate: false,
        };
        self.flights.retain_mut(|f| f.step(now, &mut flight_tick));
        if flight_tick.animate {
            tx.animated();
        }
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

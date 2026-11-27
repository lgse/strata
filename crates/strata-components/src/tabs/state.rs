// SPDX-License-Identifier: MIT

use std::time::Duration;

use rune::Tx;
use rune_kit::shell::TabDragState;

use super::data::{TabId, TabsData, TitleFormat, window_title};
use super::drag::ReorderState;
use super::intent::{DropSite, TabsIntent, Via};
use super::probe::{Probe, Reveal};
use super::{scroll_key, tab_key};
use crate::contract::dnd::DropHover;

/// How long a file or content drag rests on a tab before it is selected (Strata's 450 ms).
pub const HOVER_DELAY: f64 = 0.45;

/// A drag resting on a tab, waiting to select it.
#[derive(Clone, Debug, PartialEq)]
struct Spring {
    id: TabId,
    delay: f64,
    since: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LeftLanding {
    slot: usize,
    /// Ticks seen since the leave.
    ticks: u8,
}

/// How many ticks a left gap's slot outlives its leave: a frame or two.
const LEFT_LANDING_TICKS: u8 = 2;

/// Ticks a reveal waits for its tab to draw before it gives up.
const REVEAL_TICKS: u8 = 3;

/// What a tick asks of the host: the intents to route, and whether the state changed.
#[derive(Clone, Debug, Default, PartialEq)]
#[must_use]
pub struct Polled {
    pub intents: Vec<TabsIntent>,
    pub changed: bool,
}

#[derive(Clone)]
pub struct BrowserTabsState {
    key: String,
    title_format: TitleFormat,
    /// Legacy drag-state handle kept for callers passing `|s| &mut s.tabs.drag`.
    pub drag: TabDragState,
    /// Variable-width strip drag, tear-out, incoming drop gap, and scroll tracking.
    pub(super) reorder: ReorderState,
    drop: Option<(DropSite, DropHover)>,
    open_drag: bool,
    /// The zone whose moves place the incoming gap for a drag that opens tabs.
    incoming_from: Option<DropSite>,
    left_landing: Option<LeftLanding>,
    spring: Option<Spring>,
    title: Option<String>,
    active: Option<TabId>,
    /// A newly active tab to bring clear of the edge fades, and the ticks it has waited.
    reveal: Option<(TabId, u8)>,
    pub(super) probe: Probe,
}

impl Default for BrowserTabsState {
    fn default() -> Self {
        BrowserTabsState::new(super::DEFAULT_KEY)
    }
}

impl BrowserTabsState {
    /// A strip keyed `key`: its elements' keys start with it ([`tab_key`], [`super::new_tab_key`]).
    pub fn new(key: impl Into<String>) -> Self {
        BrowserTabsState {
            key: key.into(),
            title_format: TitleFormat::default(),
            drag: TabDragState::new(),
            reorder: ReorderState::default(),
            drop: None,
            open_drag: false,
            incoming_from: None,
            left_landing: None,
            spring: None,
            title: None,
            active: None,
            reveal: None,
            probe: Probe::default(),
        }
    }

    /// The same state, titling the window with `format`.
    pub fn title_format(mut self, format: TitleFormat) -> Self {
        self.title_format = format;
        self
    }

    /// The strip's key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Where a drag is and how, if one is over the strip.
    pub fn drop_hover(&self) -> Option<(DropSite, DropHover)> {
        self.drop
    }

    pub fn set_drop_hover(&mut self, site: DropSite, hover: DropHover) {
        match hover {
            DropHover::Left if self.drop.is_some_and(|(s, _)| s == site) => self.drop = None,
            DropHover::Left => {}
            _ => self.drop = Some((site, hover)),
        }
    }

    pub fn set_open_drag(&mut self, on: bool) {
        self.open_drag = on;
    }

    pub fn open_drag(&self) -> bool {
        self.open_drag
    }

    /// The tab a resting drag will select, if one is armed.
    pub fn spring(&self) -> Option<TabId> {
        self.spring.as_ref().map(|s| s.id)
    }

    pub fn set_incoming(&mut self, point: Option<[f32; 2]>) {
        self.reorder.set_incoming(point);
        self.drag.set_incoming(point);
    }

    /// Where an incoming tab drag lands if released now, resolved against the laid-out tab widths.
    pub fn incoming_slot(&self) -> Option<usize> {
        self.reorder.incoming_slot()
    }

    pub fn hand_off(&mut self) {
        self.reorder.hand_off();
        self.drag.hand_off();
    }

    /// Whether the torn-out tab has been handed off to its own window.
    pub fn handed_off(&self) -> bool {
        self.reorder.handed_off() || self.drag.handed_off()
    }

    /// Cancels any reorder or tear-out gesture currently in flight.
    pub fn cancel_drag(&mut self) {
        self.reorder.cancel();
        self.drag.cancel();
    }

    pub fn tick(&mut self, now: f64, data: &TabsData<'_>, tx: &mut Tx) -> Polled {
        let mut out = Polled::default();
        if self.reorder.incoming_slot().is_some() {
            // A newer gap built: it is the drop's slot now.
            self.left_landing = None;
        }
        if let Some(left) = self.left_landing.as_mut() {
            left.ticks += 1;
            if left.ticks > LEFT_LANDING_TICKS {
                self.left_landing = None;
            } else {
                tx.wake_after(Duration::ZERO);
            }
        }
        if let Some(spring) = self.spring.as_mut() {
            if data.get(spring.id).is_none() || data.active == Some(spring.id) {
                self.spring = None;
            } else {
                let since = *spring.since.get_or_insert(now);
                let left = spring.delay - (now - since);
                if left <= 1e-6 {
                    let id = spring.id;
                    self.spring = None;
                    out.intents.push(TabsIntent::Select {
                        id,
                        via: Via::Spring,
                    });
                } else {
                    tx.wake_after(Duration::from_secs_f64(left));
                }
            }
        }
        // The user's wheel since the last tick wins over a reveal still to land (review 2).
        self.probe.frame_begins();
        let wheeled = self.probe.take_wheel();
        if wheeled {
            self.reveal = None;
        }
        self.reveal_step(tx);
        if data.active != self.active {
            self.active = data.active;
            self.reveal = None;
            match data.active {
                _ if wheeled => {}
                Some(id) if self.reorder.is_idle() => {
                    self.reveal = Some((id, 0));
                    tx.wake_after(Duration::ZERO);
                }
                // A press that starts a drag: rune-kit revealed the tab it lifts.
                Some(id) => tx.scroll_into_view_in(scroll_key(&self.key), tab_key(&self.key, id)),
                None => {}
            }
        }
        let title = window_title(
            data.active_tab().map(|t| t.name.as_ref()),
            &self.title_format,
        );
        if self.title.as_deref() != Some(title.as_str()) {
            self.title = Some(title.clone());
            out.intents.push(TabsIntent::Title(title));
        }
        out
    }

    fn reveal_step(&mut self, tx: &mut Tx) {
        let Some((id, waited)) = self.reveal else {
            return;
        };
        match self.probe.reveal(id) {
            Reveal::Pending if waited < REVEAL_TICKS => {
                self.reveal = Some((id, waited + 1));
                tx.wake_after(Duration::ZERO);
            }
            // It never drew where it could be read: Rune's reveal, to the edge at least.
            Reveal::Pending => {
                self.reveal = None;
                tx.scroll_into_view_in(scroll_key(&self.key), tab_key(&self.key, id));
            }
            Reveal::Clear => self.reveal = None,
            Reveal::To(x) => {
                self.reveal = None;
                tx.scroll_to_instant(scroll_key(&self.key), x, 0.0);
                self.probe.revealed();
            }
        }
    }

    pub(super) fn hover_tab(
        &mut self,
        id: TabId,
        hover: DropHover,
        arm: Option<f64>,
        now: f64,
    ) -> bool {
        match (hover, arm) {
            (DropHover::Accepting, Some(_)) if self.spring.as_ref().is_some_and(|s| s.id == id) => {
                false
            }
            (DropHover::Accepting, Some(delay)) => {
                self.spring = Some(Spring {
                    id,
                    delay,
                    since: (now > 0.0).then_some(now),
                });
                true
            }
            // A leave, a refusal, or a drag that may not select it: its spring stops.
            _ => {
                if self.spring.as_ref().is_some_and(|s| s.id == id) {
                    self.spring = None;
                }
                false
            }
        }
    }

    pub(super) fn follow_open(&mut self, site: DropSite, over: bool, tx: &mut Tx) {
        if over {
            self.incoming_from = Some(site);
        } else if self.incoming_from == Some(site) {
            // Its gap's slot, or, for a zone whose gap hasn't built yet, the one kept before.
            let slot = self
                .reorder
                .incoming_slot()
                .or(self.left_landing.map(|l| l.slot));
            self.left_landing = slot.map(|slot| LeftLanding { slot, ticks: 0 });
            self.incoming_from = None;
            self.reorder.set_incoming(None);
            self.drag.set_incoming(None);
            if self.left_landing.is_some() {
                // The ticks that expire it, in a window at rest too.
                tx.wake_after(Duration::ZERO);
            }
        }
    }

    /// The pointer of a drag that opens tabs moved over `site`, in window points.
    pub(super) fn open_moved(&mut self, site: DropSite, point: [f32; 2]) {
        if self.incoming_from == Some(site) {
            self.reorder.set_incoming(Some(point));
            self.drag.set_incoming(Some(point));
        }
    }

    pub(super) fn landing(&self) -> Option<usize> {
        self.reorder
            .incoming_slot()
            .or(self.left_landing.map(|l| l.slot))
    }

    pub fn end_drag(&mut self) {
        self.spring = None;
        self.drop = None;
        self.incoming_from = None;
        self.left_landing = None;
        self.reorder.set_incoming(None);
        self.drag.set_incoming(None);
    }

    pub(super) fn drag_ended(&mut self) {
        self.end_drag();
    }
}

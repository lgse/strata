//! [`BrowserTabsState`]: what the strip keeps between views, and [`BrowserTabsState::tick`],
//! which runs its timer and follows the active tab.

use std::time::Duration;

use rune::Tx;
use rune_kit::shell::TabDragState;

use super::data::{TabId, TabsData, TitleFormat, window_title};
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
    /// When the wait began, on `Tx::clock`.
    since: f64,
}

/// The gap's slot after the zone holding it left. Both real binders report a zone's leave
/// before they deliver its drop (Rune's `runtime/input/drop.rs`, P1's `dnd/window.rs`): W1's
/// `Left` means "left, moved, or dropped". So the gap closes at once, as a cancel or a drag
/// leaving the strip needs, and its slot waits here for the drop that may follow in the same
/// turn: the drop takes it. A zone the drag reaches next keeps it until that zone's own gap has
/// built (a drop in the same turn has no other slot to go by: rune-kit finds a gap's slot as
/// the strip builds), and the ticks after drop it, each asking for the next, so it expires in
/// a window at rest too.
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

/// The strip's state, held by the host per window and reached by accessor: rune-kit's drag
/// gestures ([`BrowserTabsState::drag`], a field so the host can pass `|s| &mut s.tabs.drag`
/// as TabStrip's plain `fn` accessor), the drop hover, the spring, and what the last tick saw.
#[derive(Clone)]
pub struct BrowserTabsState {
    key: String,
    title_format: TitleFormat,
    /// TabStrip's gestures: reorder, tear-out, and a tab coming in from another window
    /// ([`TabDragState::set_incoming`], [`TabDragState::incoming_slot`],
    /// [`TabDragState::hand_off`]).
    pub drag: TabDragState,
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

    /// Sets the drop hover at `site`: for binders, and for scenes that show it. A
    /// [`DropHover::Left`] clears it only if `site` holds it.
    pub fn set_drop_hover(&mut self, site: DropSite, hover: DropHover) {
        match hover {
            DropHover::Left if self.drop.is_some_and(|(s, _)| s == site) => self.drop = None,
            DropHover::Left => {}
            _ => self.drop = Some((site, hover)),
        }
    }

    /// Whether a drag that opens tabs runs anywhere in the window (a sidebar row or a folder-only
    /// drag, which X2's binder tells): the New tab button shows its drop tint while it does.
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

    /// One frame at `now` seconds on the window's clock (the poll's), with the tabs as they are
    /// now. It selects the tab a drag has rested on for the spring delay, emits the
    /// window title when it changed, and scrolls a newly active tab into view, however it
    /// became active (a click, the keyboard, a reopen, a restore).
    pub fn tick(&mut self, now: f64, data: &TabsData<'_>, tx: &mut Tx) -> Polled {
        let mut out = Polled::default();
        if self.drag.incoming_slot().is_some() {
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
                let left = spring.delay - (now - spring.since);
                if left <= 1e-6 {
                    let id = spring.id;
                    self.spring = None;
                    out.intents.push(TabsIntent::Select {
                        id,
                        via: Via::Spring,
                    });
                } else {
                    // Re-armed for the time left at every tick, so a window at rest, which polls
                    // only at the wakes it asked for, still fires on time.
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
                // A strip at rest brings the tab a fade's width clear of the edges in one jump,
                // once it has drawn and its place can be read (U-C7-7).
                Some(id) if self.drag.phase() == rune_kit::shell::TabDragPhase::Idle => {
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

    /// Brings the tab a reveal waits on clear of the edge fades, once it has drawn, in one
    /// jump: Rune's `scroll_into_view_in` stops at the edge, under the fade, and a horizontal
    /// viewport has no scroll margin (U-C7-7). Reads the strip's offset from its markers
    /// (`probe.rs`), so it lands a frame after the selection.
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

    /// A drag reached or left tab `id` at `now` (`Tx::clock`). `arm` is the spring delay when
    /// this drag may select the tab (it isn't active and doesn't open tabs). The same hover
    /// again keeps the spring's start: a binder may report it on each motion. True when it
    /// armed a spring.
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
                    since: now,
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

    /// A drag that opens tabs reached `site` (`Some`), or left it: only the zone holding the
    /// gap closes it, so moving from a tab to the gap beside it never closes it for a frame.
    pub(super) fn follow_open(&mut self, site: DropSite, over: bool, tx: &mut Tx) {
        if over {
            self.incoming_from = Some(site);
        } else if self.incoming_from == Some(site) {
            // Its gap's slot, or, for a zone whose gap hasn't built yet, the one kept before.
            let slot = self
                .drag
                .incoming_slot()
                .or(self.left_landing.map(|l| l.slot));
            self.left_landing = slot.map(|slot| LeftLanding { slot, ticks: 0 });
            self.incoming_from = None;
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
            self.drag.set_incoming(Some(point));
        }
    }

    /// Where a drag that opens tabs lands if it drops now: the open gap, or the one its zone
    /// just left (see [`LeftLanding`]).
    pub(super) fn landing(&self) -> Option<usize> {
        self.drag
            .incoming_slot()
            .or(self.left_landing.map(|l| l.slot))
    }

    /// The drag dropped or was refused: nothing springs, nothing is hovered, the gap closes.
    pub(super) fn drag_ended(&mut self) {
        self.spring = None;
        self.drop = None;
        self.incoming_from = None;
        self.left_landing = None;
        self.drag.set_incoming(None);
    }
}

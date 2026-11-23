// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::Live;

use super::place::{Placement, place};
use crate::collection::element::{RestReport, RestView};
use crate::contract::{EntryId, Rect};

/// How a peek opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// The pointer rested on the folder for the open delay.
    Hover,
    /// The host opened it from a key: it stays until the host closes it, and its card is a polite live region.
    Keyboard,
}

/// The folder a peek shows: what the host lists.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PeekTarget {
    /// The key of the collection the folder's row is in.
    pub collection: Rc<str>,
    /// The folder's entry.
    pub folder: EntryId,
    /// A number new with each peek opened, for matching listing results to the peek that asked for them.
    pub serial: u64,
    /// How it opened.
    pub origin: Origin,
}

/// What a transition did, for the host's listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Nothing the host lists changed.
    None,
    /// A peek opened, or another replaced it: list [`PeekState::target`].
    Opened,
    /// The peek closed: drop its listing.
    Closed,
}

#[derive(Clone, Debug)]
struct Open {
    target: PeekTarget,
    place: Placement,
    /// The folder's row reports the pointer resting on it.
    row: bool,
    sideways_at: f32,
}

#[derive(Clone)]
pub(super) struct Lives {
    pub(super) sideways: Live<f32>,
    pub(super) shown: Live<f32>,
}

impl Default for Lives {
    fn default() -> Self {
        Self {
            sideways: Live::new(0.0),
            shown: Live::new(1.0),
        }
    }
}

impl std::fmt::Debug for Lives {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lives")
            .field("sideways", &self.sideways.get())
            .field("shown", &self.shown.get())
            .finish()
    }
}

/// The folder peek's state, which the host keeps and passes by accessor.
#[derive(Clone, Debug, Default)]
pub struct PeekState {
    enabled: bool,
    open: Option<Open>,
    serial: u64,
    /// The card reports the pointer on it.
    card: bool,
    /// The host has an overlay up (a menu, popover, dialog or palette).
    covered: bool,
    pub(super) lives: Lives,
}

impl PeekState {
    /// Peeking off, nothing open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Turns peeking on or off.
    pub fn set_enabled(&mut self, on: bool) -> bool {
        self.enabled = on;
        !on && self.close()
    }

    /// Whether peeking is on.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Says an overlay the host shows covers the collections, or no longer does.
    pub fn set_covered(&mut self, covered: bool) -> Change {
        self.covered = covered;
        if covered && self.close() {
            Change::Closed
        } else {
            Change::None
        }
    }

    /// Whether the host said an overlay covers the collections.
    pub fn covered(&self) -> bool {
        self.covered
    }

    /// The folder peeked now, if any: the host lists it, and matches results to its `PeekTarget::serial`.
    pub fn target(&self) -> Option<&PeekTarget> {
        self.open.as_ref().map(|o| &o.target)
    }

    /// Where the open card goes.
    pub fn placement(&self) -> Option<Placement> {
        self.open.as_ref().map(|o| o.place)
    }

    /// Whether entry `id` of collection `collection` is the folder a peek is open from: the host's `RowMarks::peek_anchor`.
    pub fn is_anchor(&self, collection: &str, id: EntryId) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| o.target.folder == id && &*o.target.collection == collection)
    }

    /// Closes the open peek.
    pub fn close(&mut self) -> bool {
        self.card = false;
        self.open.take().is_some()
    }

    /// Opens a peek on folder `folder` of collection `collection`, whose row is at `item` in `viewport`).
    pub fn open(
        &mut self,
        collection: impl Into<Rc<str>>,
        folder: EntryId,
        origin: Origin,
        item: Rect,
        viewport: Rect,
    ) -> Change {
        if !self.enabled || self.covered {
            return Change::None;
        }
        let Some(place) = place(item, viewport) else {
            return Change::None;
        };
        self.serial += 1;
        self.lives.shown.set(1.0);
        self.open = Some(Open {
            target: PeekTarget {
                collection: collection.into(),
                folder,
                serial: self.serial,
                origin,
            },
            place,
            row: origin == Origin::Hover,
            sideways_at: self.lives.sideways.get(),
        });
        Change::Opened
    }

    /// The keyboard peek: closes the peek open on this folder, else opens one with `Origin::Keyboard`.
    pub fn toggle(
        &mut self,
        collection: impl Into<Rc<str>>,
        folder: EntryId,
        item: Rect,
        viewport: Rect,
    ) -> Change {
        let collection = collection.into();
        if self.is_anchor(&collection, folder) {
            self.close();
            return Change::Closed;
        }
        self.open(collection, folder, Origin::Keyboard, item, viewport)
    }

    pub(super) fn row_rest(&mut self, r: &RestReport) -> Change {
        if !self.enabled || self.covered {
            return Change::None;
        }
        let mine = self.is_anchor(&r.collection, r.id);
        if mine {
            let open = self.open.as_mut().expect("anchored");
            open.row = r.inside;
            return self.settle();
        }
        if !r.inside || self.card {
            return Change::None;
        }
        self.open(r.collection.clone(), r.id, Origin::Hover, r.row, r.viewport)
    }

    /// The sideways offset the open peek opened at.
    pub(super) fn sideways_at(&self) -> Option<f32> {
        self.open.as_ref().map(|o| o.sideways_at)
    }

    pub(super) fn anchor_drawn(&mut self, x: f32) -> Change {
        let Some(o) = &self.open else {
            return Change::None;
        };
        let placed = o.place.x - super::place::CARD_GAP;
        if (x - placed).abs() > 0.5 {
            self.close();
            Change::Closed
        } else {
            Change::None
        }
    }

    pub(super) fn view(&mut self, v: &RestView) -> Change {
        let left = |folder: &EntryId| !v.visible.is_empty() && !v.visible.contains(folder);
        match &self.open {
            Some(o) if o.target.collection == v.collection && left(&o.target.folder) => {
                self.close();
                Change::Closed
            }
            _ => Change::None,
        }
    }

    /// The card's own rest report.
    pub(super) fn card_rest(&mut self, inside: bool) -> Change {
        self.card = inside && self.open.is_some();
        self.settle()
    }

    /// A hover peek with the pointer on neither its folder nor its card closes.
    fn settle(&mut self) -> Change {
        match &self.open {
            Some(o) if o.target.origin == Origin::Hover && !o.row && !self.card => {
                self.close();
                Change::Closed
            }
            _ => Change::None,
        }
    }
}

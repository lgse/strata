// SPDX-License-Identifier: MIT

//! The drop zones the in-window router hit-tests, by element key, and where a drag lands among
//! them: the innermost zone along the picked element's path that accepts the offer, or that
//! stops the drag by rejecting it.

use std::{collections::HashMap, rc::Rc};

use rune::{Live, Mods, PointerEvent};
use strata_components::contract::dnd::{DropAction, DropActions, DropHover, DropOffer, DropZone};

use crate::App;

const REGISTRY_CAP: usize = 16_384;

pub(super) type PointActionFn =
    Rc<dyn Fn(&App, &DropOffer, Mods, PointerEvent) -> Option<DropAction>>;

/// A drop zone whose action depends on where in it the pointer is, as the Columns strip's.
pub(crate) struct PointZone {
    pub(super) zone: DropZone<App>,
    pub(super) action: PointActionFn,
}

impl PointZone {
    pub(crate) fn new(
        zone: DropZone<App>,
        action: impl Fn(&App, &DropOffer, Mods, PointerEvent) -> Option<DropAction> + 'static,
    ) -> Self {
        Self {
            zone,
            action: Rc::new(action),
        }
    }
}

/// What a drag offers, where it is, and the modifiers held.
pub(super) struct Probe<'a> {
    pub(super) offer: &'a DropOffer,
    pub(super) at: [f32; 2],
    pub(super) mods: Mods,
}

struct TargetEntry {
    zone: DropZone<App>,
    by_point: Option<PointActionFn>,
    rect: Live<[f32; 4]>,
    stamp: u64,
}

impl TargetEntry {
    fn point_of(&self, at: [f32; 2]) -> PointerEvent {
        let [rx, ry, ..] = self.rect.get();
        let mut event = PointerEvent::default();
        event.x = at[0] - rx;
        event.y = at[1] - ry;
        event.window_x = at[0];
        event.window_y = at[1];
        event
    }

    fn choose(&self, app: &App, probe: &Probe<'_>) -> Option<DropAction> {
        match &self.by_point {
            Some(by_point) => by_point(app, probe.offer, probe.mods, self.point_of(probe.at)),
            None => self.zone.choose(app, probe.offer, probe.mods),
        }
    }

    /// Where the drag lands on this zone, `key`: `None` lets it through to the zone beneath.
    fn landing(&self, key: String, app: &App, probe: &Probe<'_>) -> Option<Landing> {
        let chosen = if self.zone.accepts(probe.offer) {
            self.choose(app, probe)
        } else {
            None
        };
        match negotiate_local(chosen, probe.offer.actions) {
            Some(action) => Some(Landing::Accepting { key, action }),
            None => self.zone.rejects().then_some(Landing::Rejecting { key }),
        }
    }
}

pub(super) fn scoped_id(scope: &str, key: &str) -> String {
    format!("{scope}\u{1f}{key}")
}

fn keyed_segments(path: &str) -> impl DoubleEndedIterator<Item = &str> {
    path.split('/')
        .filter(|seg| !seg.is_empty() && !seg.starts_with('#'))
}

#[derive(Default)]
pub(super) struct Registry {
    entries: HashMap<String, TargetEntry>,
    next: u64,
}

impl Registry {
    pub(super) fn put(
        &mut self,
        key: &str,
        zone: DropZone<App>,
        by_point: Option<PointActionFn>,
    ) -> Live<[f32; 4]> {
        self.next += 1;
        let stamp = self.next;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.zone = zone;
            entry.by_point = by_point;
            entry.stamp = stamp;
            return entry.rect.clone();
        }
        let rect = Live::new([0.0; 4]);
        self.entries.insert(
            key.to_owned(),
            TargetEntry {
                zone,
                by_point,
                rect: rect.clone(),
                stamp,
            },
        );
        if self.entries.len() > REGISTRY_CAP {
            self.evict();
        }
        rect
    }

    fn evict(&mut self) {
        let mut stamps: Vec<u64> = self.entries.values().map(|e| e.stamp).collect();
        stamps.sort_unstable();
        let cut = stamps[stamps.len() / 8];
        self.entries.retain(|_, e| e.stamp > cut);
    }

    pub(super) fn zone(&self, key: &str) -> Option<DropZone<App>> {
        self.entries.get(key).map(|e| e.zone.clone())
    }

    /// `at` in the zone `key`'s own coordinates.
    pub(super) fn point(&self, key: &str, at: [f32; 2]) -> PointerEvent {
        self.entries
            .get(key)
            .map_or_else(PointerEvent::default, |e| e.point_of(at))
    }

    /// The registered zones along `path`, innermost first.
    fn along(&self, path: &str) -> Vec<String> {
        let segs: Vec<&str> = keyed_segments(path).collect();
        let mut out = Vec::new();
        for (idx, seg) in segs.iter().enumerate().rev() {
            if self.entries.contains_key(*seg) {
                out.push((*seg).to_owned());
                continue;
            }
            if let Some(id) = segs[..idx]
                .iter()
                .rev()
                .map(|scope| scoped_id(scope, seg))
                .find(|id| self.entries.contains_key(id))
            {
                out.push(id);
            }
        }
        out
    }

    /// Where a drag over the element at `path` lands.
    pub(super) fn resolve(&self, path: Option<&str>, app: &App, probe: &Probe<'_>) -> Landing {
        let Some(path) = path else {
            return Landing::Nowhere;
        };
        self.along(path)
            .into_iter()
            .find_map(|key| self.entries.get(&key)?.landing(key, app, probe))
            .unwrap_or(Landing::Nowhere)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Landing {
    Nowhere,
    Accepting { key: String, action: DropAction },
    Rejecting { key: String },
}

impl Landing {
    pub(super) fn key(&self) -> Option<&str> {
        match self {
            Self::Nowhere => None,
            Self::Accepting { key, .. } | Self::Rejecting { key } => Some(key),
        }
    }

    pub(super) fn hover(&self) -> DropHover {
        match self {
            Self::Nowhere => DropHover::Left,
            Self::Accepting { .. } => DropHover::Accepting,
            Self::Rejecting { .. } => DropHover::Rejecting,
        }
    }
}

fn negotiate_local(chosen: Option<DropAction>, offered: DropActions) -> Option<DropAction> {
    let action = chosen?;
    let allowed = match action {
        DropAction::Copy | DropAction::Ask => {
            offered.is_empty() || offered.contains(DropActions::COPY)
        }
        DropAction::Move => offered.contains(DropActions::MOVE),
        DropAction::Link => offered.contains(DropActions::LINK),
    };
    allowed.then_some(action)
}

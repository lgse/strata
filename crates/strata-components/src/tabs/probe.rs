// SPDX-License-Identifier: MIT

use std::cell::Cell;
use std::rc::Rc;

use rune::{El, Live, el};

use super::data::TabId;

/// What a reveal does now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Reveal {
    /// The active tab hasn't drawn yet.
    Pending,
    /// It shows clear of the fades.
    Clear,
    /// Scroll the viewport to this offset.
    To(f32),
}

/// The markers' rects, and what the strip was built with.
#[derive(Clone)]
pub(super) struct Probe {
    /// The strip root's left edge (an absolute marker at its origin).
    strip: Live<[f32; 4]>,
    /// Its right edge (one pinned to its top right).
    end: Live<[f32; 4]>,
    /// Tab 0's origin.
    first: Live<[f32; 4]>,
    /// The active tab's slot.
    active: Live<[f32; 4]>,
    /// The tab whose slot `active` marks, as last built.
    active_id: Rc<Cell<Option<TabId>>>,
    fade: Rc<Cell<f32>>,
    trailing: Rc<Cell<f32>>,
    wheeled: Rc<Cell<bool>>,
    /// A reveal's jump went out in this frame's tick, so the screen doesn't show it yet.
    revealing: Rc<Cell<bool>>,
    owning: Rc<Cell<bool>>,
    pending: Rc<Cell<Option<f32>>>,
}

impl Default for Probe {
    fn default() -> Self {
        Probe {
            strip: Live::new([0.0; 4]),
            end: Live::new([0.0; 4]),
            first: Live::new([0.0; 4]),
            active: Live::new([0.0; 4]),
            active_id: Rc::new(Cell::new(None)),
            fade: Rc::new(Cell::new(0.0)),
            trailing: Rc::new(Cell::new(0.0)),
            wheeled: Rc::new(Cell::new(false)),
            revealing: Rc::new(Cell::new(false)),
            owning: Rc::new(Cell::new(false)),
            pending: Rc::new(Cell::new(None)),
        }
    }
}

fn marker(key: &str) -> El {
    el().key(key).size(1.0, 1.0).a11y_hidden()
}

impl Probe {
    /// The markers for the strip's root: at its origin and its top right.
    pub(super) fn root_markers(&self) -> [El; 2] {
        [
            marker("probe-origin")
                .absolute(0.0, 0.0)
                .drawn_rect_into(&self.strip),
            marker("probe-end")
                .pin(None, Some(0.0), Some(0.0), None)
                .drawn_rect_into(&self.end),
        ]
    }

    /// The marker for tab 0's slot.
    pub(super) fn first_marker(&self) -> El {
        marker("probe-first")
            .absolute(0.0, 0.0)
            .drawn_rect_into(&self.first)
    }

    pub(super) fn active_marker(&self, id: TabId, version: u64) -> El {
        self.active_id.set(Some(id));
        el().key(format!("probe-active-{version:x}"))
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .a11y_hidden()
            .drawn_rect_into(&self.active)
    }

    /// What the strip was built with: its fade and the space after its viewport.
    pub(super) fn built(&self, fade: f32, trailing: f32, active: Option<TabId>) {
        self.fade.set(fade);
        self.trailing.set(trailing);
        if active.is_none() {
            self.active_id.set(None);
        }
    }

    /// A wheel reached a tab (`wheel.rs`), whichever way it scrolls.
    pub(super) fn note_wheel(&self) {
        self.wheeled.set(true);
    }

    /// Whether a wheel reached a tab since the last call.
    pub(super) fn take_wheel(&self) -> bool {
        self.wheeled.replace(false)
    }

    /// A tick begins a frame: the last frame's jumps, if any, now show.
    pub(super) fn frame_begins(&self) {
        self.revealing.set(false);
        self.pending.set(None);
    }

    /// Whether a tab's wheel handler holds the current gesture.
    pub(super) fn owning(&self) -> bool {
        self.owning.get()
    }

    pub(super) fn set_owning(&self, on: bool) {
        self.owning.set(on);
    }

    pub(super) fn wheel_base(&self) -> f32 {
        self.pending.get().unwrap_or_else(|| self.offset())
    }

    /// The wheel jumped the viewport to `to` in this frame.
    pub(super) fn wheel_jumped(&self, to: f32) {
        self.pending.set(Some(to));
    }

    /// A reveal's jump went out in this frame's tick.
    pub(super) fn revealed(&self) {
        self.revealing.set(true);
    }

    pub(super) fn revealing(&self) -> bool {
        self.revealing.get()
    }

    /// How far the viewport has scrolled, in points.
    pub(super) fn offset(&self) -> f32 {
        self.strip.get()[0] - self.first.get()[0]
    }

    pub(super) fn reveal(&self, id: TabId) -> Reveal {
        let [x, _, w, _] = self.active.get();
        if self.active_id.get() != Some(id) || w <= 0.0 {
            return Reveal::Pending;
        }
        let first = self.first.get()[0];
        let (left, right) = (x - first, x - first + w);
        let offset = self.offset();
        let view = self.end.get()[0] + 1.0 - self.strip.get()[0] - self.trailing.get();
        let margin = self.fade.get();
        let to = if left - margin < offset {
            left - margin
        } else if right + margin > offset + view {
            right + margin - view
        } else {
            return Reveal::Clear;
        };
        Reveal::To(to.max(0.0))
    }
}

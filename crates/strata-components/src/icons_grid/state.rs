//! [`IconsGridState`]: what the grid keeps between frames, in the host's state.

use std::cell::Cell;

use super::caption::CaptionCache;
use super::size::WheelSteps;
use crate::scroll::{Bar, BarMode, ScrollFeed};
use crate::thumbnails::Step;

/// The grid's own state, kept by the host for the life of the view beside the collection's
/// state: measured captions, the size wheel's leftover, and what it last told the thumbnail
/// store.
///
/// Used by: hosts.
#[derive(Debug, Default)]
pub struct IconsGridState {
    pub(super) captions: CaptionCache,
    pub(super) wheel: Cell<WheelSteps>,
    /// The display scale the view last built at (0 until it has).
    pub(super) scale: Cell<f32>,
    /// The step last given to the thumbnail store, with the size it was for.
    pub(super) synced: Cell<Option<Step>>,
    /// The size the grid last reflowed for.
    pub(super) size: Cell<Option<f32>>,
    /// The grid's width as last laid out: a new width builds again, so captions cut to it.
    pub(super) width: Cell<f32>,
    /// The grid's scroll bar (C24) and where the grid stands.
    pub(super) bar: Bar,
    pub(super) feed: ScrollFeed,
}

impl IconsGridState {
    /// Nothing recorded yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The thumbnail step the grid last asked the store for, if it has.
    pub fn synced_step(&self) -> Option<Step> {
        self.synced.get()
    }

    /// Where the grid stands: give it to C24's `AutoscrollPointer::new` for the collection,
    /// with [`super::autoscroll_background`].
    pub fn feed(&self) -> &ScrollFeed {
        &self.feed
    }

    /// When the scroll bar shows: overlay (Strata's, the default) or always.
    pub fn set_bar_mode(&mut self, mode: BarMode) {
        self.bar.set_mode(mode);
    }

    /// The display scale the view last built at, 1 before it has.
    pub fn scale(&self) -> f32 {
        let s = self.scale.get();
        if s > 0.0 { s } else { 1.0 }
    }
}

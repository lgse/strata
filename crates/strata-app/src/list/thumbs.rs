// SPDX-License-Identifier: MIT

//! Thumbnails in List rows, from the window's store (`shell::thumbnails`), as GTK's List rows
//! draw them: 18 px, in place of the row's icon, at full opacity
//! (`request_thumbnail_and_metadata` in `src/ui/browser_modes/list_factory.rs`).
//!
//! One store viewer for the one pane. As in Columns (`crate::columns::thumbs`, whose
//! [`Synced`] rule it shares), the view's `on_visible` only records its range
//! ([`Thumbs::see`]), and the part's `poll` syncs the store once.

use std::{cell::Cell, ops::Range};

use strata_components::{
    collection::element::Slot,
    contract::EntryId,
    thumbnails::{Step, ThumbnailStore, Viewport, icon_slot},
};
use strata_kit::look::Look;

use crate::{
    App,
    columns::thumbs::{Sync, Synced},
    shell,
    slots::Later,
    thumbnails::Viewer,
};

/// A row's thumbnail, in points: GTK's 18 × 18 (`set_thumbnail_or_icon(…, 18, 18)`).
const ROW_THUMBNAIL: f32 = 18.0;

pub(super) struct Thumbs {
    viewer: Viewer,
    synced: Synced,
    /// The display scale the view last built at, which picks the step.
    scale: Cell<f32>,
    step: Option<Step>,
}

impl Thumbs {
    pub(super) fn new(later: Later<App>) -> Self {
        Self {
            viewer: Viewer::new(later, locate),
            synced: Synced::default(),
            scale: Cell::new(1.0),
            step: None,
        }
    }

    /// The view shows `range` now. Nothing syncs until the next poll.
    pub(super) fn see(&mut self, range: Range<usize>) {
        self.synced.range = Some(range);
    }

    /// The scale a build of the view runs at.
    pub(super) fn set_scale(&self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.scale.set(scale);
        }
    }

    fn step(&self, look: &Look) -> Step {
        Step::for_tile(look, ROW_THUMBNAIL, self.scale.get())
    }

    /// The rows' icon: the entry's thumbnail from the window's store, else its icon, at full
    /// opacity, as GTK's List rows draw it.
    pub(super) fn icon(&self, look: &Look) -> Slot<App> {
        icon_slot(shell::thumbnails, self.step(look), ROW_THUMBNAIL).into()
    }
}

/// Where entry `id` of the folder shown is in the browser.
fn locate(app: &App, id: EntryId) -> Option<(usize, usize)> {
    let pane = &app.list.single.pane;
    let depth = pane.depth()?;
    Some((depth, pane.position(depth, id)?))
}

fn parts(app: &mut App) -> (&mut Viewer, &mut ThumbnailStore) {
    (&mut app.list.thumbs.viewer, app.window.thumbnails_mut())
}

/// Syncs the store with the pane when its range, listing or step changed, and asks the pool
/// for what the store wants. True when the store pruned handles a row may still draw, so
/// the view builds again.
pub(super) fn poll(app: &mut App, clock: f64) -> bool {
    let step = app.list.thumbs.step(&app.look);
    let store = app.window.thumbnails_mut();
    let list = &mut app.list;
    let thumbs = &mut list.thumbs;
    thumbs.viewer.fit(store, 1);
    let restep = thumbs.step.replace(step) != Some(step);
    let pruned = match (thumbs.viewer.id(), list.single.pane.listing()) {
        (Some(id), Some(listing)) => {
            let version = listing.version();
            match thumbs.synced.next(listing.location(), version, restep) {
                Sync::View(range) => {
                    store.sync(id, listing, Viewport::new(range, step).listing(version))
                }
                Sync::Step => store.set_step(id, listing, step),
                Sync::Again => store.resync(id, listing, version),
                Sync::Nothing => false,
            }
        }
        _ => false,
    };
    Viewer::pump(app, parts, clock);
    pruned
}

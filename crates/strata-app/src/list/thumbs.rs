// SPDX-License-Identifier: MIT

//! Thumbnails in List rows, from the window's store (`shell::thumbnails`), as GTK's List rows
//! draw them: 18 px, in place of the row's icon, at full opacity
//! (`request_thumbnail_and_metadata` in `src/ui/browser_modes/list_factory.rs`).
//!
//! One store viewer for the one pane. As in Columns (`crate::columns::thumbs`, whose
//! [`Synced`] rule it shares), the view's `on_visible` only records its range
//! ([`Thumbs::see`]), and the part's `poll` syncs the store once.

use std::{cell::Cell, ops::Range, time::Duration};

use rune::Tx;
use strata_components::{
    collection::element::Slot,
    contract::{EntryId, EntrySource},
    thumbnails::{Step, ThumbnailStore, icon_slot},
};
use strata_kit::look::Look;

use crate::{App, columns::thumbs::Synced, shell, slots::Later, thumbnails::Viewer};

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
        self.synced.see(range);
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

    /// Whether List is currently deferring thumbnail handles mid-thumb-drag.
    pub(super) fn is_deferring(&self, dragging: bool) -> bool {
        self.synced.is_deferring(dragging)
    }

    /// The rows' icon: the entry's thumbnail from the window's store, else its icon, at full
    /// opacity, as GTK's List rows draw it.
    pub(super) fn icon(&self, look: &Look) -> Slot<App> {
        icon_slot(shell::thumbnails, self.step(look), ROW_THUMBNAIL)
            .dragging(|app: &App| app.list.thumbs.is_deferring(app.list.table.is_dragging()))
            .badge(
                crate::file_providers::badge_version,
                crate::file_providers::badge_pixels,
            )
            .into()
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
/// for what the store wants. Defers syncing while the scrollbar thumb is actively dragged
/// until release or a brief pause. True when the store pruned handles a row may still draw,
/// so the view builds again.
pub(super) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let step = app.list.thumbs.step(&app.look);
    let dragging = app.list.table.is_dragging();
    let store = app.window.thumbnails_mut();
    let list = &mut app.list;
    let thumbs = &mut list.thumbs;
    thumbs.viewer.fit(store, 1);
    let restep = thumbs.step.replace(step) != Some(step);
    let mut deferred = false;
    let mut pruned = match (thumbs.viewer.id(), list.single.pane.listing()) {
        (Some(id), Some(listing)) => {
            match thumbs
                .synced
                .settled_for_drag(listing.location(), clock, dragging)
            {
                Ok(rebind) => {
                    rebind
                        | thumbs
                            .synced
                            .next(
                                listing.location(),
                                listing.thumb_version(),
                                listing.len(),
                                restep,
                            )
                            .tell(store, id, listing, step)
                }
                Err(left) => {
                    tx.wake_after(Duration::from_secs_f64(left));
                    deferred = true;
                    false
                }
            }
        }
        _ => false,
    };
    if !deferred {
        pruned |= store.take_skipped();
        Viewer::pump(app, parts, clock);
    }
    pruned
}

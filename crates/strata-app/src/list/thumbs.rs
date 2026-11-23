// SPDX-License-Identifier: MIT

use std::{cell::Cell, ops::Range, time::Duration};

use rune::Tx;
use strata_components::{
    collection::element::Slot,
    contract::{EntryId, EntrySource},
    thumbnails::{Step, ThumbnailStore, icon_slot},
};
use strata_kit::look::Look;

use crate::{App, columns::thumbs::Synced, shell, slots::Later, thumbnails::Viewer};

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

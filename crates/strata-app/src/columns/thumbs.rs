// SPDX-License-Identifier: MIT

use std::{cell::Cell, ops::Range, time::Duration};

use rune::Tx;
use strata_components::{
    collection::element::Slot,
    contract::{EntryId, EntrySource},
    thumbnails::{Step, ThumbnailStore, ViewerId, Viewport, icon_slot},
};
use strata_core::model::Location;
use strata_kit::look::Look;

use crate::{App, fills::ScrollSettle, listing::Column, shell, slots::Later, thumbnails::Viewer};

const ROW_THUMBNAIL: f32 = 17.0;

#[derive(Default, Clone)]
pub(crate) struct Synced {
    /// The range its view reported since the last poll, not yet synced.
    pub(crate) range: Option<Range<usize>>,
    location: Option<Location>,
    version: Option<u64>,
    /// Defers thumbnail syncs while the scrollbar thumb is actively dragged.
    settle: ScrollSettle,
    /// Set while a drag deferred syncs, so the flush triggers one rebuild to bind handles.
    was_deferred: bool,
    /// True once a held thumb has settled past the drag debounce and flushed thumbnails.
    settled_while_held: bool,
}

/// What a poll tells the store of one column.
#[derive(Debug, PartialEq)]
pub(crate) enum Sync {
    /// The rows the view shows now.
    View(Range<usize>),
    /// The step changed: the last rows at the new step.
    Step,
    /// The listing changed, or another folder took the column: the last rows of it.
    Again,
    Nothing,
}

impl Sync {
    pub(crate) fn tell(
        self,
        store: &mut ThumbnailStore,
        id: ViewerId,
        listing: &Column,
        version: u64,
        step: Step,
    ) -> bool {
        match self {
            Self::View(range) => {
                store.sync(id, listing, Viewport::new(range, step).listing(version))
            }
            Self::Step => store.set_step(id, listing, step),
            Self::Again => store.resync(id, listing, version),
            Self::Nothing => false,
        }
    }
}

impl Synced {
    /// Records the visible range reported by the view.
    pub(crate) fn see(&mut self, range: Range<usize>) {
        if self.settle.see(&range) {
            self.settled_while_held = false;
        }
        self.range = Some(range);
    }

    /// Whether thumbnail handle allocation should defer for this column while `dragging`.
    pub(crate) fn is_deferring(&self, dragging: bool) -> bool {
        dragging && !self.settled_while_held
    }

    pub(crate) fn settled_for_drag(
        &mut self,
        location: &Location,
        clock: f64,
        dragging: bool,
    ) -> Result<bool, f64> {
        self.settle.visit(location);
        match self.settle.settled_for_drag(clock, dragging) {
            Ok(()) => {
                self.settled_while_held = dragging;
                Ok(std::mem::take(&mut self.was_deferred))
            }
            Err(left) => {
                self.settled_while_held = false;
                self.was_deferred = true;
                Err(left)
            }
        }
    }
    pub(crate) fn next(
        &mut self,
        location: &Location,
        version: u64,
        entries: usize,
        restep: bool,
    ) -> Sync {
        let moved = self.location.as_ref() != Some(location);
        let changed = moved || self.version != Some(version);
        self.location = Some(location.clone());
        self.version = Some(version);
        match self
            .range
            .take()
            .filter(|range| !range.is_empty() || entries == 0)
        {
            Some(range) => Sync::View(range),
            None if restep => Sync::Step,
            None if changed => Sync::Again,
            None => Sync::Nothing,
        }
    }
}

pub(super) struct Thumbs {
    viewer: Viewer,
    columns: Vec<Synced>,
    /// The display scale the view last built at, which picks the step.
    scale: Cell<f32>,
    step: Option<Step>,
}

impl Thumbs {
    pub(super) fn new(later: Later<App>) -> Self {
        Self {
            viewer: Viewer::new(later, locate),
            columns: Vec::new(),
            scale: Cell::new(1.0),
            step: None,
        }
    }

    /// Column `depth`'s view shows `range` now. Nothing syncs until the next poll.
    pub(super) fn see(&mut self, depth: usize, range: Range<usize>) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.see(range);
        }
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

    /// Whether column `depth` is currently deferring thumbnail handles mid-thumb-drag.
    pub(super) fn is_deferring(&self, depth: usize, dragging: bool) -> bool {
        self.columns
            .get(depth)
            .map_or(dragging, |column| column.is_deferring(dragging))
    }

    pub(super) fn icon(&self, look: &Look, depth: usize) -> Slot<App> {
        icon_slot(shell::thumbnails, self.step(look), ROW_THUMBNAIL)
            .dim_files()
            .dragging(move |app: &App| {
                let dragging = app
                    .columns
                    .state
                    .column(depth)
                    .is_some_and(|column| column.is_dragging());
                app.columns.thumbs.is_deferring(depth, dragging)
            })
            .badge(
                crate::file_providers::badge_version,
                crate::file_providers::badge_pixels,
            )
            .into()
    }

    #[cfg(test)]
    pub(super) fn viewers(&self) -> usize {
        (0..)
            .take_while(|index| self.viewer.id_at(*index).is_some())
            .count()
    }
}

/// Where entry `id` is: the open column that lists it, and its position there.
fn locate(app: &App, id: EntryId) -> Option<(usize, usize)> {
    let columns = &app.columns.listings.columns;
    columns
        .iter()
        .enumerate()
        .find_map(|(depth, column)| Some((depth, column.position_of(id)?)))
}

fn parts(app: &mut App) -> (&mut Viewer, &mut ThumbnailStore) {
    (&mut app.columns.thumbs.viewer, app.window.thumbnails_mut())
}

pub(super) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let open = app.columns.open();
    let step = app.columns.thumbs.step(&app.look);
    let store = app.window.thumbnails_mut();
    let state = &app.columns.state;
    let thumbs = &mut app.columns.thumbs;
    thumbs.viewer.fit(store, open);
    thumbs.columns.resize(open, Synced::default());
    let restep = thumbs.step.replace(step) != Some(step);
    let mut pruned = false;
    let mut wake: Option<f64> = None;
    let mut deferred = false;
    let listings = &app.columns.listings;
    for depth in 0..open {
        let Some(listing) = listings.shown(depth) else {
            continue;
        };
        let (Some(id), Some(column)) = (thumbs.viewer.id_at(depth), thumbs.columns.get_mut(depth))
        else {
            continue;
        };
        let dragging = state.column(depth).is_some_and(|c| c.is_dragging());
        if let Err(left) = column.settled_for_drag(listing.location(), clock, dragging) {
            wake = Some(wake.map_or(left, |at| at.min(left)));
            deferred = true;
            continue;
        }
        let version = listings.thumb_version(depth);
        pruned |= column
            .next(listing.location(), version, listing.len(), restep)
            .tell(store, id, listing, version, step);
    }
    if let Some(left) = wake {
        tx.wake_after(Duration::from_secs_f64(left));
    }
    if !deferred {
        pruned |= store.take_skipped();
        Viewer::pump(app, parts, clock);
    }
    pruned
}

#[cfg(test)]
mod tests;

// SPDX-License-Identifier: MIT

//! Thumbnails in Columns rows, from the window's store (`shell::thumbnails`), as GTK's
//! Columns rows draw them: 17 px, in place of the row's icon
//! (`show_entry_icon` in `src/ui/browser/columns/rows/bind.rs`).
//!
//! - **One store viewer per open column**, all in one [`Viewer`]: it grows as columns open
//!   and drops the last ones as they close ([`Viewer::fit`]).
//! - **Once a poll.** A column's `on_visible` only records its range ([`Thumbs::see`]); the
//!   part's `poll` syncs each column that changed once, so a fast scroll in a long column
//!   does not sync every frame, as [`crate::fills`] asks the browser.
//! - **Search results** draw their own way on GTK (a result's local path); they are A10's.

use std::{cell::Cell, ops::Range};

use strata_components::{
    collection::element::Slot,
    contract::{EntryId, EntrySource},
    thumbnails::{Step, ThumbnailStore, ViewerId, Viewport, icon_slot},
};
use strata_core::model::Location;
use strata_kit::look::Look;

use crate::{App, listing::Column, shell, slots::Later, thumbnails::Viewer};

/// A row's thumbnail, in points: GTK's 17 × 17 (`set_thumbnail_or_icon(…, 17, 17)`).
const ROW_THUMBNAIL: f32 = 17.0;

/// What a column last synced with the store: each Columns column's, and the List view's one
/// pane's (`crate::list::thumbs`).
#[derive(Default, Clone)]
pub(crate) struct Synced {
    /// The range its view reported since the last poll, not yet synced.
    pub(crate) range: Option<Range<usize>>,
    /// The folder and the listing's version the store last saw: a sibling opened at the same
    /// depth is another folder, whose listing's version may equal the last one's.
    location: Option<Location>,
    version: Option<u64>,
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
    /// Tells the store what viewer `id` shows of `listing`, at `step`. True when the store
    /// pruned handles a row may still draw.
    pub(crate) fn tell(
        self,
        store: &mut ThumbnailStore,
        id: ViewerId,
        listing: &Column,
        step: Step,
    ) -> bool {
        let version = listing.version();
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
    /// What to tell the store of the column now showing `location` at `version`, and the
    /// column recorded as told.
    ///
    /// A range reported empty while the listing has `entries` is passed over: Rune reports a
    /// folder's rows as none before its list has laid them out, and again once they show
    /// (Rune packet entry 101 is the case where no second report comes, which the
    /// collection's catch-up makes). Synced, it would cancel what the rows had asked for,
    /// renders running included, which GTK's rows keep while they stay bound.
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
            column.range = Some(range);
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

    /// The rows' icon: the entry's thumbnail from the window's store, else its icon, a file's
    /// fainter than a folder's, as GTK's Columns rows draw it (`set_base_opacity` in
    /// `src/ui/browser/columns/rows/bind.rs`).
    pub(super) fn icon(&self, look: &Look) -> Slot<App> {
        icon_slot(shell::thumbnails, self.step(look), ROW_THUMBNAIL)
            .dim_files()
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

/// Fits the viewers to the open columns, syncs each column whose range, listing or step
/// changed, and asks the pool for what the store wants. True when the store pruned handles
/// a row may still draw, so the view builds again.
pub(super) fn poll(app: &mut App, clock: f64) -> bool {
    let open = app.columns.open();
    let step = app.columns.thumbs.step(&app.look);
    let store = app.window.thumbnails_mut();
    let thumbs = &mut app.columns.thumbs;
    thumbs.viewer.fit(store, open);
    thumbs.columns.resize(open, Synced::default());
    let restep = thumbs.step.replace(step) != Some(step);
    let mut pruned = false;
    for (depth, listing) in app.columns.listings.columns.iter().enumerate().take(open) {
        let (Some(id), Some(column)) = (thumbs.viewer.id_at(depth), thumbs.columns.get_mut(depth))
        else {
            continue;
        };
        pruned |= column
            .next(listing.location(), listing.version(), listing.len(), restep)
            .tell(store, id, listing, step);
    }
    Viewer::pump(app, parts, clock);
    pruned
}

#[cfg(test)]
mod tests;

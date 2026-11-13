// SPDX-License-Identifier: MIT

//! Metadata fills for Columns rows, one [`Fills`] for each open column, as GTK's Columns rows
//! ask for them (`request_metadata` in `src/ui/browser/columns/rows/bind.rs`): every row near
//! the viewport whose time, a file's size or a folder's `listable` is unknown, and a folder's
//! item count, or its mode, while sizes show (`columns_show_sizes`), as GTK's registration and
//! then `schedule_refresh`'s test pass them ([`Fills::unfilled`]).
//!
//! - **Not while the column scrolls.** GTK asks for no fill while the column scrolled in the
//!   last 150 ms (`track_listing_scrolling` in `src/ui/browser/columns/append/surface.rs`).
//!   Here a column's range that changed less than [`SETTLE`] ago is not asked for; the part
//!   wakes again once it has settled. GTK asks when a row is bound, so a row bound while the
//!   column scrolls is not asked until it is bound again; here the rows near the settled range
//!   are asked, those included.
//! - **Once a poll**, as [`crate::fills`] asks for every view.

use std::{ops::Range, time::Duration};

use rune::Tx;
use strata_core::model::Location;

use crate::{
    App,
    fills::{Details, Fills},
};

/// How long a column's range stays still before its rows are asked for: GTK's 150 ms.
const SETTLE: f64 = 0.150;

/// One column's fills, and when its range last moved.
struct Column {
    fills: Fills,
    /// The folder the column shows: another folder at this depth starts unscrolled.
    folder: Option<Location>,
    /// The poll's clock when the range was last seen moving, until it settles.
    moved_at: Option<f64>,
    moved: bool,
    /// The range last seen: a first range, or one that only grows as rows arrive, is no
    /// scroll.
    last: Range<usize>,
}

pub(super) struct ColumnFills {
    details: Details,
    columns: Vec<Column>,
}

impl ColumnFills {
    /// Folder counts asked for while sizes show, else none.
    pub(super) fn new(show_sizes: bool) -> Self {
        let details = match show_sizes {
            true => Details::Folders,
            false => Details::Off,
        };
        Self {
            details,
            columns: Vec::new(),
        }
    }

    /// Column `depth`'s view shows `range` now; it is asked for once it settles. A range
    /// reported before the poll that counts the column open is kept.
    pub(super) fn see(&mut self, depth: usize, range: Range<usize>) {
        self.grow(depth + 1);
        let column = &mut self.columns[depth];
        let scrolled = !column.last.is_empty() && column.last.start != range.start;
        column.moved |= scrolled;
        column.last = range.clone();
        column.fills.see(range);
    }

    fn fit(&mut self, open: usize) {
        self.columns.truncate(open);
        self.grow(open);
    }

    fn grow(&mut self, len: usize) {
        let details = self.details;
        while self.columns.len() < len {
            self.columns.push(Column {
                fills: Fills::unfilled(details),
                folder: None,
                moved_at: None,
                moved: false,
                last: 0..0,
            });
        }
    }
}

impl Column {
    /// The column shows `folder` now. A first range in another folder is no scroll, as a new
    /// column in GTK is a new scrolled window.
    fn visit(&mut self, folder: &Location) {
        if self.folder.as_ref() != Some(folder) {
            self.folder = Some(folder.clone());
            self.moved = false;
            self.moved_at = None;
            self.last = 0..0;
        }
    }

    /// Whether the range is still: it last moved at least [`SETTLE`] ago. When it is not, the
    /// time left before it is.
    fn settled(&mut self, clock: f64) -> Result<(), f64> {
        if std::mem::take(&mut self.moved) {
            self.moved_at = Some(clock);
        }
        match self.moved_at {
            Some(at) if clock - at < SETTLE => Err(SETTLE - (clock - at)),
            _ => {
                self.moved_at = None;
                Ok(())
            }
        }
    }
}

/// Asks for each open column's fills once a poll, but for a column still scrolling, which
/// wakes the part once it has settled.
pub(super) fn poll(app: &mut App, clock: f64, tx: &mut Tx) {
    let open = app.columns.open();
    let fills = &mut app.columns.fills;
    fills.fit(open);
    let mut wake: Option<f64> = None;
    for (depth, listing) in app.columns.listings.columns.iter().enumerate().take(open) {
        let Some(column) = fills.columns.get_mut(depth) else {
            continue;
        };
        column.visit(listing.location());
        match column.settled(clock) {
            Ok(()) => column.fills.request(&app.browser, depth, listing),
            Err(left) => wake = Some(wake.map_or(left, |at| at.min(left))),
        }
    }
    if let Some(left) = wake {
        tx.wake_after(Duration::from_secs_f64(left));
    }
}

#[cfg(test)]
mod tests;

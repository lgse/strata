// SPDX-License-Identifier: MIT

use std::{ops::Range, time::Duration};

use rune::Tx;

use crate::{
    App,
    fills::{Details, Fills},
};

pub(super) struct ColumnFills {
    details: Details,
    columns: Vec<Fills>,
}

impl ColumnFills {
    /// Folder counts asked for while sizes show, else none.
    pub(super) fn new(show_sizes: bool) -> Self {
        let mut fills = Self {
            details: Details::Off,
            columns: Vec::new(),
        };
        fills.show_sizes(show_sizes);
        fills
    }

    pub(super) fn show_sizes(&mut self, show: bool) {
        self.details = match show {
            true => Details::Folders,
            false => Details::Off,
        };
        for column in &mut self.columns {
            column.set_details(self.details);
        }
    }

    pub(crate) fn invalidate(&mut self) {
        for column in &mut self.columns {
            column.invalidate();
        }
    }

    pub(crate) fn invalidate_depth(&mut self, depth: usize) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.invalidate();
        }
    }

    pub(super) fn see(&mut self, depth: usize, range: Range<usize>) {
        self.grow(depth + 1);
        self.columns[depth].see(range);
    }

    fn fit(&mut self, open: usize) {
        self.columns.truncate(open);
        self.grow(open);
    }

    fn grow(&mut self, len: usize) {
        let details = self.details;
        while self.columns.len() < len {
            self.columns.push(Fills::unfilled(details));
        }
    }
}

pub(super) fn poll(app: &mut App, clock: f64, tx: &mut Tx) {
    let open = app.columns.open();
    let state = &app.columns.state;
    let fills = &mut app.columns.fills;
    fills.fit(open);
    let mut wake: Option<f64> = None;
    let listings = &app.columns.listings;
    for (depth, listing) in listings.columns.iter().enumerate().take(open) {
        // The rows on screen are a filter's results, not the folder's.
        if listings.hits_at(depth) {
            continue;
        }
        let Some(column) = fills.columns.get_mut(depth) else {
            continue;
        };
        let dragging = state.column(depth).is_some_and(|c| c.is_dragging());
        let poll = crate::fills::ScrollPoll { clock, dragging };
        if let Some(left) = column.request_settled(&app.browser, depth, listing, poll) {
            wake = Some(wake.map_or(left, |at| at.min(left)));
        }
    }
    if let Some(left) = wake {
        tx.wake_after(Duration::from_secs_f64(left));
    }
}

#[cfg(test)]
mod tests;

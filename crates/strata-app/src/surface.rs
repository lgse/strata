// SPDX-License-Identifier: MIT

//! What a listing that shows no entry says instead, in every view, as GTK's directory surface
//! says it (`DirectorySurface::sync` in `src/ui/loading_skeleton/surface.rs`, installed on
//! each Columns column by `with_focus_fallback` in `src/ui/browser/presentation.rs` and on a
//! single pane by `src/ui/browser_modes/pane_shell.rs`): the empty page's text, the browser's
//! message for a folder that could not be read, or, while the folder is read, nothing for a
//! grace period and then "Loading". The list that keeps the keyboard is described by it
//! (`describe_pane_surface` in `src/ui/accessibility.rs`). Shared code, owned by the contract
//! ([`crate::slots`], "Who owns what").
//!
//! A view holds one [`Surface`] for its listings, watches them in its poll
//! ([`Surface::watch`]) and asks it what each says ([`Surface::status`]). Each listing waits
//! out a grace period of its own, as each GTK column has its own loading pages.

use std::time::Duration;

use rune::Tx;
use strata_components::contract::EntrySource;
use strata_core::i18n::tr;

use crate::listing::{Column, Load};

/// How long a read goes unannounced: GTK's `GRACE_PERIOD`
/// (`src/ui/loading_skeleton/delay.rs`), after which its loading page shows. A read that ends
/// sooner is never announced.
pub(crate) const GRACE: Duration = Duration::from_millis(150);

/// One listing's read, while it shows no entry: which folder, when it started on the poll's
/// clock, and whether it has outlasted the grace period.
#[derive(Debug, Default)]
struct Reading {
    of: Option<String>,
    since: f64,
    announced: bool,
}

/// What a view's listings say while they show no entry, and the reads they are waiting out.
pub(crate) struct Surface {
    empty: String,
    loading: String,
    /// One for each listing the view watched last, by its place: a view's listings are its
    /// columns, or its one pane, so this holds no more than the view has columns.
    readings: Vec<Reading>,
}

/// The listing is being read and shows nothing yet.
fn reading(listing: &Column) -> bool {
    listing.len() == 0 && listing.is_loading() && !matches!(listing.load(), Load::Failed(_))
}

impl Reading {
    /// One poll of `listing` at `clock`. True when what it says changed.
    fn watch(&mut self, listing: Option<&Column>, clock: f64, tx: &mut Tx) -> bool {
        let Some(listing) = listing.filter(|listing| reading(listing)) else {
            return std::mem::take(self).announced;
        };
        if self.of.as_deref() != Some(listing.title()) {
            *self = Self {
                of: Some(listing.title().to_owned()),
                since: clock,
                announced: false,
            };
            tx.wake_after(GRACE);
            return false;
        }
        if self.announced {
            return false;
        }
        let left = GRACE.as_secs_f64() - (clock - self.since);
        // A wake lands on its deadline give or take the clock's rounding.
        if left > 1e-6 {
            tx.wake_after(Duration::from_secs_f64(left));
            return false;
        }
        self.announced = true;
        true
    }
}

impl Surface {
    pub(crate) fn translated() -> Self {
        Self {
            empty: tr("This directory is empty"),
            loading: tr("Loading"),
            readings: Vec::new(),
        }
    }

    /// What `listing`, the view's listing at `place`, says in place of entries while it
    /// shows none, and whether the text is drawn as well as heard: nothing is drawn while a
    /// folder is read, as Rune has no loading skeleton.
    pub(crate) fn status<'a>(
        &'a self,
        place: usize,
        listing: &'a Column,
    ) -> Option<(&'a str, bool)> {
        if listing.len() > 0 {
            return None;
        }
        let announced = self.readings.get(place).is_some_and(|read| read.announced);
        match listing.load() {
            Load::Failed(message) => Some((message, true)),
            _ if reading(listing) && announced => Some((&self.loading, false)),
            _ if reading(listing) => Some(("", false)),
            _ => Some((&self.empty, true)),
        }
    }

    /// One poll at `clock`, in seconds, of the view's listings in their places: a read
    /// starts its grace period, and one that outlasts it is announced. A place the view no
    /// longer has is dropped. True when what one says changed.
    pub(crate) fn watch<'a>(
        &mut self,
        listings: impl ExactSizeIterator<Item = Option<&'a Column>>,
        clock: f64,
        tx: &mut Tx,
    ) -> bool {
        self.readings.resize_with(listings.len(), Reading::default);
        let mut changed = false;
        for (read, listing) in self.readings.iter_mut().zip(listings) {
            changed |= read.watch(listing, clock, tx);
        }
        changed
    }
}

#[cfg(test)]
mod tests;

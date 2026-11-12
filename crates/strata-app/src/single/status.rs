// SPDX-License-Identifier: MIT

//! What a pane that shows no entry says instead, as GTK's directory surface says it
//! (`DirectorySurface::sync` in `src/ui/loading_skeleton/surface.rs`): the empty page's text,
//! the browser's message for a folder that could not be read, or, while the folder is read,
//! nothing for a grace period and then "Loading". The list that keeps the keyboard is
//! described by it (`describe_pane_surface` in `src/ui/accessibility.rs`).

use std::time::Duration;

use rune::Tx;
use strata_components::contract::EntrySource;
use strata_core::i18n::tr;

use crate::listing::{Column, Load};

/// How long a read goes unannounced: GTK's `GRACE_PERIOD`
/// (`src/ui/loading_skeleton/delay.rs`), after which its loading page shows. A read that ends
/// sooner is never announced.
pub(super) const GRACE: Duration = Duration::from_millis(150);

/// A read of the shown folder with no entry yet: which folder, when it started on the poll's
/// clock, and whether it has outlasted the grace period.
#[derive(Debug, Default)]
struct Reading {
    of: Option<String>,
    since: f64,
    announced: bool,
}

/// What a pane with no entry says, and the read it is waiting out.
pub(super) struct Surface {
    empty: String,
    loading: String,
    reading: Reading,
}

/// The listing is being read and shows nothing yet.
fn reading(listing: &Column) -> bool {
    listing.len() == 0 && listing.is_loading() && !matches!(listing.load(), Load::Failed(_))
}

impl Surface {
    pub(super) fn translated() -> Self {
        Self {
            empty: tr("This directory is empty"),
            loading: tr("Loading"),
            reading: Reading::default(),
        }
    }

    /// What `listing` says in place of entries while it shows none, and whether the text is
    /// drawn as well as heard: nothing is drawn while a folder is read, as Rune has no loading
    /// skeleton.
    pub(super) fn status<'a>(&'a self, listing: &'a Column) -> Option<(&'a str, bool)> {
        if listing.len() > 0 {
            return None;
        }
        match listing.load() {
            Load::Failed(message) => Some((message, true)),
            _ if reading(listing) && self.reading.announced => Some((&self.loading, false)),
            _ if reading(listing) => Some(("", false)),
            _ => Some((&self.empty, true)),
        }
    }

    /// One poll at `clock`, in seconds: a read starts its grace period, and one that
    /// outlasts it is announced. True when what the pane says changed.
    pub(super) fn watch(&mut self, listing: Option<&Column>, clock: f64, tx: &mut Tx) -> bool {
        let Some(listing) = listing.filter(|listing| reading(listing)) else {
            let was = std::mem::take(&mut self.reading);
            return was.announced;
        };
        if self.reading.of.as_deref() != Some(listing.title()) {
            self.reading = Reading {
                of: Some(listing.title().to_owned()),
                since: clock,
                announced: false,
            };
            tx.wake_after(GRACE);
            return false;
        }
        if self.reading.announced {
            return false;
        }
        let left = GRACE.as_secs_f64() - (clock - self.reading.since);
        // A wake lands on its deadline give or take the clock's rounding.
        if left > 1e-6 {
            tx.wake_after(Duration::from_secs_f64(left));
            return false;
        }
        self.reading.announced = true;
        true
    }
}

#[cfg(test)]
mod tests;

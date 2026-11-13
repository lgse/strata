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
//! ([`Surface::watch`]), asks it what each says ([`Surface::status`]) and draws that with
//! [`drawn`], the one drawing every view shares. Each listing waits
//! out a grace period of its own, as each GTK column has its own loading pages.

use std::{borrow::Cow, time::Duration};

use rune::{El, Tx, col, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::contract::EntrySource;
use strata_core::{i18n::tr, model::Location};

use crate::{
    App,
    listing::{Column, Load},
    shell,
};

/// How long a read goes unannounced: GTK's `GRACE_PERIOD`
/// (`src/ui/loading_skeleton/delay.rs`), after which its loading page shows. A read that ends
/// sooner is never announced.
pub(crate) const GRACE: Duration = Duration::from_millis(150);

/// One listing's read, while it shows no entry: which folder, when it started on the poll's
/// clock, and whether it has outlasted the grace period.
#[derive(Debug, Default)]
struct Reading {
    of: Option<Location>,
    since: f64,
    announced: bool,
}

/// What a view's listings say while they show no entry, and the reads they are waiting out.
pub(crate) struct Surface {
    empty: String,
    loading: String,
    retry: String,
    administrator: String,
    /// One for each listing the view watched last, by its place: a view's listings are its
    /// columns, or its one pane, so this holds no more than the view has columns.
    readings: Vec<Reading>,
}

/// The listing is being read and shows nothing yet.
fn reading(listing: &Column) -> bool {
    listing.len() == 0 && listing.is_loading() && !matches!(listing.load(), Load::Failed(_))
}

impl Reading {
    /// One poll of `listing` at `clock`. True when what it says changed by the clock alone:
    /// a read that outlasted the grace period. A read ends by a change the browser sent (its
    /// rows, its load, its column closed), which runs the view itself, so its end is false:
    /// the poll returns true only for what the view would not run for otherwise.
    fn watch(&mut self, listing: Option<&Column>, clock: f64, tx: &mut Tx) -> bool {
        let Some(listing) = listing.filter(|listing| reading(listing)) else {
            *self = Self::default();
            return false;
        };
        if self.of.as_ref() != Some(listing.location()) {
            *self = Self {
                of: Some(listing.location().clone()),
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
            retry: tr("Retry"),
            administrator: tr("Open as Administrator"),
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
    ) -> Option<(Cow<'a, str>, bool)> {
        if listing.len() > 0 {
            return None;
        }
        let announced = self.readings.get(place).is_some_and(|read| read.announced);
        match listing.load() {
            Load::Failed(message) => Some((Cow::Owned(unreadable(message)), true)),
            _ if reading(listing) && announced => Some((Cow::Borrowed(&self.loading), false)),
            _ if reading(listing) => Some((Cow::Borrowed(""), false)),
            _ => Some((Cow::Borrowed(&self.empty), true)),
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

/// What GTK's surface says of a folder that could not be read, over the browser's `message`.
fn unreadable(message: &str) -> String {
    rust_i18n::t!(
        "Unable to read this directory\n%{message}",
        message = message
    )
    .into_owned()
}

/// What a view's listing draws in place of entries: `said`, from [`Surface::status`], and
/// under it, when `failed` names the browser's column of a folder that could not be read
/// ([`failed`]), Retry and, where the browser offers it, Open as Administrator, as GTK's status
/// page stacks them.
pub(crate) fn drawn(app: &App, surface: &Surface, said: &str, failed: Option<usize>) -> El {
    let look = &app.look;
    let message = text(said.to_owned())
        .font(look.kit.ty.ui)
        .color(look.kit.c.text_muted);
    let page = col()
        .w_full()
        .px(look.space.lg)
        .py(look.space.lg)
        .gap(look.space.sm)
        .child(message);
    let Some(depth) = failed else {
        return page;
    };
    let button = |label: &str, key: String, act: fn(&mut App, usize, &mut Tx)| {
        Button::new(label.to_owned())
            .variant(ButtonVariant::Neutral)
            .on_press_tx(move |app: &mut App, tx: &mut Tx| act(app, depth, tx))
            .build(&look.kit)
            .key(key)
    };
    let page = match administrator(app, depth) {
        Some(_) => page.child(button(
            &surface.administrator,
            format!("surface-administrator-{depth}"),
            open_as_administrator,
        )),
        None => page,
    };
    page.child(button(
        &surface.retry,
        format!("surface-retry-{depth}"),
        retry,
    ))
}

/// `depth`, when `listing`, the browser's column `depth`, could not be read: what
/// [`drawn`] takes to offer Retry.
pub(crate) fn failed(listing: &Column, depth: usize) -> Option<usize> {
    matches!(listing.load(), Load::Failed(_)).then_some(depth)
}

/// The `admin://` form the browser offers for column `depth`'s folder, when only
/// permissions kept the user out.
fn administrator(app: &App, depth: usize) -> Option<Location> {
    app.browser.column_snapshot(depth)?.administrator
}

/// Retry: the column's folder is read again (`install_pane_surface`'s `retry_column`).
fn retry(app: &mut App, depth: usize, tx: &mut Tx) {
    shell::publish(app);
    app.browser.retry_column(depth);
    shell::settle(app, tx);
}

/// Open as Administrator: the folder's `admin://` form opens in its place
/// (`open_as_administrator` in `src/ui/browser/location.rs`).
fn open_as_administrator(app: &mut App, depth: usize, tx: &mut Tx) {
    let Some(administrator) = administrator(app, depth) else {
        return;
    };
    shell::publish(app);
    app.browser.navigate_location(administrator, true);
    shell::settle(app, tx);
}

#[cfg(test)]
mod tests;

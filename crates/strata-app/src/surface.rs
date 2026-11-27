// SPDX-License-Identifier: MIT

use std::{borrow::Cow, time::Duration};

use rune::{El, Tx, col, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::contract::EntrySource;
use strata_core::{i18n::tr, model::Location};

use crate::{
    App, buttons,
    listing::{Column, Load},
    shell,
};

pub(crate) const GRACE: Duration = Duration::from_millis(150);

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
    readings: Vec<Reading>,
}

/// The listing is being read and shows nothing yet.
fn reading(listing: &Column) -> bool {
    listing.len() == 0 && listing.is_loading() && !matches!(listing.load(), Load::Failed(_))
}

impl Reading {
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

fn unreadable(message: &str) -> String {
    rust_i18n::t!(
        "Unable to read this directory\n%{message}",
        message = message
    )
    .into_owned()
}

pub(crate) fn drawn(app: &App, surface: &Surface, said: &str, failed: Option<usize>) -> El {
    let look = &app.look;
    let colour = match failed {
        Some(_) => look.kit.c.text_accent,
        None => look.kit.c.text_muted,
    };
    let message = text(said.to_owned()).font(look.kit.ty.ui).color(colour);
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
        let button = Button::new(label.to_owned())
            .variant(ButtonVariant::Neutral)
            .on_press_tx(move |app: &mut App, tx: &mut Tx| act(app, depth, tx))
            .build(&look.kit);
        buttons::on_return(key, button)
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

pub(crate) fn failed(listing: &Column, depth: usize) -> Option<usize> {
    matches!(listing.load(), Load::Failed(_)).then_some(depth)
}

fn administrator(app: &App, depth: usize) -> Option<Location> {
    app.browser.column_snapshot(depth)?.administrator
}

/// Retry: the column's folder is read again (`install_pane_surface`'s `retry_column`).
fn retry(app: &mut App, depth: usize, tx: &mut Tx) {
    shell::publish(app);
    app.browser.retry_column(depth);
    shell::settle(app, tx);
}

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

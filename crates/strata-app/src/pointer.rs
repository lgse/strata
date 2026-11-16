// SPDX-License-Identifier: MIT

//! Window pointer wiring (A9): middle-click [`Autoscroll`] driving and its [`Shield`] modal
//! overlay, plus leading-chrome marquee origins (the sidebar's blank scroll area and the
//! single-pane header bar).

mod marquee;
#[cfg(test)]
mod tests;

use rune::{El, Tx};
use strata_components::scroll::{Autoscroll, Shield};
use strata_core::i18n::tr;

use crate::{
    App,
    slots::{Frame, Part, Place, Setup},
};

pub(crate) use marquee::leading_origin_drag;

/// Per-tab pointer state: an in-flight marquee begun on leading chrome, and the translated
/// accessible name of the autoscroll shield.
pub(crate) struct Pointer {
    origin: marquee::OriginState,
    label: String,
}

fn active_autoscroll(app: &App) -> &Autoscroll {
    for autoscroll in [
        app.columns.autoscroll(),
        app.icons.autoscroll(),
        app.list.autoscroll(),
        app.filter.pane.autoscroll(),
    ] {
        if autoscroll.active() {
            return autoscroll;
        }
    }
    app.columns.autoscroll()
}

fn stop_all_autoscroll(app: &mut App) {
    app.columns.autoscroll().stop_gone(None);
    app.icons.autoscroll().stop_gone(None);
    app.list.autoscroll().stop_gone(None);
    app.filter.pane.autoscroll().stop_gone(None);
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    if let Some(shield) = Shield::new(active_autoscroll, |app: &App| &app.look)
        .label(app.pointer.label.clone())
        .build(app)
    {
        frame.layer.push(shield);
    }
    None
}

impl Part for Pointer {
    const PLACE: Place<App> = Place::Overlays(build);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            origin: marquee::OriginState::default(),
            label: tr("Autoscroll"),
        }
    }

    fn switched(app: &mut App, tx: &mut Tx) {
        stop_all_autoscroll(app);
        marquee::cancel(app, tx);
    }

    fn tab_hidden(app: &mut App, tx: &mut Tx) {
        stop_all_autoscroll(app);
        marquee::cancel(app, tx);
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let autoscroll = active_autoscroll(app).clone();
        autoscroll.drive(clock, app.look.animations, tx)
    }
}

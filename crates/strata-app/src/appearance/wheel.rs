// SPDX-License-Identifier: MIT

use rune::{Tx, WheelEvent};
use strata_components::icons_grid::WheelSteps;

use crate::{App, icons, shell};

/// What the window's wheel is part way through.
#[derive(Debug, Default)]
pub(crate) struct WheelZoom {
    steps: WheelSteps,
    thumbnails: bool,
}

impl WheelZoom {
    fn reset(&mut self) {
        self.steps.reset();
    }
}

/// What a claimed wheel sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sizes {
    Text,
    Tiles,
}

fn zooms(m: rune::Mods) -> bool {
    m.ctrl && !(m.alt || m.logo)
}

fn claims(app: &App, e: &WheelEvent) -> Option<Sizes> {
    if !zooms(e.mods) {
        return None;
    }
    if !e.mods.shift {
        return Some(Sizes::Text);
    }
    let icons = shell::shown_view(app).is_some_and(|view| (view.shows)("icons"));
    icons.then_some(Sizes::Tiles)
}

/// The window's zoom wheel, asked where the wheel lands: true when it took the delta.
pub(crate) fn heard(app: &mut App, e: WheelEvent, tx: &mut Tx) -> bool {
    let delta = if e.mods.shift && e.dy == 0.0 {
        e.dx
    } else {
        e.dy
    };
    if delta == 0.0 {
        return false;
    }
    let Some(sizes) = claims(app, &e) else {
        app.appearance.wheel.reset();
        return false;
    };
    let zoom = &mut app.appearance.wheel;
    let tiles = sizes == Sizes::Tiles;
    if std::mem::replace(&mut zoom.thumbnails, tiles) != tiles {
        zoom.reset();
    }
    // `WheelSteps` reads `dx` when `dy` is 0, as `delta` above does for Shift.
    let steps = zoom.steps.steps(&e);
    match (steps, sizes) {
        (0, _) => tx.live_only(),
        (steps, Sizes::Text) => super::text_size::step(app, steps),
        (steps, Sizes::Tiles) => icons::size::step(app, steps, tx),
    }
    true
}

pub(crate) fn over_grid(app: &mut App, e: WheelEvent, tx: &mut Tx) -> bool {
    if e.mods.shift {
        return false;
    }
    heard(app, e, tx)
}

// SPDX-License-Identifier: MIT

//! The window's zoom wheel, as GTK's `zoom_controller` and `claims_zoom` have it
//! (`src/ui/window/keyboard/wheel_zoom.rs`):
//!
//! - **Ctrl+wheel** steps the interface's text size, as Ctrl+plus and Ctrl+minus do
//!   ([`super::text_size::step`]).
//! - **Ctrl+Shift+wheel** steps the Icons view's tile size while Icons shows, as Ctrl+Alt+plus
//!   and Ctrl+Alt+minus do (`icons::size::step`). Over the grid C3 claims it first with the
//!   same steps (`WheelSteps`), so the grid's wheel hook leaves it to C3 ([`over_grid`]).
//! - **Only Ctrl**, with Shift for the tiles: with Alt or Super the wheel scrolls, and so does
//!   a Ctrl+Shift+wheel outside Icons.
//! - **Steps** are GTK's: a notch of a wheel is one, 10 points of a precise scroll are one, and
//!   what is left over carries on until the direction, the kind of scroll or what is being
//!   sized changes, or a wheel isn't claimed (C3's `WheelSteps`, the same rule).
//! - **The PDF preview zooms itself**: its pages take Ctrl+wheel before any hook here hears
//!   it, as GTK's wheel passes through the PDF's scroll.
//!
//! # Where it hears the wheel
//!
//! GTK's controller is a capture on the window's root, so no scrolled window takes the wheel
//! first. Rune has no capture phase for the wheel: it hands `on_wheel` to the handlers under
//! the pointer, innermost first, only up to the nearest scroll view, which then scrolls (Rune
//! packet entry 10, case 1, `docs/rune/upstream/rune/10-wheel-routing.md`). Rune's other route,
//! `on_magnify`, takes every Ctrl+wheel off macOS but carries no modifiers and never falls
//! back to the wheel (`viewers/viewport`'s module docs). So [`heard`] is asked from:
//!
//! - the window's root element, for whatever no scroll view holds: the header, the footer, the
//!   tab strip, and a part's empty space;
//! - each column of Columns, through `ColumnsView::collection`;
//! - the List view's rows and the Icons grid, through their wheel hooks (`single::wheeled`).
//!
//! Elsewhere, over a scroll view whose component has no wheel hook (the sidebar, the preview's
//! text, a dialog's page), the wheel scrolls as Rune's does. A Ctrl+wheel within 0.1 s of a
//! plain scroll stays a scroll too (entry 10's `WHEEL_IDLE`). Entry 10 asks for the routing
//! that retires these hooks.

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

/// What `e` sizes, if anything: Ctrl alone for the text, with Shift for the tiles while Icons
/// shows (`claims_zoom`).
fn claims(app: &App, e: &WheelEvent) -> Option<Sizes> {
    let m = e.mods;
    if !m.ctrl || m.alt || m.logo {
        return None;
    }
    if !m.shift {
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

/// [`heard`], from the Icons grid's hook, which C3 asks before its own size wheel: the grid's
/// Ctrl+Shift+wheel is C3's, with the same steps, so it is left to it.
pub(crate) fn over_grid(app: &mut App, e: WheelEvent, tx: &mut Tx) -> bool {
    if e.mods.shift {
        return false;
    }
    heard(app, e, tx)
}

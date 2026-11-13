// SPDX-License-Identifier: MIT

//! View switching and the look: package A8 of the parity plan
//! (`docs/rune/parity/packages/A8.md`).
//!
//! - **The view switch** ([`views`]): Ctrl+1, Ctrl+2 and Ctrl+3 show Columns, Icons and List
//!   while the window is open, and the choice is saved as `browser_mode`.
//! - **The text size** ([`text_size`]): Ctrl+plus, Ctrl+=, Ctrl+minus and Ctrl+0, saved as
//!   `text_size`, which the look follows.
//! - **The Appearance popover** ([`popover`]), from the header's Appearance button.
//!
//! The view switch's and the text size's commands are the window's own (`commands::own`),
//! for the popover's rows and the keys alike; the part holds the popover.

use rune::{El, Tx};

use crate::{
    App,
    commands::{Action, Holds},
    slots::{Frame, Part, Place, Setup},
};

mod popover;
pub(crate) mod text_size;
pub(crate) mod views;

pub(crate) use popover::face;

pub(crate) struct Appearance {
    popover: popover::State,
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    popover::build(app, frame);
    None
}

impl Part for Appearance {
    const PLACE: Place<App> = Place::Overlays(build);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            popover: popover::State::translated(),
        }
    }

    const COMMANDS: &'static [Action<App>] = &[popover::TOGGLE];

    /// GTK's plain popover takes every key while it is open.
    fn keyboard(app: &App) -> Option<Holds> {
        app.appearance.popover.is_open().then_some(Holds::Popover)
    }

    fn poll(app: &mut App, _: f64, tx: &mut Tx) -> bool {
        popover::follow(app, tx)
    }
}

#[cfg(test)]
mod tests;

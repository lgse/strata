// SPDX-License-Identifier: MIT

//! View switching and the look: package A8 of the parity plan
//! (`docs/rune/parity/packages/A8.md`).
//!
//! - **The view switch** ([`views`]): Ctrl+1, Ctrl+2 and Ctrl+3 show Columns, Icons and List
//!   while the window is open, and the choice is saved as `browser_mode`.
//!
//! Nothing of it is drawn yet: the Appearance popover comes with the header's button.

use rune::El;

use crate::{
    App,
    commands::Action,
    slots::{Frame, Part, Place, Setup},
};

mod views;

pub(crate) struct Appearance;

fn nothing(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    None
}

impl Part for Appearance {
    const PLACE: Place<App> = Place::Overlays(nothing);

    fn new(_: &Setup<'_>) -> Self {
        Self
    }

    const COMMANDS: &'static [Action<App>] = views::COMMANDS;
}

#[cfg(test)]
mod tests;

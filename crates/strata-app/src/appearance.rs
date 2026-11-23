// SPDX-License-Identifier: MIT

use rune::{El, Tx};

use crate::{
    App,
    commands::{Action, Holds},
    slots::{Frame, Part, Place, Setup},
};

pub(crate) mod density;
mod popover;
pub(crate) mod text_size;
pub(crate) mod views;
pub(crate) mod wheel;

pub(crate) use popover::{button_icon, face};

pub(crate) struct Appearance {
    popover: popover::State,
    density: density::Sliders,
    wheel: wheel::WheelZoom,
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
            density: density::Sliders::translated(),
            wheel: wheel::WheelZoom::default(),
        }
    }

    const COMMANDS: &'static [Action<App>] = &[popover::TOGGLE];

    fn keyboard(app: &App) -> Option<Holds> {
        app.appearance.popover.is_open().then_some(Holds::Popover)
    }

    fn poll(app: &mut App, _: f64, tx: &mut Tx) -> bool {
        popover::follow(app, tx)
    }
}

#[cfg(test)]
mod tests;

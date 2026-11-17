// SPDX-License-Identifier: MIT

//! A chooser's own window, as the portal daemon ([`super::portal`]) and the browser window's
//! floating destination chooser ([`super::floating`]) open one.

use rune::{AppSpec, Tx, WindowOptions};
use strata_kit::fonts::{FAMILIES, FONTS};
use strata_platform::csd::{self, Frame};

use crate::{App, commands, shell};

/// What a chooser's window is called until its state names it, as the browser window's is:
/// `App::retitle` gives it the request's title once the chooser lists its folder.
const OPENING_TITLE: &str = "Strata";

/// How a chooser's window runs.
pub(crate) struct ChooserWindow {
    pub(crate) size: (f32, f32),
    pub(crate) frame: Frame,
    /// Shown at once, or once the host has placed it.
    pub(crate) visible: bool,
    pub(crate) poll: fn(&mut App, f64, &mut Tx) -> bool,
    pub(crate) on_close: fn(&mut App, &mut Tx),
}

/// The chooser's app, over a state the host builds with `App::new_chooser`.
pub(crate) fn chooser_app(window: ChooserWindow) -> rune::App<App> {
    rune::App::new(AppSpec {
        poll: Some(window.poll),
        on_key: Some(commands::on_key),
        fonts: FONTS,
        families: FAMILIES,
        ..AppSpec::new(OPENING_TITLE, window.size, || unreachable!(), shell::view)
    })
    .window_options(WindowOptions {
        visible: window.visible,
        ..csd::window_options(window.frame, WindowOptions::default())
    })
    .on_modifiers(shell::modifiers)
    .on_close(window.on_close)
}

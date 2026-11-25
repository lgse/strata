// SPDX-License-Identifier: MIT

use rune::{AppHandle, AppSpec, Tx, WindowHandle, WindowOptions};
use strata_kit::fonts::{FAMILIES, FONTS};
use strata_platform::{
    backdrop_blur::BackdropBlur,
    csd::{self, Frame},
};

use crate::{App, commands, menus, shell};

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

fn on_rest(app: &mut App, _: &mut Tx) -> bool {
    crate::preview::clear_idle_caches(app);
    false
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
    .window_options(csd::window_options(
        window.frame,
        WindowOptions {
            visible: window.visible,
            transparent: true,
            ..WindowOptions::default()
        },
    ))
    .on_field_menu(menus::open_field_menu)
    .on_modifiers(shell::modifiers)
    .on_rest(on_rest)
    .on_close(window.on_close)
}

pub(crate) fn bind_backdrop_blur(state: &mut App) -> BackdropBlur {
    let blur = BackdropBlur::new();
    shell::bind_backdrop_blur(state, blur.clone());
    blur
}

/// Attaches the shared native window shims (CSD and Wayland backdrop blur) for a chooser window.
pub(crate) fn attach_window(window: &WindowHandle, handle: &AppHandle<App>, blur: &BackdropBlur) {
    csd::attach(window, handle, shell::csd_of);
    blur.attach(window, handle);
}

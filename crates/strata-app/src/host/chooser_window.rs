// SPDX-License-Identifier: MIT

use rune::{AppHandle, AppSpec, Tx, WindowHandle, WindowOptions};
use strata_kit::fonts::{FAMILIES, FONTS};

use crate::{App, commands, menus, shell};

const OPENING_TITLE: &str = "Strata";

/// How a chooser's window runs.
pub(crate) struct ChooserWindow {
    pub(crate) size: (f32, f32),
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
    .window_options(WindowOptions {
        borderless: true,
        visible: window.visible,
        transparent: true,
        ..WindowOptions::default()
    })
    .on_field_menu(menus::open_field_menu)
    .on_modifiers(shell::modifiers)
    .on_rest(on_rest)
    .on_close(window.on_close)
}

/// Binds the native window handle for a chooser window so backdrop support changes can sync live.
pub(crate) fn attach_window(window: &WindowHandle, handle: &AppHandle<App>) {
    let bound = window.clone();
    handle.update(move |app, _| shell::bind_window_handle(app, bound));
}

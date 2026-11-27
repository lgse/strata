// SPDX-License-Identifier: MIT

//! One shell window: its own Rune app drawing the gallery's shell view.

use rune::{App, AppSpec, Cx, El, Tx, WindowOptions, WindowSpec};
use serde_json::json;
use strata_components::header::{self, WindowAction, WindowButtons, WindowChrome};
use strata_gallery::scenes::x1::shell::{self, Hooks, ShellWindow, Volumes};
use strata_kit::look::Look;

use crate::controller::{self, Controller, windows};
use crate::log;

pub struct Window {
    pub key: String,
    pub shell: ShellWindow,
    pub look: Look,
    pub borderless: bool,
    titled: bool,
}

fn shell_of(w: &mut Window) -> &mut ShellWindow {
    &mut w.shell
}

/// `AppSpec::init` needs a plain function; `open` replaces this state with `App::init`.
fn placeholder() -> Window {
    Window {
        key: String::new(),
        shell: ShellWindow::default(),
        look: Look::fallback(false),
        borderless: true,
        titled: false,
    }
}

const HOOKS: Hooks<Window> = Hooks {
    at: shell_of,
    on_action,
    on_volumes,
};

fn view(w: &Window, cx: &mut Cx<Window>) -> El {
    let mut shown = w.shell.clone();
    let chrome = WindowChrome::from_cx(cx, w.shell.chrome.active);
    shown.chrome = chrome;
    let borderless = w.borderless;
    shell::view(
        &shown,
        &w.look,
        cx,
        HOOKS,
        |bar| bar,
        |window| header::resize_bands(window, borderless, &chrome),
    )
}

/// Sets the window's title once: `AppSpec` takes only a static one.
fn poll(w: &mut Window, _: f64, tx: &mut Tx) -> bool {
    if w.titled {
        return false;
    }
    w.titled = true;
    tx.set_title(format!("{} — Strata shell", w.shell.location));
    true
}

/// The header's window buttons, routed natively through Rune.
fn on_action(w: &mut Window, action: WindowAction, tx: &mut Tx) {
    match action {
        WindowAction::Minimize => tx.minimize(),
        WindowAction::ToggleMaximize => tx.toggle_maximized(),
        WindowAction::Close => tx.close_window(),
    }
    log(
        "action",
        json!({ "window": w.key, "action": format!("{action:?}") }),
    );
}

fn on_volumes(w: &mut Window) {
    w.shell.volumes = Volumes::Loading;
    if let Some(c) = controller::controller() {
        c.update(|c: &mut Controller, _| c.list_volumes());
    }
}

fn on_close(w: &mut Window, _: &mut Tx) {
    let empty = {
        let mut open = windows();
        open.retain(|(key, _)| *key != w.key);
        open.is_empty()
    };
    if empty {
        controller::last_window_closed();
    }
}

pub fn open(c: &mut Controller, location: String, token: Option<&str>) {
    c.launches += 1;
    let key = format!("window-{}", c.launches);
    let buttons = if c.all_buttons {
        WindowButtons::ALL
    } else {
        WindowButtons::default()
    };
    let state = Window {
        key: key.clone(),
        shell: ShellWindow {
            location: location.clone(),
            buttons,
            desktop: c.desktop,
            overrides: c.status.clone(),
            volumes: c.mounts.clone(),
            ..ShellWindow::default()
        },
        look: c.look.clone(),
        borderless: c.borderless,
        titled: false,
    };
    log(
        "window",
        json!({ "key": key, "location": location, "token": token.is_some() }),
    );
    let app = App::new(AppSpec {
        poll: Some(poll),
        fonts: strata_kit::fonts::FONTS,
        families: strata_kit::fonts::FAMILIES,
        ..AppSpec::new("Strata shell", (1200.0, 760.0), placeholder, view)
    })
    .init(move || state)
    .windowless()
    .args(Vec::new())
    .window_options(WindowOptions {
        borderless: c.borderless,
        ..WindowOptions::default()
    })
    .on_close(on_close);
    let handle = app.handle();
    // Before the window starts, so it is created with the token.
    match token {
        Some(token) => handle.activate(token),
        None => handle.open(),
    }
    let ready = key.clone();
    rune::open_window(WindowSpec::from_app(
        key.clone(),
        app.on_ready(move |_| log("ready", json!({ "key": ready }))),
    ));
    windows().push((key, handle));
}

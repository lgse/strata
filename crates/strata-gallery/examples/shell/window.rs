//! One shell window: its own Rune app with a `Csd`, drawing the gallery's shell view.

use rune::{App, AppSpec, Cx, El, Tx, WindowHandle, WindowOptions, WindowSpec};
use serde_json::json;
use strata_components::header::{WindowAction, WindowButtons, WindowChrome};
use strata_gallery::scenes::x1::shell::{self, Hooks, ShellWindow, Volumes};
use strata_kit::look::Look;
use strata_platform::csd::{self, Backend, Csd, Notice, Outcome};

use crate::controller::{self, Controller, windows};
use crate::log;

pub struct Window {
    pub key: String,
    pub shell: ShellWindow,
    pub look: Look,
    pub csd: Csd,
    titled: bool,
}

fn csd_of(w: &mut Window) -> &mut Csd {
    &mut w.csd
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
        csd: Csd::new(csd::Policy::Auto.frame(), Backend::Headless),
        titled: false,
    }
}

/// The header's data from what `csd` knows, and the window's focus.
fn chrome(csd: &Csd, active: bool) -> WindowChrome {
    let state = csd.state();
    let caps = csd.capabilities();
    WindowChrome {
        maximized: state.maximized,
        fullscreen: state.fullscreen,
        can_minimize: caps.minimize,
        can_maximize: caps.maximize,
        active,
    }
}

const HOOKS: Hooks<Window> = Hooks {
    at: shell_of,
    on_action,
    on_volumes,
};

fn view(w: &Window, cx: &mut Cx<Window>) -> El {
    let mut shown = w.shell.clone();
    shown.chrome = chrome(&w.csd, w.shell.chrome.active);
    shell::view(
        &shown,
        &w.look,
        cx,
        HOOKS,
        |bar| csd::header(bar, &w.csd, csd_of),
        |window| csd::decorate(window, &w.csd, csd_of),
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

/// The header's window buttons, routed to the window manager through `csd`.
fn on_action(w: &mut Window, action: WindowAction, tx: &mut Tx) {
    let outcome = match action {
        WindowAction::Minimize => w.csd.minimize(tx),
        WindowAction::ToggleMaximize => w.csd.toggle_maximized(tx),
        WindowAction::Close => w.csd.close(tx),
    };
    log(
        "action",
        json!({ "window": w.key, "action": format!("{action:?}"), "outcome": format!("{outcome:?}") }),
    );
}

fn on_volumes(w: &mut Window) {
    w.shell.volumes = Volumes::Loading;
    if let Some(c) = controller::controller() {
        c.update(|c: &mut Controller, _| c.list_volumes());
    }
}

fn observe(csd: &Csd, notice: Notice<'_>) {
    let fields = match notice {
        Notice::Attached | Notice::Report => json!({
            "backend": format!("{:?}", csd.backend()),
            "state": format!("{:?}", csd.state()),
            "capabilities": format!("{:?}", csd.capabilities()),
        }),
        Notice::Outcome(Outcome::Sent { .. }) => return,
        Notice::Outcome(outcome) => json!({ "outcome": format!("{outcome:?}") }),
        Notice::Error(e) => json!({ "error": e }),
    };
    log("csd", fields);
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

/// Opens a window on `location`, created with `token` (so the compositor lets it take focus)
/// or opened without one.
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
        csd: Csd::new(c.frame, Backend::from_env()).observe(observe),
        titled: false,
    };
    log(
        "window",
        json!({ "key": key, "location": location, "token": token.is_some() }),
    );
    // Strata's default window size (`app/window`: 1200×760). Each window is its own app,
    // started windowless (which implies keep_running) and shown by `activate` or `open`, as
    // strata-platform's `single-instance-demo` does: closing it runs `on_close`, which drops it
    // from `WINDOWS` and quits the controller after the last one. Text draws in the bundled
    // JetBrains Mono, in both slots, as Strata's does (K6).
    let app = App::new(AppSpec {
        poll: Some(poll),
        fonts: strata_kit::fonts::FONTS,
        families: strata_kit::fonts::FAMILIES,
        ..AppSpec::new("Strata shell", (1200.0, 760.0), placeholder, view)
    })
    .init(move || state)
    .windowless()
    .args(Vec::new())
    .window_options(csd::window_options(c.frame, WindowOptions::default()))
    .on_close(on_close);
    let handle = app.handle();
    let attach = handle.clone();
    let app = app.on_window(move |w: &WindowHandle| csd::attach(w, &attach, csd_of));
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

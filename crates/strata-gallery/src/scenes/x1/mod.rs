// SPDX-License-Identifier: MIT

pub mod reload;
pub mod shell;

use rune::{Cx, El, Key, KeyEvent, Mods, Tx, col, text};
use strata_components::header::{WindowAction, WindowButtons, WindowChrome};
use strata_kit::overrides::Overrides;
use strata_kit::theme::{self, Theme, bundled_theme};

use crate::{Gallery, Scene};
use reload::{Change, Debounce};
use shell::{Hooks, ShellWindow, Volume, Volumes};

pub const FILE: &str = r##"# The gallery's fake overrides.toml: the page never reads a real file.
frame.inset = 3

[light]
color.border_base = "#d20f39"
color.bg_deep = "#e6e9ef"

[dark]
color.border_base = "#7aa2f7"
color.bg_deep = "#16161e"
"##;

pub const FILE_BROKEN: &str = r##"# The gallery's fake overrides.toml: the page never reads a real file.
frame.inset = 4
color.text_base = "nope"

[light]
color.border_base = "#d20f39"
color.bg_deep = "#e6e9ef"

[dark]
color.border_base = "#7aa2f7"
color.bg_deep = "#16161e"
"##;

/// How long the fake volume listing takes, in seconds.
pub const VOLUMES_AFTER: f64 = 0.2;

/// What the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// One shell window.
    #[default]
    Shell,
    /// Four headers, one per window state that draws differently.
    HeaderStates,
}

#[derive(Default)]
pub struct Demo {
    pub mode: Mode,
    pub window: ShellWindow,
    /// The bundled theme's id.
    pub theme: &'static str,
    /// The fake overrides file: `None` when deleted.
    pub file: Option<&'static str>,
    pub overrides: Overrides,
    pub debounce: Debounce,
    /// A file edit the next poll times.
    pub edited: bool,
    /// The volumes were asked for; the next poll times the fake listing.
    pub volumes_asked: bool,
    /// The fake GLib bridge didn't start, with why: a listing fails at once.
    pub no_bridge: Option<&'static str>,
    pub volumes_due: Option<f64>,
}

fn theme_for(id: &str) -> &'static Theme {
    bundled_theme(id).unwrap_or_else(|| panic!("{id} is bundled"))
}

fn fake_volumes() -> Vec<Volume> {
    vec![
        Volume {
            name: "System".into(),
            detail: "/".into(),
        },
        Volume {
            name: "Photos 2024".into(),
            detail: "/run/media/ada/Photos 2024".into(),
        },
        Volume {
            name: "projects on nas.local".into(),
            detail: "smb://nas.local/projects".into(),
        },
    ]
}

/// The X11 window manager's report for a window that can do everything.
fn x11(maximized: bool) -> WindowChrome {
    WindowChrome {
        maximized: Some(maximized),
        fullscreen: Some(false),
        can_minimize: Some(true),
        can_maximize: Some(true),
        active: true,
        ..WindowChrome::default()
    }
}

fn open(g: &mut Gallery, dark: bool, mode: Mode) {
    g.open("x1", dark);
    let id = if dark { "tokyo-night" } else { "omarchy-light" };
    let file = Some(FILE);
    let (overrides, present) = reload::read(file.map(str::as_bytes));
    let window = ShellWindow {
        location: "~/Projects".into(),
        buttons: WindowButtons::ALL,
        chrome: x11(false),
        overrides: reload::status(&overrides, present, 0),
        ..ShellWindow::default()
    };
    g.look = reload::build(theme_for(id), &window.desktop, &overrides);
    g.dark = g.look.kit.dark;
    g.x1 = Demo {
        mode,
        window,
        theme: id,
        file,
        overrides,
        ..Demo::default()
    };
}

fn maximized(g: &mut Gallery, dark: bool) {
    open(g, dark, Mode::Shell);
    g.x1.window.chrome = x11(true);
}

fn volumes(g: &mut Gallery, dark: bool, volumes: Volumes) {
    open(g, dark, Mode::Shell);
    g.x1.window.volumes_open = true;
    g.x1.window.volumes = volumes;
}

/// Golden scenes: each gets an accessibility and a PNG golden.
pub const SCENES: &[Scene] = &[
    ("x1-header-states-light", |g| {
        open(g, false, Mode::HeaderStates)
    }),
    ("x1-header-states-dark", |g| {
        open(g, true, Mode::HeaderStates)
    }),
    ("x1-shell-light", |g| open(g, false, Mode::Shell)),
    ("x1-shell-dark", |g| open(g, true, Mode::Shell)),
    ("x1-shell-maximized-light", |g| maximized(g, false)),
    ("x1-shell-maximized-dark", |g| maximized(g, true)),
    ("x1-volumes-light", |g| {
        volumes(g, false, Volumes::Listed(fake_volumes()))
    }),
    ("x1-volumes-dark", |g| {
        volumes(g, true, Volumes::Listed(fake_volumes()))
    }),
];

pub const REVIEW: &[Scene] = &[
    ("x1-volumes-loading-light", |g| {
        volumes(g, false, Volumes::Loading)
    }),
    ("x1-volumes-loading-dark", |g| {
        volumes(g, true, Volumes::Loading)
    }),
    ("x1-volumes-empty-light", |g| {
        volumes(g, false, Volumes::Listed(Vec::new()))
    }),
    ("x1-volumes-empty-dark", |g| {
        volumes(g, true, Volumes::Listed(Vec::new()))
    }),
    ("x1-volumes-failed-light", |g| volumes(g, false, failed())),
    ("x1-volumes-failed-dark", |g| volumes(g, true, failed())),
    ("x1-shell-no-bridge-light", |g| no_bridge(g, false)),
    ("x1-shell-no-bridge-dark", |g| no_bridge(g, true)),
];

/// A shell whose GLib bridge didn't start.
fn no_bridge(g: &mut Gallery, dark: bool) {
    open(g, dark, Mode::Shell);
    g.x1.no_bridge = Some("the GLib thread couldn't start");
}

fn failed() -> Volumes {
    Volumes::Failed("No GLib bridge: the GLib thread couldn't start".into())
}

fn window(g: &mut Gallery) -> &mut ShellWindow {
    &mut g.x1.window
}

/// Stands in for the window manager: Maximize flips the state as an X11 report would.
fn on_action(g: &mut Gallery, action: WindowAction, _: &mut Tx) {
    let chrome = &mut g.x1.window.chrome;
    if action == WindowAction::ToggleMaximize {
        chrome.maximized = chrome.maximized.map(|m| !m);
    }
}

fn on_volumes(g: &mut Gallery) {
    let d = &mut g.x1;
    match d.no_bridge {
        Some(why) => d.window.volumes = Volumes::Failed(format!("No GLib bridge: {why}")),
        None => {
            d.window.volumes = Volumes::Loading;
            d.volumes_asked = true;
        }
    }
}

const HOOKS: Hooks<Gallery> = Hooks {
    at: window,
    on_action,
    on_volumes,
};

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    match g.x1.mode {
        Mode::Shell => shell::view(
            &g.x1.window,
            g.look(),
            cx,
            HOOKS,
            |bar| bar,
            |window| window,
        ),
        Mode::HeaderStates => header_states(g, cx),
    }
}

/// The header states that draw differently, each under a caption, each with its own keys.
fn header_states(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    let k = &look.kit;
    let all = WindowButtons::ALL;
    let states = [
        (
            "fresh",
            "A fresh profile",
            WindowButtons::default(),
            x11(false),
        ),
        ("x11", "X11, every button", all, x11(false)),
        ("maximized", "X11, maximised", all, x11(true)),
        (
            "wayland",
            "Wayland, every button turned on",
            all,
            WindowChrome {
                maximized: None,
                fullscreen: None,
                can_minimize: Some(false),
                can_maximize: None,
                active: true,
                ..WindowChrome::default()
            },
        ),
    ];
    let reduced = !look.animations || cx.system_reduced_motion();
    cx.reduced_motion(reduced);
    col()
        .key("x1-header-states")
        .w_full()
        .h_full()
        .fill(k.c.bg_base)
        .gap(look.space.md)
        .py(look.space.lg)
        .children(states.into_iter().map(|(id, caption, buttons, chrome)| {
            let w = ShellWindow {
                location: "~/Projects".into(),
                buttons,
                chrome,
                ..ShellWindow::default()
            };
            col()
                .key(format!("x1-state-{id}"))
                .w_full()
                .gap(look.space.xs)
                .child(
                    text(caption)
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .px(look.space.lg),
                )
                .child(shell::bar(&w, look, &format!("x1-{id}"), false, HOOKS))
        }))
}

/// Keys no focused element took while this page is open (the table in the module docs).
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let Key::Text(t) = &key.key else {
        return false;
    };
    if key.mods != Mods::default() {
        return false;
    }
    let d = &mut g.x1;
    let buttons = &mut d.window.buttons;
    match t.as_str() {
        "1" => buttons.minimize = !buttons.minimize,
        "2" => buttons.maximize = !buttons.maximize,
        "3" => buttons.close = !buttons.close,
        "e" => {
            d.file = Some(if d.file == Some(FILE) {
                FILE_BROKEN
            } else {
                FILE
            });
            d.edited = true;
        }
        "d" => {
            d.file = None;
            d.edited = true;
        }
        "t" => d.edited = true,
        "n" => {
            d.file = Some("");
            d.edited = true;
        }
        "s" => {
            let desktop = &mut d.window.desktop;
            desktop.text_scale = if desktop.text_scale == 1.0 { 1.25 } else { 1.0 };
            relook(g, tx);
        }
        "a" => {
            d.window.desktop.animations = !d.window.desktop.animations;
            relook(g, tx);
        }
        _ => return false,
    }
    true
}

fn relook(g: &mut Gallery, tx: &mut Tx) {
    let d = &g.x1;
    let next = reload::build(theme_for(d.theme), &d.window.desktop, &d.overrides);
    theme::switch(&mut g.look, next, tx);
}

/// Times the fake file's reads and the fake volume listing on the page's clock.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    let d = &mut g.x1;
    if std::mem::take(&mut d.edited) {
        let due = d.debounce.changed(t);
        tx.wake_after(std::time::Duration::from_secs_f64((due - t).max(0.0)));
    }
    if d.debounce.take_due(t) {
        let (overrides, present) = reload::read(d.file.map(str::as_bytes));
        let status = &mut d.window.overrides;
        match Change::of(&d.overrides, status.present, &overrides, present) {
            Change::Nothing => {}
            Change::Presence => {
                status.present = present;
                changed = true;
            }
            Change::Look => {
                *status = reload::status(&overrides, present, status.reloads + 1);
                d.overrides = overrides;
                relook(g, tx);
                changed = true;
            }
        }
    }
    let d = &mut g.x1;
    if std::mem::take(&mut d.volumes_asked) {
        d.volumes_due = Some(t + VOLUMES_AFTER);
        tx.wake_after(std::time::Duration::from_secs_f64(VOLUMES_AFTER));
    }
    if d.volumes_due.is_some_and(|due| t + 1e-6 >= due) {
        d.volumes_due = None;
        d.window.volumes = Volumes::Listed(fake_volumes());
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests;

// SPDX-License-Identifier: MIT

//! The window's zoom wheel (`appearance::wheel`): Ctrl+wheel over a view's entries steps the
//! text size in each view and over the window's header, where no scroll view holds it; with
//! Alt it does not. Ctrl+Shift+wheel steps the Icons tiles a step a notch, once, and sizes
//! nothing in List.

use rune::{Mods, Tx, WheelEvent};
use strata_desktop::settings::TextSize;

use super::{ICONS, LIST, MODULE, ctrl};
use crate::{
    host::offscreen::Offscreen,
    shell,
    test_support::{
        fixture,
        view::{Shape, all, hosted_with, idle, open, text, wheel_held},
    },
};

const COLUMNS: Shape = Shape {
    described: "Columns view",
    list: "list-box",
    entry: "list-box-option",
};

/// One notch of a mouse wheel turned up, in Rune's points.
const NOTCH: f32 = 40.0;

fn text_size(host: &Offscreen) -> TextSize {
    shell::preferences(host.app()).text_size
}

fn with(more: Mods) -> Mods {
    Mods {
        shift: more.shift,
        alt: more.alt,
        ..ctrl()
    }
}

/// The tile size one step up from the one shown, which is shown again after.
fn step_and_back(host: &mut Offscreen) -> i32 {
    host.runtime()
        .update(|app, tx: &mut Tx| crate::icons::size::step(app, 1, tx));
    idle(host);
    let up = crate::icons::size::shown(host.app());
    host.runtime()
        .update(|app, tx: &mut Tx| crate::icons::size::step(app, -1, tx));
    idle(host);
    up
}

/// One notch over the window's Settings button, which no scroll view holds.
fn wheel_over_header(host: &mut Offscreen, mods: Mods) {
    let tree = host.tree();
    let settings = all(&tree, &|node| {
        node["role"] == "button" && text(node, "name") == "Settings"
    });
    let bounds = &settings.first().expect("the Settings button")["bounds"];
    let at = |index: usize| bounds[index].as_f64().expect("a bound");
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let (x, y) = ((at(0) + at(2) / 2.0) as f32, (at(1) + at(3) / 2.0) as f32);
    host.runtime()
        .wheel_event(WheelEvent::new(0.0, NOTCH).at(x, y).mods(mods));
    idle(host);
}

/// In the view `mode` shows, two notches of Ctrl+wheel over an entry make the text two pixels
/// larger, one with Ctrl+Alt changes nothing, and one over the header one pixel more.
fn ctrl_wheel_steps_the_text_size(mode: &'static str, shape: Shape, name: &str) {
    hosted_with(
        MODULE,
        mode,
        "text_size = 20\n".to_owned(),
        name,
        |mut host| {
            let folder = fixture();
            open(&mut host, shape, folder.path());

            wheel_held(&mut host, NOTCH, ctrl());
            wheel_held(&mut host, NOTCH, ctrl());
            assert_eq!(
                text_size(&host),
                TextSize::new(22),
                "{mode}: two notches of Ctrl+wheel"
            );

            let alt = Mods {
                alt: true,
                ..Mods::default()
            };
            wheel_held(&mut host, NOTCH, with(alt));
            assert_eq!(
                text_size(&host),
                TextSize::new(22),
                "{mode}: Ctrl+Alt+wheel"
            );

            wheel_over_header(&mut host, ctrl());
            assert_eq!(
                text_size(&host),
                TextSize::new(23),
                "{mode}: over the header"
            );
        },
    );
}

#[test]
fn ctrl_wheel_steps_the_text_size_in_columns() {
    ctrl_wheel_steps_the_text_size(
        "columns",
        COLUMNS,
        "wheel::ctrl_wheel_steps_the_text_size_in_columns",
    );
}

#[test]
fn ctrl_wheel_steps_the_text_size_in_list() {
    ctrl_wheel_steps_the_text_size(
        "list",
        LIST,
        "wheel::ctrl_wheel_steps_the_text_size_in_list",
    );
}

#[test]
fn ctrl_wheel_steps_the_text_size_in_icons() {
    ctrl_wheel_steps_the_text_size(
        "icons",
        ICONS,
        "wheel::ctrl_wheel_steps_the_text_size_in_icons",
    );
}

fn ctrl_shift() -> Mods {
    with(Mods {
        shift: true,
        ..Mods::default()
    })
}

/// Ctrl+Shift+wheel steps the Icons tiles one step a notch, over the grid (C3's) and over the
/// header (the window's), and leaves the text.
#[test]
fn ctrl_shift_wheel_steps_the_icons_tiles_once_a_notch() {
    hosted_with(
        MODULE,
        "icons",
        String::new(),
        "wheel::ctrl_shift_wheel_steps_the_icons_tiles_once_a_notch",
        |mut host| {
            let folder = fixture();
            open(&mut host, ICONS, folder.path());
            let text = text_size(&host);
            let tiles = crate::icons::size::shown(host.app());
            let one_step = step_and_back(&mut host);
            assert!(one_step > tiles, "a step makes the tiles larger");

            wheel_held(&mut host, NOTCH, ctrl_shift());
            let shown = crate::icons::size::shown(host.app());
            assert_eq!(shown, one_step, "a notch over the grid");
            wheel_over_header(&mut host, ctrl_shift());
            let shown = crate::icons::size::shown(host.app());
            assert!(shown > one_step, "a notch over the header");
            assert_eq!(text_size(&host), text, "the text");
        },
    );
}

/// In List, Ctrl+Shift+wheel sizes nothing: there are no tiles, and it is no text wheel.
#[test]
fn ctrl_shift_wheel_sizes_nothing_in_list() {
    hosted_with(
        MODULE,
        "list",
        String::new(),
        "wheel::ctrl_shift_wheel_sizes_nothing_in_list",
        |mut host| {
            let folder = fixture();
            open(&mut host, LIST, folder.path());
            let text = text_size(&host);
            wheel_held(&mut host, NOTCH, ctrl_shift());
            assert_eq!(text_size(&host), text, "Ctrl+Shift+wheel in List");
        },
    );
}

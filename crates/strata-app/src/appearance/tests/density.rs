// SPDX-License-Identifier: MIT

//! The Frame and Content densities: the look follows the saved ones from the start, the
//! popover's sliders show them as GTK's do, and each change a slider makes is saved and drawn
//! at once, by its keys and by its Compact and Airy toggles, Return included.

use rune::{Key, KeyEvent, Mods, Tx};
use serde_json::Value;

use super::{MODULE, settings_file};
use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, send_key,
        view::{self, hosted_with},
    },
};

const SAVED: &str = "frame_density = \"airy\"\n\
                     browser_density = \"custom\"\n\
                     browser_density_custom_px = 6\n";

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn node(host: &Offscreen, key: &str) -> Value {
    let tree = host.tree();
    let found = view::all(&tree, &|node| node["key"] == key);
    found
        .first()
        .map(|node| (*node).clone())
        .unwrap_or_else(|| panic!("{key} in the tree"))
}

fn checked(node: &Value) -> bool {
    node["states"]
        .as_array()
        .is_some_and(|states| states.iter().any(|seen| seen == "checked"))
}

/// The slider keyed `key`: its name and value text, and whether its Compact and Airy toggles
/// are pressed.
fn slider(host: &Offscreen, key: &str) -> (String, String, [bool; 2]) {
    let slider = node(host, key);
    let snap = |i: usize| checked(&node(host, &format!("{key}-snap-{i}")));
    (
        view::text(&slider, "name"),
        view::text(&slider, "value"),
        [snap(0), snap(1)],
    )
}

/// The look's Frame inset and Content pad.
fn drawn(host: &Offscreen) -> (f32, f32) {
    let look = &host.app().look;
    (look.frame.inset, look.content.pad)
}

/// What `settings.toml` says of `field`, once the window has written it.
fn saved(field: &str) -> Option<toml::Value> {
    let text = std::fs::read_to_string(settings_file()).ok()?;
    text.parse::<toml::Table>().ok()?.get(field).cloned()
}

fn wait_saved(host: &mut Offscreen, field: &str, value: toml::Value) {
    let reached = host.run_until(PATIENCE, |_| saved(field).as_ref() == Some(&value));
    assert!(
        reached,
        "{field} to be saved as {value}: {:?}",
        saved(field)
    );
}

fn focus(host: &mut Offscreen, key: &'static str) {
    host.runtime()
        .update(move |_: &mut App, tx: &mut Tx| tx.focus(key, true));
    idle(host);
}

fn press(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
    idle(host);
}

fn open_popover(host: &mut Offscreen) {
    focus(host, "header-appearance");
    press(host, Key::Enter);
    let opened = host.run_until(PATIENCE, |host| {
        !view::all(&host.tree(), &|node| node["key"] == "appearance-frame").is_empty()
    });
    assert!(opened, "the popover to open");
}

/// The window draws at the saved densities from the start, and the popover's sliders are
/// named and valued as GTK's, the toggle of the preset the value rests on pressed.
#[test]
fn the_saved_densities_are_drawn_from_the_start_and_shown_by_the_sliders() {
    hosted_with(
        MODULE,
        "columns",
        SAVED.to_owned(),
        "density::the_saved_densities_are_drawn_from_the_start_and_shown_by_the_sliders",
        |mut host| {
            idle(&mut host);
            assert_eq!(drawn(&host), (4.0, 6.0));

            open_popover(&mut host);
            assert_eq!(
                slider(&host, "appearance-frame"),
                ("Frame".to_owned(), "Airy · 4 px".to_owned(), [false, true])
            );
            assert_eq!(
                slider(&host, "appearance-content"),
                (
                    "Content".to_owned(),
                    "Custom · 6 px".to_owned(),
                    [false, false]
                )
            );
        },
    );
}

/// Each change a slider makes is saved and drawn at once and the popover stays open: an arrow
/// steps 1 px, Page Up the distance between the presets, and Return on the Compact toggle
/// sets that preset, saved by its name.
#[test]
fn a_sliders_change_is_saved_and_drawn_at_once() {
    hosted_with(
        MODULE,
        "columns",
        SAVED.to_owned(),
        "density::a_sliders_change_is_saved_and_drawn_at_once",
        |mut host| {
            idle(&mut host);
            open_popover(&mut host);
            focus(&mut host, "appearance-content");

            press(&mut host, Key::Right);
            wait_saved(&mut host, "browser_density_custom_px", 7.into());
            assert_eq!(drawn(&host), (4.0, 7.0));
            press(&mut host, Key::PageUp);
            wait_saved(&mut host, "browser_density_custom_px", 11.into());
            assert_eq!(drawn(&host).1, 11.0);
            assert_eq!(saved("browser_density"), Some("custom".into()));

            focus(&mut host, "appearance-content-snap-0");
            press(&mut host, Key::Enter);
            wait_saved(&mut host, "browser_density", "compact".into());
            assert_eq!(drawn(&host), (4.0, 4.0));
            assert_eq!(
                slider(&host, "appearance-content"),
                (
                    "Content".to_owned(),
                    "Compact · 4 px".to_owned(),
                    [true, false]
                )
            );
        },
    );
}

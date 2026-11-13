// SPDX-License-Identifier: MIT

//! The tile size on the offscreen host: the window's keys and Ctrl+Shift+wheel step it, a
//! reset goes back to the size the folder follows, and it is kept as GTK keeps it.

use std::path::Path;

use rune::{Key, Mods};

use crate::{
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, fixture,
        view::{Shape, hosted_with, open, press, wheel_held},
    },
};

const MODULE: &str = "icons::size::tests";

const ICONS: Shape = Shape {
    described: "Icons view",
    list: "grid",
    entry: "grid-cell",
};

fn ctrl_alt() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        alt: true,
        ..Mods::default()
    }
}

/// Ctrl+Alt with `key`, as Rune's window host sends a chord with Ctrl held.
fn chord(host: &mut Offscreen, key: char) {
    press(host, Key::Command(key), ctrl_alt());
}

/// One notch of Ctrl+Shift+wheel over the grid: up is larger.
fn size_wheel(host: &mut Offscreen, up: bool) {
    let held = Mods {
        ctrl: true,
        cmd: true,
        shift: true,
        ..Mods::default()
    };
    wheel_held(host, if up { 120.0 } else { -120.0 }, held);
}

fn size(host: &Offscreen) -> f32 {
    host.app().icons.size
}

fn saved_size() -> Option<i64> {
    let file = strata_core::storage::config_directory().join("settings.toml");
    let text = std::fs::read_to_string(file).ok()?;
    let table: toml::Table = text.parse().ok()?;
    table.get("icons_thumbnail_size")?.as_integer()
}

fn wait_saved(host: &mut Offscreen, size: i64) {
    let saved = host.run_until(PATIENCE, |_| saved_size() == Some(size));
    assert!(saved, "settings.toml holds {:?}, not {size}", saved_size());
}

fn opened(host: &mut Offscreen, folder: &Path) {
    open(host, ICONS, folder);
}

/// While folders' views are not remembered the size is the saved default: each step is
/// saved, and a reset goes back to GTK's 64 and saves it.
#[test]
fn the_window_keys_step_and_reset_the_saved_size() {
    let name = "the_window_keys_step_and_reset_the_saved_size";
    let saved = "remember_folder_views = false\nicons_thumbnail_size = 96\n".to_owned();
    hosted_with(MODULE, "icons", saved, name, |mut host| {
        let folder = fixture();
        opened(&mut host, folder.path());
        assert_eq!(size(&host), 96.0, "the saved size");

        chord(&mut host, '=');
        assert_eq!(size(&host), 112.0, "a step larger");
        chord(&mut host, '-');
        chord(&mut host, '-');
        assert_eq!(size(&host), 80.0, "two steps smaller");
        wait_saved(&mut host, 80);

        chord(&mut host, '0');
        assert_eq!(size(&host), 64.0, "reset");
        wait_saved(&mut host, 64);
    });
}

/// Ctrl+Shift+wheel over the grid steps the size as the keys do.
#[test]
fn ctrl_shift_wheel_steps_the_size() {
    let name = "ctrl_shift_wheel_steps_the_size";
    let saved = "remember_folder_views = false\nicons_thumbnail_size = 64\n".to_owned();
    hosted_with(MODULE, "icons", saved, name, |mut host| {
        let folder = fixture();
        opened(&mut host, folder.path());

        size_wheel(&mut host, true);
        assert_eq!(size(&host), 72.0, "a notch up");
        size_wheel(&mut host, false);
        size_wheel(&mut host, false);
        assert_eq!(size(&host), 56.0, "two notches down");
        wait_saved(&mut host, 56);
    });
}

/// While folders' views are remembered, a changed size is not saved as the default: it lasts
/// while its folder shows, another folder shows the default, and a reset goes back to it.
#[test]
fn a_remembered_folders_size_leaves_the_default_alone() {
    let name = "a_remembered_folders_size_leaves_the_default_alone";
    let saved = "remember_folder_views = true\nicons_thumbnail_size = 96\n".to_owned();
    hosted_with(MODULE, "icons", saved, name, |mut host| {
        let folder = fixture();
        opened(&mut host, folder.path());

        chord(&mut host, '=');
        chord(&mut host, '=');
        assert_eq!(size(&host), 128.0, "two steps larger");
        chord(&mut host, '0');
        assert_eq!(size(&host), 96.0, "reset to the default");

        chord(&mut host, '=');
        opened(&mut host, &folder.path().join("documents"));
        assert_eq!(size(&host), 96.0, "another folder");
        assert_eq!(saved_size(), Some(96), "the default saved");
    });
}

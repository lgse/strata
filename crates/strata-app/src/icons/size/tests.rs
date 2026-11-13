// SPDX-License-Identifier: MIT

//! The tile size on the offscreen host: the window's keys and Ctrl+Shift+wheel step it, a
//! reset goes back to the size the folder follows, and it is kept as GTK keeps it.

use std::path::{Path, PathBuf};

use rune::{Key, Mods};
use strata_core::model::Location;
use strata_desktop::settings::{
    folder_views::{FolderViews, key_for_location},
    folder_views_path,
};

use crate::{
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, fixture, hosted_after,
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

/// The default as this process holds it, saved or not.
fn default_in_memory(host: &mut Offscreen) -> i32 {
    shell::saved(&mut host.runtime().state)
        .preferences()
        .icons_thumbnail_size
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

/// While folders' views are remembered, a changed size is the folder's own: the default is
/// left alone, another folder shows the default, the folder shows its size again when it
/// comes back, `folder-views.toml` holds it, and a reset forgets it.
#[test]
fn a_remembered_folder_keeps_its_own_size() {
    let name = "a_remembered_folder_keeps_its_own_size";
    let saved = "remember_folder_views = true\nicons_thumbnail_size = 96\n".to_owned();
    hosted_with(MODULE, "icons", saved, name, |mut host| {
        let folder = fixture();
        opened(&mut host, folder.path());

        chord(&mut host, '=');
        chord(&mut host, '=');
        assert_eq!(size(&host), 128.0, "two steps larger");
        assert_eq!(
            default_in_memory(&mut host),
            96,
            "the default after the steps"
        );
        wait_folder_saved(&mut host, folder.path(), Some(128));
        chord(&mut host, '0');
        assert_eq!(size(&host), 96.0, "reset to the default");
        wait_folder_saved(&mut host, folder.path(), None);

        chord(&mut host, '=');
        opened(&mut host, &folder.path().join("documents"));
        assert_eq!(size(&host), 96.0, "another folder");
        opened(&mut host, folder.path());
        assert_eq!(size(&host), 112.0, "the folder's own size again");
        wait_folder_saved(&mut host, folder.path(), Some(112));
        assert_eq!(default_in_memory(&mut host), 96, "the default");
        assert_eq!(saved_size(), Some(96), "the default saved");
    });
}

/// A folder's size saved by an earlier session, or by another Strata process while this one
/// runs, is the size it shows.
#[test]
fn a_saved_folder_size_is_shown_from_start_and_followed() {
    let name = "a_saved_folder_size_is_shown_from_start_and_followed";
    hosted_after(
        MODULE,
        name,
        || {
            seed_settings("remember_folder_views = true\nicons_thumbnail_size = 64\n");
            std::fs::create_dir_all(shown().join("inner")).expect("folder");
            seed_folder_size(&shown(), 160);
        },
        |mut host| {
            opened(&mut host, &shown());
            assert_eq!(size(&host), 160.0, "the size saved before start-up");

            seed_folder_size(&shown(), 200);
            let followed = host.run_until(PATIENCE, |host| size(host) == 200.0);
            assert!(followed, "another process's size at {}", size(&host));
        },
    );
}

/// A folder of the test's own configuration directory, so that start-up can find its saved
/// size: the fixture is made only once the window is up.
fn shown() -> PathBuf {
    strata_core::storage::config_directory().join("shown")
}

fn seed_settings(more: &str) {
    let file = strata_core::storage::config_directory().join("settings.toml");
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
    let saved =
        format!("mode = \"theme\"\ntheme = \"azure-glow\"\nbrowser_mode = \"icons\"\n{more}");
    std::fs::write(file, saved).expect("settings");
}

/// Writes `folder-views.toml` as another Strata process saves it.
fn seed_folder_size(folder: &Path, size: i32) {
    let key = key_for_location(&Location::local(folder)).expect("a key");
    let mut views = FolderViews::default();
    views.update(&key, |view| view.icons_size = Some(size));
    let path = folder_views_path();
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("config folder");
    std::fs::write(path, views.to_toml().expect("toml")).expect("folder views");
}

/// The size `folder-views.toml` holds for `folder`.
fn folder_saved(folder: &Path) -> Option<Option<i32>> {
    let text = std::fs::read_to_string(folder_views_path()).ok()?;
    let (views, _) = FolderViews::parse(&text).ok()?;
    let key = key_for_location(&Location::local(folder))?;
    Some(views.view(&key).icons_size)
}

fn wait_folder_saved(host: &mut Offscreen, folder: &Path, size: Option<i32>) {
    let saved = host.run_until(PATIENCE, |_| folder_saved(folder) == Some(size));
    assert!(saved, "folder-views.toml holds {:?}", folder_saved(folder));
}

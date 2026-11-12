// SPDX-License-Identifier: MIT

//! The registry over the window the crate registers: what a key, a menu row and a button
//! reach, and what the catalog reads of the saved settings.

use rune::{Key, KeyEvent, Mods, Tx};
use strata_components::contract::Rect;
use strata_core::model::Location;
use strata_desktop::keys::{Map, Modifiers, SHORTCUT_MODIFIERS};

use super::{On, Outcome, Reads, Stage, bound, has, run};
use crate::{
    App,
    host::offscreen::Offscreen,
    menus::{self, Request},
    shell,
    test_support::{PATIENCE, fixture, hosted, hosted_after, send_key},
};

const MODULE: &str = "commands::tests";

mod menu_key;
mod routing;
mod views;

/// Opens the fixture and waits for the browser to list it.
fn listed(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    folder
}

/// A key with its modifiers held, as a keyboard sends one.
fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn act<T>(host: &mut Offscreen, act: impl FnOnce(&mut App, &mut Tx) -> T) -> T {
    let mut said = None;
    host.runtime().update(|app, tx| said = Some(act(app, tx)));
    while host.turn() {}
    said.expect("the update ran")
}

/// `browser_commands` in `src/ui/window/keyboard/commands.rs` reads refresh with any
/// modifiers held, which the window's own chords, matched as Rune spells them, did not.
#[test]
fn refresh_is_read_with_any_modifiers_held_as_the_gtk_stage_reads_it() {
    hosted(
        MODULE,
        "refresh_is_read_with_any_modifiers_held_as_the_gtk_stage_reads_it",
        |mut host| {
            let _folder = listed(&mut host);
            let copied = host.app().rows_copied();
            let held = Mods {
                shift: true,
                cmd: true,
                ctrl: true,
                ..Mods::default()
            };

            press(&mut host, Key::Function(5), held);

            let read_again = host.run_until(PATIENCE, |host| host.app().rows_copied() > copied);
            assert!(read_again, "Ctrl+Shift+F5 read nothing");
        },
    );
}

/// The catalog resolves a command through the map the settings select, read at each press
/// as the GTK application reads its preference manager.
#[test]
fn the_catalogs_map_follows_the_saved_10xer_mode() {
    hosted_after(
        MODULE,
        "the_catalogs_map_follows_the_saved_10xer_mode",
        || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = "mode = \"theme\"\ntheme = \"azure-glow\"\ntenxer_mode = true\n";
            std::fs::write(file, saved).expect("settings");
        },
        |mut host| {
            while host.turn() {}
            assert_eq!(Map::active(), Map::Tenxer);

            act(&mut host, |app, _| {
                shell::saved(app).change(|settings| settings.tenxer_mode = false);
            });
            press(&mut host, Key::Function(9), Mods::default());

            assert_eq!(Map::active(), Map::Default);
        },
    );
}

/// A menu row and a key end in one command: the pane menu's row for the hidden files does
/// what Ctrl+H does, on the column the menu was for. A name no part registered runs nothing.
#[test]
fn a_command_runs_by_its_name_as_it_does_by_its_key() {
    hosted(
        MODULE,
        "a_command_runs_by_its_name_as_it_does_by_its_key",
        |mut host| {
            let _folder = listed(&mut host);
            let shown = |host: &Offscreen| host.app().browser().preferences().show_hidden;
            let before = shown(&host);

            let row = act(&mut host, |app, tx| run(app, "hidden", On::Column(0), tx));
            assert_eq!((row, shown(&host)), (Outcome::Ran, !before));

            let ctrl = Mods {
                cmd: true,
                ctrl: true,
                ..Mods::default()
            };
            press(&mut host, Key::Command('h'), ctrl);
            assert_eq!(shown(&host), before, "the key ran the same command");

            let unknown = act(&mut host, |app, tx| run(app, "no-such", On::Focus, tx));
            assert_eq!((unknown, shown(&host)), (Outcome::Declined, before));
        },
    );
}

/// `input_owner` gives a menu every key, so the chain's commands wait under one; what follows
/// a key the chain left still runs there, and refresh is one of those window actions
/// (`input_owner.rs:21`, `keyboard.rs:147`).
#[test]
fn under_an_open_menu_the_chains_commands_wait_and_refresh_still_runs() {
    hosted(
        MODULE,
        "under_an_open_menu_the_chains_commands_wait_and_refresh_still_runs",
        |mut host| {
            let _folder = listed(&mut host);
            let shown = |host: &Offscreen| host.app().browser().preferences().show_hidden;
            let before = shown(&host);
            let ctrl = Mods {
                cmd: true,
                ctrl: true,
                ..Mods::default()
            };
            act(&mut host, |app, tx| {
                let folder = Request {
                    depth: 0,
                    entry: None,
                    anchor: Rect::new(40.0, 80.0, 0.0, 0.0),
                };
                menus::open(app, folder, tx);
            });
            assert!(host.app().menu.state.is_open());

            press(&mut host, Key::Command('h'), ctrl);
            assert_eq!(shown(&host), before, "Ctrl+H waited under the menu");

            let copied = host.app().rows_copied();
            press(&mut host, Key::Function(5), Mods::default());
            let read_again = host.run_until(PATIENCE, |host| host.app().rows_copied() > copied);
            assert!(read_again, "F5 read nothing under the menu");
        },
    );
}

/// What each of the header's five buttons does comes from the registry. None has a command
/// until the part that owns it registers one, and the list shrinks as those parts land.
#[test]
fn the_headers_buttons_wait_for_the_parts_that_own_their_commands() {
    hosted(
        MODULE,
        "the_headers_buttons_wait_for_the_parts_that_own_their_commands",
        |host| {
            let inert: Vec<&str> = shell::header_commands()
                .filter(|command| !has(host.app(), command))
                .collect();
            assert_eq!(
                inert,
                [
                    "toggle-sidebar",
                    "tab-new",
                    "search",
                    "appearance",
                    "settings"
                ]
            );
        },
    );
}

/// What lists keys reads them from the registry: every matcher of a command's keys is the
/// catalog command the action says it is, at the stage GTK reads it.
#[test]
fn the_keys_a_command_answers_are_listed_with_their_catalog_command_and_stage() {
    hosted(
        MODULE,
        "the_keys_a_command_answers_are_listed_with_their_catalog_command_and_stage",
        |host| {
            let mut refresh = Vec::new();
            for key in bound(host.app()) {
                let Reads::Commands(matchers) = key.reads else {
                    continue;
                };
                for matcher in *matchers {
                    assert_eq!(Some(matcher.command()), key.command, "{}", key.id);
                }
                if key.id == "refresh" {
                    refresh.push((key.stage, matchers[0].map(), matchers[0].ignored()));
                }
            }
            assert_eq!(
                refresh,
                [
                    (Stage::FileCommands, None, SHORTCUT_MODIFIERS),
                    (Stage::AfterYield, None, Modifiers::empty()),
                ]
            );
        },
    );
}

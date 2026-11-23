// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods, Tx};
use strata_components::contract::Rect;
use strata_core::model::Location;
use strata_desktop::{
    keys::{Map, Modifiers, SHORTCUT_MODIFIERS},
    settings::KeymapOverrides,
};

use super::{On, Outcome, Reads, Stage, bound, has, run};
use crate::{
    App,
    host::offscreen::Offscreen,
    menus::{self, Request},
    shell,
    slots::{Entry, View},
    test_support::{PATIENCE, fixture, hosted, hosted_after, send_key},
};

const MODULE: &str = "commands::tests";

macro_rules! in_each_view {
    ($run:ident: $($mode:literal => $test:ident,)*) => {
        $(
            #[test]
            fn $test() {
                $run($mode, stringify!($test));
            }
        )*

        #[test]
        fn every_registered_view_has_this_test() {
            crate::commands::tests::assert_each_view_answers_to_one_of(&[$($mode,)*]);
        }
    };
}

mod menu_key;
mod navigate_up;
mod places;
mod refresh_anchor;
mod routing;
mod views;
mod wake;

fn assert_each_view_answers_to_one_of(modes: &[&str]) {
    let views: Vec<_> = crate::PARTS.iter().filter_map(Entry::view).collect();
    let tests_of = |view: &&View<App>| modes.iter().filter(|mode| (view.shows)(mode)).count();
    assert_eq!(
        views.iter().map(tests_of).collect::<Vec<_>>(),
        vec![1; modes.len()],
        "each registered view answers to one of {modes:?}, and each of those is a view"
    );
}

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

#[test]
fn a_saved_keymap_rebinds_a_command_and_a_change_to_it_applies() {
    hosted_after(
        MODULE,
        "a_saved_keymap_rebinds_a_command_and_a_change_to_it_applies",
        || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = "[keymap.default]\ntoggle_hidden_files = [\"<Control>j\"]\n";
            std::fs::write(file, saved).expect("settings");
        },
        |mut host| {
            let _folder = listed(&mut host);
            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            let hidden_shown = |host: &Offscreen| host.app().browser().preferences().show_hidden;

            press(&mut host, Key::Command('h'), ctrl);
            assert!(!hidden_shown(&host), "the shipped key still answers");
            press(&mut host, Key::Command('j'), ctrl);
            assert!(hidden_shown(&host), "the saved key does not answer");

            act(&mut host, |app, _| {
                shell::saved(app).change(|settings| settings.keymap = KeymapOverrides::default());
            });
            press(&mut host, Key::Command('j'), ctrl);
            assert!(hidden_shown(&host), "the removed key still answers");
            press(&mut host, Key::Command('h'), ctrl);
            assert!(!hidden_shown(&host), "the shipped key is not back");
        },
    );
}

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

#[test]
fn the_headers_buttons_wait_for_the_parts_that_own_their_commands() {
    hosted(
        MODULE,
        "the_headers_buttons_wait_for_the_parts_that_own_their_commands",
        |host| {
            let inert: Vec<&str> = shell::header_commands()
                .filter(|command| !has(host.app(), command))
                .collect();
            assert!(inert.is_empty(), "every header button is wired: {inert:?}");
        },
    );
}

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

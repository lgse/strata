// SPDX-License-Identifier: MIT

//! The view switch in a window: the keys show another view of the browser's folder, with its
//! cursor and selection, and save the choice; Columns shown again opens the selected folder
//! while it mirrors.

use std::{path::PathBuf, time::Duration};

use rune::{Key, Mods};
use strata_fs::app::VisibleDepths;

use crate::{
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, fixture,
        view::{Shape, browser_holds, hosted_with, idle, open, press, seen, wait},
    },
};

const MODULE: &str = "appearance::tests";

/// How long a test waits to see that a child column does not open.
const NONE_OPENS: Duration = Duration::from_millis(500);

const ICONS: Shape = Shape {
    described: "Icons view",
    list: "grid",
    entry: "grid-cell",
};

const LIST: Shape = Shape {
    described: "List view",
    list: "list",
    entry: "list-item",
};

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

/// The name the QA protocol gives the view shown.
fn shown(host: &Offscreen) -> Option<&'static str> {
    shell::shown_view(host.app()).map(|view| view.probe.name)
}

/// The columns the browser's background revalidation rechecks.
fn rechecks(host: &Offscreen) -> VisibleDepths {
    host.app().browser().freshness_input().scope
}

fn settings_file() -> PathBuf {
    strata_core::storage::config_directory().join("settings.toml")
}

/// Waits for `settings.toml` to say `line`.
fn wait_saved(host: &mut Offscreen, line: &str) {
    let saved = |_: &Offscreen| {
        std::fs::read_to_string(settings_file()).is_ok_and(|text| text.contains(line))
    };
    let reached = host.run_until(PATIENCE, saved);
    assert!(reached, "settings.toml to say {line}");
}

/// Each key shows its view of the folder the browser has open, with the browser's cursor and
/// selection on the same entry and the keyboard in the view, and saves the choice: List
/// shown again after Columns shows the folder as it is now, not as it last showed it. The
/// browser rechecks the columns the view shows, from the start: every column in Columns, the
/// active one in Icons and List, as GTK's `enter_mode` sets it.
#[test]
fn a_view_key_shows_that_view_of_the_open_folder_and_saves_it() {
    hosted_with(
        MODULE,
        "list",
        String::new(),
        "a_view_key_shows_that_view_of_the_open_folder_and_saves_it",
        |mut host| {
            let folder = fixture();
            open(&mut host, LIST, folder.path());
            press(&mut host, Key::End, Mods::default());
            let held = browser_holds(&host);
            assert_eq!(held.1.as_deref(), Some("todo.txt"));
            assert_eq!(rechecks(&host), VisibleDepths::Active, "List at start");

            for (digit, name, mode, shape, scope) in [
                ('2', "Icons", "icons", Some(ICONS), VisibleDepths::Active),
                ('1', "Columns", "columns", None, VisibleDepths::All),
                ('3', "List", "list", Some(LIST), VisibleDepths::Active),
            ] {
                press(&mut host, Key::Command(digit), ctrl());
                assert_eq!(shown(&host), Some(name));
                assert_eq!(browser_holds(&host), held, "{name}");
                assert_eq!(rechecks(&host), scope, "{name}");
                wait_saved(&mut host, &format!("browser_mode = \"{mode}\""));
                let Some(shape) = shape else {
                    continue;
                };
                let seen = wait(&mut host, shape, "the switched view's listing", |seen| {
                    seen.entries.len() == 5
                });
                assert_eq!(seen.selected, ["todo.txt"], "{name}");
                assert_eq!(seen.focused.as_deref(), Some("todo.txt"), "{name}");
                assert!(seen.keyboard, "{name} has the keyboard");
            }
        },
    );
}

/// Columns shown again after List opens the folder List left selected as the child column
/// while it mirrors the selection.
#[test]
fn columns_shown_again_opens_the_selected_folder_while_it_mirrors() {
    returning_to_columns(
        true,
        "columns_shown_again_opens_the_selected_folder_while_it_mirrors",
    );
}

/// And opens none while it does not mirror.
#[test]
fn columns_shown_again_opens_no_folder_while_it_does_not_mirror() {
    returning_to_columns(
        false,
        "columns_shown_again_opens_no_folder_while_it_does_not_mirror",
    );
}

fn returning_to_columns(mirrors: bool, name: &str) {
    let saved = format!("columns_mirror_selection = {mirrors}\n");
    hosted_with(MODULE, "list", saved, name, |mut host| {
        let folder = fixture();
        open(&mut host, LIST, folder.path());
        press(&mut host, Key::Down, Mods::default());
        assert_eq!(seen(&host, LIST).selected, ["documents"]);

        press(&mut host, Key::Command('1'), ctrl());
        assert_eq!(shown(&host), Some("Columns"));
        let browser = host.app().browser().clone();
        // Long enough for a child to open, where none is to.
        let patience = if mirrors { PATIENCE } else { NONE_OPENS };
        host.run_until(patience, |_| browser.location_at(1).is_some());
        idle(&mut host);
        let child = browser
            .location_at(1)
            .map(|location| location.display_name());
        let expected = mirrors.then(|| "documents".to_owned());
        assert_eq!(child, expected, "mirroring {mirrors}");
        assert_eq!(browser_holds(&host).0, ["documents"]);
    });
}

/// A child column the cursor was to open in Columns, its wait not over when Icons shows, is
/// not opened: Columns mirrors only while it is shown, as GTK's `child_mirror_blocked` asks
/// for the mode when its wait is over.
#[test]
fn a_mirror_waiting_when_another_view_shows_opens_no_column() {
    hosted_with(
        MODULE,
        "columns",
        String::new(),
        "a_mirror_waiting_when_another_view_shows_opens_no_column",
        |mut host| {
            let folder = fixture();
            host.app()
                .open(strata_core::model::Location::local(folder.path()));
            let browser = host.app().browser().clone();
            let listed = host.run_until(PATIENCE, |_| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            assert!(listed, "the folder to be listed");
            idle(&mut host);
            assert_eq!(browser_holds(&host).1.as_deref(), Some("archive"));

            press(&mut host, Key::Down, Mods::default());
            press(&mut host, Key::Command('2'), ctrl());
            assert_eq!(shown(&host), Some("Icons"));
            host.run_until(NONE_OPENS, |_| browser.location_at(1).is_some());

            assert_eq!(browser.location_at(1), None);
            assert_eq!(browser_holds(&host).0, ["documents"]);
        },
    );
}

mod density;
mod popover;
mod text_size;

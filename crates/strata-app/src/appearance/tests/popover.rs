// SPDX-License-Identifier: MIT

//! The Appearance popover on the offscreen host: the header's toggle opens it on its first
//! row, its rows are named and checked as GTK's, a view row shows its view and closes it, it
//! keeps the window's keys while it is open and Escape closes it, its stepper changes the text
//! size and stays open, and Group by file type and Hidden files act and close it.

use rune::{Key, KeyEvent, Mods, Tx};
use serde_json::Value;

use super::{LIST, MODULE, ctrl, shown};
use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, fixture, send_key,
        view::{self, hosted_with, open},
    },
};

const BUTTON: &str = "header-appearance";

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn node(host: &Offscreen, key: &str) -> Option<Value> {
    let tree = host.tree();
    view::all(&tree, &|node| node["key"] == key)
        .into_iter()
        .next()
        .cloned()
}

fn has(node: &Value, state: &str) -> bool {
    node["states"]
        .as_array()
        .is_some_and(|states| states.iter().any(|seen| seen == state))
}

fn focused(host: &mut Offscreen) -> Option<String> {
    let mut said = None;
    host.runtime()
        .update(|_: &mut App, tx: &mut Tx| said = tx.focused().map(str::to_owned));
    idle(host);
    said
}

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    idle(host);
}

/// Return on the element keyed `key`, with the keyboard given to it first.
fn return_on(host: &mut Offscreen, key: &'static str) {
    host.runtime()
        .update(move |_: &mut App, tx: &mut Tx| tx.focus(key, true));
    idle(host);
    press(host, Key::Enter, Mods::default());
}

fn is_open(host: &Offscreen) -> bool {
    node(host, "appearance-columns").is_some()
}

/// Opens the popover from the header's Appearance button.
fn open_popover(host: &mut Offscreen) {
    return_on(host, BUTTON);
    let opened = host.run_until(PATIENCE, is_open);
    assert!(opened, "Return on Appearance opened its popover");
    idle(host);
}

fn wait_closed(host: &mut Offscreen, after: &str) {
    let closed = host.run_until(PATIENCE, |host| !is_open(host));
    assert!(closed, "{after} closed the popover");
}

/// A row's role, name and description, and whether it is checked and sensitive.
fn row(host: &Offscreen, key: &str) -> (String, String, String, bool, bool) {
    let node = node(host, key).unwrap_or_else(|| panic!("the row {key}"));
    (
        view::text(&node, "role"),
        view::text(&node, "name"),
        view::text(&node, "description"),
        has(&node, "checked"),
        !has(&node, "disabled"),
    )
}

fn in_columns(name: &'static str, run: impl FnOnce(Offscreen)) {
    hosted_with(MODULE, "columns", String::new(), name, run);
}

/// Opens the fixture in Columns and waits for the browser to list it.
fn open_in_columns(host: &mut Offscreen, folder: &std::path::Path) {
    host.app().open(strata_core::model::Location::local(folder));
    let browser = host.app().browser().clone();
    let listed = host.run_until(PATIENCE, |_| {
        browser
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    assert!(listed, "the folder to be listed");
    idle(host);
}

/// The header's Appearance button is a toggle, pressed while the popover is open; the
/// popover opens with the keyboard on its first row, and its rows are named, checked and
/// sensitive as GTK's are in Columns.
#[test]
fn the_button_opens_the_popover_on_its_first_row_named_as_gtks() {
    in_columns(
        "popover::the_button_opens_the_popover_on_its_first_row_named_as_gtks",
        |mut host| {
            let folder = fixture();
            open_in_columns(&mut host, folder.path());
            let button = node(&host, BUTTON).expect("the Appearance button");
            assert_eq!(view::text(&button, "name"), "Appearance");
            assert!(!has(&button, "checked"));

            open_popover(&mut host);
            assert!(has(&node(&host, BUTTON).expect("the button"), "checked"));
            assert_eq!(focused(&mut host).as_deref(), Some("appearance-columns"));
            let cases = [
                (
                    "appearance-columns",
                    "menu-item-radio",
                    "Columns",
                    "",
                    true,
                    true,
                ),
                (
                    "appearance-icons",
                    "menu-item-radio",
                    "Icons",
                    "",
                    false,
                    true,
                ),
                (
                    "appearance-list",
                    "menu-item-radio",
                    "List",
                    "",
                    false,
                    true,
                ),
                (
                    "appearance-preview",
                    "button",
                    "Preview panel",
                    "Toggle preview panel while browsing (Space)",
                    false,
                    true,
                ),
                (
                    "appearance-group",
                    "menu-item-check-box",
                    "Group by file type",
                    "Group List entries under file-type headings",
                    false,
                    false,
                ),
                (
                    "appearance-hidden",
                    "menu-item-check-box",
                    "Hidden files",
                    "Ctrl+H",
                    false,
                    true,
                ),
            ];
            for (key, role, name, description, checked, sensitive) in cases {
                let expected = (
                    role.to_owned(),
                    name.to_owned(),
                    description.to_owned(),
                    checked,
                    sensitive,
                );
                assert_eq!(row(&host, key), expected, "{key}");
            }
        },
    );
}

/// A view row shows its view as Ctrl+3 does, saves it, closes the popover and gives the view
/// the keyboard.
#[test]
fn a_view_row_shows_its_view_and_closes_the_popover() {
    in_columns(
        "popover::a_view_row_shows_its_view_and_closes_the_popover",
        |mut host| {
            let folder = fixture();
            open_in_columns(&mut host, folder.path());
            open_popover(&mut host);
            return_on(&mut host, "appearance-list");
            wait_closed(&mut host, "the List row");

            assert_eq!(shown(&host), Some("List"));
            assert_eq!(shell::preferences(host.app()).browser_mode, "list");
            assert_eq!(focused(&mut host).as_deref(), shell::view_focus(host.app()));
        },
    );
}

/// While the popover is open it keeps the window's keys, as GTK's plain popover does: Ctrl+3
/// shows nothing else. Escape closes it and gives the button the keyboard back.
#[test]
fn the_popover_keeps_the_windows_keys_and_escape_closes_it() {
    in_columns(
        "popover::the_popover_keeps_the_windows_keys_and_escape_closes_it",
        |mut host| {
            let folder = fixture();
            open_in_columns(&mut host, folder.path());
            open_popover(&mut host);

            press(&mut host, Key::Command('3'), ctrl());
            assert_eq!(shown(&host), Some("Columns"));
            assert!(is_open(&host), "Ctrl+3 left the popover open");

            press(&mut host, Key::Escape, Mods::default());
            wait_closed(&mut host, "Escape");
            assert_eq!(focused(&mut host).as_deref(), Some(BUTTON));
        },
    );
}

/// The stepper steps the text size and shows it, and the popover stays open; the size's
/// button resets it.
#[test]
fn the_stepper_changes_the_text_size_and_stays_open() {
    in_columns(
        "popover::the_stepper_changes_the_text_size_and_stays_open",
        |mut host| {
            let folder = fixture();
            open_in_columns(&mut host, folder.path());
            open_popover(&mut host);
            return_on(&mut host, "appearance-text-larger");
            let grew = host.run_until(PATIENCE, |host| {
                shell::preferences(host.app()).text_size.root_font_px() == 14
            });
            assert!(grew, "Increase text size");
            assert!(is_open(&host), "the stepper left the popover open");
            let reset = row(&host, "appearance-text-reset");
            assert_eq!(
                (reset.1.as_str(), reset.2.as_str()),
                ("14 px", "Reset text size (Ctrl+0)")
            );
            assert_eq!(
                row(&host, "appearance-text-larger").1,
                "Increase text size (Ctrl++)"
            );
            assert_eq!(
                row(&host, "appearance-text-smaller").1,
                "Decrease text size (Ctrl+−)"
            );

            return_on(&mut host, "appearance-text-reset");
            let reset = host.run_until(PATIENCE, |host| {
                shell::preferences(host.app()).text_size.root_font_px() == 13
            });
            assert!(reset, "the size's button reset it");
        },
    );
}

/// In List, Group by file type is sensitive: it saves the setting, the List view groups the
/// folder shown with the cursor where it was, and the popover closes. Hidden files shows them, checked, and closes it.
#[test]
fn group_by_file_type_regroups_the_list_and_hidden_files_shows_them() {
    hosted_with(
        MODULE,
        "list",
        String::new(),
        "popover::group_by_file_type_regroups_the_list_and_hidden_files_shows_them",
        |mut host| {
            let folder = fixture();
            open(&mut host, LIST, folder.path());
            press(&mut host, Key::End, Mods::default());
            press(&mut host, Key::Up, Mods::default());
            let cursor = view::seen(&host, LIST).focused;
            assert_eq!(cursor.as_deref(), Some("readme.md"));
            open_popover(&mut host);
            assert!(
                row(&host, "appearance-group").4,
                "Group is sensitive in List"
            );

            return_on(&mut host, "appearance-group");
            wait_closed(&mut host, "Group by file type");
            assert!(shell::preferences(host.app()).group_by_type);
            let browser = host.app().browser().clone();
            assert!(
                browser.shows_type_groups(0),
                "the browser groups the folder"
            );
            let headings = |host: &Offscreen| {
                let tree = host.tree();
                view::all(&tree, &|node| node["role"] == "heading").len()
            };
            idle(&mut host);
            assert!(
                headings(&host) > 0,
                "the List view shows the file-type groups at once"
            );
            assert_eq!(
                view::seen(&host, LIST).focused,
                cursor,
                "the cursor stays on its entry through the regroup"
            );

            open_popover(&mut host);
            assert!(row(&host, "appearance-group").3, "Group is checked");
            return_on(&mut host, "appearance-hidden");
            wait_closed(&mut host, "Hidden files");
            assert!(browser.preferences().show_hidden);
            open_popover(&mut host);
            assert!(row(&host, "appearance-hidden").3, "Hidden files is checked");
        },
    );
}

/// "Customize columns…" shows in List alone: it closes the popover and opens the List table's
/// columns menu, with the keyboard in it, as the header's Columns button does. In Columns the
/// row is not there.
#[test]
fn customize_columns_opens_the_list_columns_menu() {
    hosted_with(
        MODULE,
        "list",
        String::new(),
        "popover::customize_columns_opens_the_list_columns_menu",
        |mut host| {
            let folder = fixture();
            open(&mut host, LIST, folder.path());
            open_popover(&mut host);
            assert_eq!(
                row(&host, "appearance-customize-columns"),
                (
                    "menu-item".to_owned(),
                    "Customize columns…".to_owned(),
                    String::new(),
                    false,
                    true
                )
            );

            return_on(&mut host, "appearance-customize-columns");
            wait_closed(&mut host, "Customize columns…");
            let size = |host: &Offscreen| {
                let tree = host.tree();
                let items = view::all(&tree, &|node| {
                    node["role"] == "menu-item-check-box" && node["name"] == "Size"
                });
                !items.is_empty()
            };
            let opened = host.run_until(PATIENCE, size);
            assert!(opened, "the columns menu to open");
            let focus = focused(&mut host).unwrap_or_default();
            assert!(
                focus.starts_with("list-headings-menu"),
                "the keyboard is in the columns menu: {focus}"
            );

            press(&mut host, Key::Escape, Mods::default());
            press(&mut host, Key::Command('1'), ctrl());
            open_popover(&mut host);
            assert!(
                node(&host, "appearance-customize-columns").is_none(),
                "no Customize columns… in Columns"
            );
        },
    );
}

// SPDX-License-Identifier: MIT

//! 10xer's keyboard folder peek in Icons and List, on the offscreen host.

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, hosted_after, send_key,
        view::{all, text},
    },
};

const MODULE: &str = "peek::tests";

fn press(host: &mut Offscreen, key: Key) {
    press_with(host, key, Mods::default());
}

fn press_with(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn letter(host: &mut Offscreen, letter: &str) {
    press(host, Key::Text(letter.to_owned()));
}

fn peek_shown(host: &Offscreen) -> bool {
    let tree = host.tree();
    !all(&tree, &|node| text(node, "name") == "Folder peek").is_empty()
}

fn open(host: &Offscreen) -> bool {
    host.app().peek.state.target().is_some()
}

/// Selects the entry named `name` in `root` alone, the cursor on it, as a click does.
fn select(host: &mut Offscreen, root: &std::path::Path, name: &str) {
    let location = Location::local(root.join(name));
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        let id = shell::id_of(app, &location);
        let view = shell::shown_view(app).expect("a view");
        (view.rows.select_only)(app, (0, id), tx);
    });
    while host.turn() {}
}

/// A 10xer window in `mode` on a folder holding `documents` (two files), `pictures` and
/// `readme.md`, with the cursor on `name`.
fn on_folder(host: &mut Offscreen, name: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("folder");
    std::fs::create_dir(root.path().join("documents")).expect("folder");
    std::fs::create_dir(root.path().join("pictures")).expect("folder");
    for name in ["a.txt", "b.txt"] {
        std::fs::write(root.path().join("documents").join(name), name).expect("file");
    }
    std::fs::write(root.path().join("readme.md"), "readme").expect("file");
    host.app().open(Location::local(root.path()));
    assert!(host.run_until(PATIENCE, |h| {
        h.app().browser().column_entry_counts(0).is_some()
    }));
    while host.turn() {}
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(view, true);
    });
    while host.turn() {}
    select(host, root.path(), name);
    root
}

fn in_mode(mode: &'static str, name: &str, run: impl FnOnce(Offscreen)) {
    hosted_after(
        MODULE,
        name,
        move || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = format!("browser_mode = \"{mode}\"\ntenxer_mode = true\n");
            std::fs::write(file, saved).expect("settings");
        },
        run,
    );
}

fn i_toggles_the_peek_and_escape_closes_it_first(mode: &'static str, name: &str) {
    in_mode(mode, name, |mut host| {
        let _root = on_folder(&mut host, "documents");
        press(&mut host, Key::Text("i".to_owned()));
        assert!(
            open(&host) && peek_shown(&host),
            "i opens the folder's peek"
        );
        let listed = host.run_until(PATIENCE, |h| h.app().peek.entries.len() == 2);
        assert!(listed, "the browser lists the peeked folder");
        press(&mut host, Key::Text("i".to_owned()));
        assert!(!open(&host) && !peek_shown(&host), "i again closes it");

        press(&mut host, Key::Text("i".to_owned()));
        assert!(open(&host));
        press(&mut host, Key::Escape);
        assert!(!open(&host), "Escape closes the peek first");
        assert_eq!(host.app().browser().selected_positions(0), vec![0]);

        press(&mut host, Key::Text("i".to_owned()));
        assert!(open(&host));
        host.app().browser().close_peek();
        while host.turn() {}
        assert!(!open(&host), "a close from the browser closes the card");
    });
}

#[test]
fn list_i_toggles_the_peek_and_escape_closes_it_first() {
    i_toggles_the_peek_and_escape_closes_it_first(
        "list",
        "list_i_toggles_the_peek_and_escape_closes_it_first",
    );
}

#[test]
fn icons_i_toggles_the_peek_and_escape_closes_it_first() {
    i_toggles_the_peek_and_escape_closes_it_first(
        "icons",
        "icons_i_toggles_the_peek_and_escape_closes_it_first",
    );
}

/// On a file `i` is the preview's, and in Columns a folder's column opens: neither peeks.
#[test]
fn i_peeks_only_folders_in_icons_and_list() {
    in_mode(
        "list",
        "i_peeks_only_folders_in_icons_and_list",
        |mut host| {
            let _root = on_folder(&mut host, "readme.md");
            press(&mut host, Key::Text("i".to_owned()));
            assert!(!open(&host), "a file has no peek");
        },
    );
    in_mode(
        "columns",
        "i_peeks_only_folders_in_icons_and_list",
        |mut host| {
            let _root = on_folder(&mut host, "documents");
            press(&mut host, Key::Text("i".to_owned()));
            assert!(!open(&host), "Columns never peeks");
        },
    );
}

/// The keys that move the cursor one entry on in `mode`: `j` down a List, `l` along the grid.
fn step(mode: &str) -> &'static str {
    if mode == "icons" { "l" } else { "j" }
}

/// Any cursor or selection move closes the peek, as GTK's `keyboard_navigation` does: a key,
/// Invert Selection, a click, a find. Then `i` peeks the folder the cursor reached, and `i`
/// while a peek is open only closes it (`toggle_folder_peek`).
fn moves_close_the_peek(mode: &'static str, name: &str) {
    in_mode(mode, name, |mut host| {
        let root = on_folder(&mut host, "documents");
        letter(&mut host, "i");
        assert!(open(&host));
        letter(&mut host, step(mode));
        assert!(
            !open(&host) && !peek_shown(&host),
            "a cursor move closes it"
        );

        letter(&mut host, "i");
        assert!(open(&host), "i peeks the second folder");
        let ctrl = Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        };
        press_with(&mut host, Key::Command('r'), ctrl);
        assert!(!open(&host), "a selection change closes it too");

        letter(&mut host, "i");
        assert!(open(&host));
        select(&mut host, root.path(), "documents");
        assert!(!open(&host), "a click's selection closes it");
        letter(&mut host, "i");
        assert!(open(&host), "i peeks the folder selected");
        letter(&mut host, "i");
        assert!(!open(&host), "i while a peek is open closes it");

        letter(&mut host, "i");
        assert!(open(&host));
        letter(&mut host, "/");
        for typed in ["r", "e", "a", "d"] {
            letter(&mut host, typed);
        }
        press(&mut host, Key::Enter);
        assert!(!open(&host), "a find that moves the cursor closes it");
    });
}

#[test]
fn list_moves_close_the_peek() {
    moves_close_the_peek("list", "list_moves_close_the_peek");
}

#[test]
fn icons_moves_close_the_peek() {
    moves_close_the_peek("icons", "icons_moves_close_the_peek");
}

fn peeked_names(host: &Offscreen) -> Vec<String> {
    let entries = &host.app().peek.entries;
    entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect()
}

/// GTK's card lists the folder as it was when the peek opened: a file made in it later is not
/// added, and Reload (F5) reloads the view but keeps the peek open on that listing (checked by
/// hand on GTK with `qa-drive.sh`).
fn the_peek_keeps_its_listing_through_a_reload(mode: &'static str, name: &str) {
    in_mode(mode, name, |mut host| {
        let root = on_folder(&mut host, "documents");
        letter(&mut host, "i");
        let listed = |h: &Offscreen| peeked_names(h).len() == 2;
        assert!(
            host.run_until(PATIENCE, listed),
            "the card lists the folder"
        );
        let before = peeked_names(&host);

        std::fs::write(root.path().join("documents").join("fresh.txt"), "fresh").expect("file");
        let grew = host.run_until(std::time::Duration::from_secs(2), |h| {
            peeked_names(h) != before
        });
        assert!(!grew, "a file made after the peek opened is not added");

        press(&mut host, Key::Function(5));
        let _ = host.run_until(std::time::Duration::from_secs(1), |_| false);
        assert!(
            open(&host) && peek_shown(&host),
            "Reload keeps the peek open"
        );
        assert_eq!(peeked_names(&host), before, "on the listing it opened with");
    });
}

#[test]
fn list_the_peek_keeps_its_listing_through_a_reload() {
    the_peek_keeps_its_listing_through_a_reload(
        "list",
        "list_the_peek_keeps_its_listing_through_a_reload",
    );
}

#[test]
fn icons_the_peek_keeps_its_listing_through_a_reload() {
    the_peek_keeps_its_listing_through_a_reload(
        "icons",
        "icons_the_peek_keeps_its_listing_through_a_reload",
    );
}

/// A window in `mode`, 10xer or not as `tenxer` says, with "Folder peeking" on.
fn peeking(mode: &'static str, tenxer: bool, name: &str, run: impl FnOnce(Offscreen)) {
    hosted_after(
        MODULE,
        name,
        move || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = format!(
                "browser_mode = \"{mode}\"\ntenxer_mode = {tenxer}\nfolder_peeking = true\n"
            );
            std::fs::write(file, saved).expect("settings");
        },
        run,
    );
}

/// The middle of the first live element showing `text`.
fn centre_of(host: &Offscreen, text: &str) -> (f32, f32) {
    let element = host
        .elements()
        .into_iter()
        .find(|el| !el.exiting && el.text.as_deref() == Some(text))
        .unwrap_or_else(|| panic!("no element shows {text:?}"));
    let [x, y, w, h] = element.visible;
    (x + w.min(24.0) / 2.0, y + h / 2.0)
}

/// With "Folder peeking" on, the pointer resting on a folder opens its peek, listed by the
/// browser; a drag starting from it closes the peek, as GTK's item drag source does. The
/// folder is selected first, so the press that starts the drag moves no selection, which
/// would close the peek on its own.
fn a_resting_pointer_peeks_and_a_drag_closes_it(mode: &'static str, name: &str) {
    peeking(mode, false, name, |mut host| {
        let _root = on_folder(&mut host, "documents");
        let folder = centre_of(&host, "documents");
        host.runtime().pointer_move(folder.0 + 1.0, folder.1);
        host.turn();
        host.runtime().pointer_move(folder.0, folder.1);
        let listed = host.run_until(PATIENCE, |h| open(h) && h.app().peek.entries.len() == 2);
        assert!(listed, "the resting pointer opens the folder's peek");
        let to = centre_of(&host, "pictures");
        host.runtime().pointer_down(folder.0, folder.1);
        host.turn();
        assert!(open(&host), "pressing the selected folder keeps its peek");
        for step in 1..=6 {
            let t = step as f32 / 6.0;
            host.runtime().pointer_move(
                folder.0 + (to.0 - folder.0) * t,
                folder.1 + (to.1 - folder.1) * t,
            );
            host.turn();
        }
        assert!(crate::dnd::is_dragging(host.app()), "the drag started");
        assert!(!open(&host), "the drag closes the peek");
        host.runtime().pointer_up(to.0, to.1);
        while host.turn() {}
    });
}

#[test]
fn list_a_resting_pointer_peeks_and_a_drag_closes_it() {
    a_resting_pointer_peeks_and_a_drag_closes_it(
        "list",
        "list_a_resting_pointer_peeks_and_a_drag_closes_it",
    );
}

#[test]
fn icons_a_resting_pointer_peeks_and_a_drag_closes_it() {
    a_resting_pointer_peeks_and_a_drag_closes_it(
        "icons",
        "icons_a_resting_pointer_peeks_and_a_drag_closes_it",
    );
}

fn set_folder_peeking(host: &mut Offscreen, on: bool) {
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        shell::saved(app).change(|saved| saved.folder_peeking = on);
        shell::settle(app, tx);
    });
    while host.turn() {}
}

/// Pointer peeking follows "Folder peeking" live; a keyboard peek holds it on while it is
/// open, whatever the preference, and gives it back to the preference as it closes.
#[test]
fn pointer_peeking_follows_the_preference_and_a_keyboard_peek_holds_it_on() {
    let name = "pointer_peeking_follows_the_preference_and_a_keyboard_peek_holds_it_on";
    peeking("list", true, name, |mut host| {
        let _root = on_folder(&mut host, "documents");
        let enabled = |host: &Offscreen| host.app().peek.state.enabled();
        assert!(enabled(&host), "on with the preference");
        set_folder_peeking(&mut host, false);
        assert!(!enabled(&host), "off as the preference turns off");
        press(&mut host, Key::Text("i".to_owned()));
        assert!(open(&host) && enabled(&host), "a keyboard peek turns it on");
        press(&mut host, Key::Text("i".to_owned()));
        assert!(
            !open(&host) && !enabled(&host),
            "and its close gives it back"
        );
        set_folder_peeking(&mut host, true);
        assert!(enabled(&host), "on again with the preference");
    });
}

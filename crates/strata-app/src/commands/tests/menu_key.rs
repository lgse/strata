// SPDX-License-Identifier: MIT

//! The Menu key and Shift+F10 in each registered view: which menu opens, by where the cursor
//! is. A menu is named by what it acts on (`menus.rs`).

use rune::{Key, Mods, Tx};
use strata_core::model::Location;
use strata_desktop::keys::{Binding, Key as Named, KeyPress, Modifiers};

use super::super::{Action, Call, Outcome, Stage};
use super::routing::{at, note, noted, part};
use super::{MODULE, listed, press};
use crate::{
    App,
    host::offscreen::Offscreen,
    slots::Entry,
    test_support::{
        PATIENCE, folder_of,
        view::{browser_holds, drawn, hosted_over_in, open_menu, top_of_first, wheel},
    },
};

/// The Menu key, F10 and F9, with anything held.
fn a_menu_key_or_f9(press: &KeyPress) -> bool {
    [Named::Menu, Named::F10, Named::F9]
        .into_iter()
        .any(|key| press.is(Binding::plain(key).ignoring(Modifiers::all())))
}

fn reached_the_root(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    note("not taken by the view");
    Outcome::Declined
}

/// Says when the view left a menu key to go on: the root element asks this stage only for a
/// key the view's controller did not take, and Rune would make a secondary press of one.
const WATCH: &[Action<App>] = &[Action::new("reached-the-root", reached_the_root)
    .keys(&[at(Stage::Unclaimed, a_menu_key_or_f9)])];

fn the_views_and_a_watch() -> Vec<Entry<App>> {
    struct Watch;
    crate::parts_and(vec![part::<Watch>("watch", WATCH, |_| None)])
}

/// Nothing opened, and the view took the key.
fn assert_taken_and_nothing_opened(host: &Offscreen, after: &str) {
    assert_eq!(open_menu(host), None, "{after}");
    assert_eq!(
        noted(),
        Vec::<String>::new(),
        "{after}: the view left the key"
    );
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

/// Rune's one flag for Super, Hyper and Meta.
fn logo() -> Mods {
    Mods {
        logo: true,
        ..Mods::default()
    }
}

/// GTK's `open_focused_context_menu` (`src/ui/browser/view_filter.rs`): the cursor's entry
/// when it is selected, with the selection it is in, and the folder otherwise. A menu key
/// with anything else held opens nothing.
fn the_menu_keys_open_the_cursors_menu_or_the_folders(mode: &'static str, name: &str) {
    let name = format!("menu_key::{name}");
    hosted_over_in((MODULE, &name), mode, the_views_and_a_watch, |mut host| {
        let folder = listed(&mut host);
        let title = Location::local(folder.path()).display_name();
        press(&mut host, Key::Home, Mods::default());
        let (selected, cursor) = browser_holds(&host);
        assert_eq!(selected, ["archive"]);
        // The watch hears a key the view leaves.
        press(&mut host, Key::Function(9), Mods::default());
        assert_eq!(noted(), ["not taken by the view"]);

        for (key, mods) in [
            (Key::ContextMenu, Mods::default()),
            (Key::Function(10), shift()),
        ] {
            press(&mut host, key.clone(), mods);
            assert_eq!(open_menu(&host), cursor, "{key:?} on a selected cursor");
            assert_eq!(browser_holds(&host).0, ["archive"]);
            press(&mut host, Key::Escape, Mods::default());
            assert_eq!(open_menu(&host), None);
        }

        assert_eq!(noted(), Vec::<String>::new(), "the view took each key");

        press(&mut host, Key::ContextMenu, logo());
        assert_taken_and_nothing_opened(&host, "Super with the Menu key");
        press(&mut host, Key::ContextMenu, shift());
        assert_taken_and_nothing_opened(&host, "Shift with the Menu key");

        press(&mut host, Key::End, shift());
        let (selected, cursor) = browser_holds(&host);
        assert_eq!((selected.len(), cursor.as_deref()), (5, Some("todo.txt")));
        press(&mut host, Key::ContextMenu, Mods::default());
        assert_eq!(
            open_menu(&host),
            cursor,
            "a cursor in a selection of several"
        );
        assert_eq!(browser_holds(&host).0, selected, "which the menu keeps");
        press(&mut host, Key::Escape, Mods::default());

        // Escape empties the selection and leaves the cursor where it is.
        press(&mut host, Key::Escape, Mods::default());
        assert_eq!(browser_holds(&host), (Vec::new(), cursor));
        press(&mut host, Key::ContextMenu, Mods::default());
        assert_eq!(
            open_menu(&host),
            Some(title),
            "a cursor outside the selection"
        );
    });
}

/// A folder of 400 files, listed, with the cursor on the first and the first selected: what
/// the browser holds.
fn many_files(host: &mut Offscreen) -> (tempfile::TempDir, (Vec<String>, Option<String>)) {
    let names: Vec<String> = (0..400).map(|at| format!("file-{at:03}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let folder = folder_of(&names);
    host.app().open(Location::local(folder.path()));
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder's listing");
    while host.turn() {}
    press(host, Key::Home, Mods::default());
    let held = browser_holds(host);
    assert_eq!(held.1.as_deref(), Some("file-000"));
    (folder, held)
}

/// With the cursor's row scrolled out of view nothing opens and nothing moves, in every view:
/// what GTK's Columns and Icons views do. Its List view opens another entry's menu there
/// (finding lgse/strata#2235), which is not copied.
fn the_menu_key_opens_nothing_for_a_cursor_out_of_view(mode: &'static str, name: &str) {
    let name = format!("menu_key::out_of_view::{name}");
    hosted_over_in((MODULE, &name), mode, the_views_and_a_watch, |mut host| {
        let (_folder, held) = many_files(&mut host);
        // Far enough that the list no longer builds the cursor's row.
        wheel(&mut host, -240.0, 30);
        let gone = |host: &Offscreen| !drawn(host).contains(&"file-000".to_owned());
        assert!(host.run_until(PATIENCE, gone), "{:?}", drawn(&host).first());
        while host.turn() {}
        let shown = drawn(&host);

        for (key, mods) in [
            (Key::ContextMenu, Mods::default()),
            (Key::Function(10), shift()),
        ] {
            press(&mut host, key.clone(), mods);

            assert_taken_and_nothing_opened(&host, &format!("{key:?}"));
            assert_eq!(drawn(&host), shown, "{key:?}: the view scrolled");
            assert_eq!(browser_holds(&host), held, "{key:?}");
        }
    });
}

/// A row partly in view is one GTK has drawn, so its menu opens, at the row where it is.
fn the_menu_key_opens_the_menu_of_a_cursor_partly_in_view(mode: &'static str, name: &str) {
    let name = format!("menu_key::partly_in_view::{name}");
    hosted_over_in((MODULE, &name), mode, the_views_and_a_watch, |mut host| {
        let (_folder, held) = many_files(&mut host);
        let top = top_of_first(&host);
        // Less than any row's height.
        wheel(&mut host, -10.0, 1);
        let scrolled = host.run_until(PATIENCE, |host| top_of_first(host) != top);
        assert!(scrolled, "the wheel moved nothing");
        while host.turn() {}
        let shown = drawn(&host);

        press(&mut host, Key::ContextMenu, Mods::default());

        assert_eq!(open_menu(&host), held.1);
        assert_eq!(browser_holds(&host), held);
        // The rows are read again once the menu, which covers them, is gone.
        press(&mut host, Key::Escape, Mods::default());
        assert_eq!(drawn(&host), shown);
    });
}

/// GTK's `open_focused_context_menu` asks the cursor's own column: an entry of a column
/// after the first gets its menu as one of the first does.
#[test]
fn the_menu_key_opens_the_menu_of_an_entry_in_a_column_after_the_first() {
    let name = "menu_key::the_menu_key_opens_the_menu_of_an_entry_in_a_column_after_the_first";
    hosted_over_in(
        (MODULE, name),
        "columns",
        the_views_and_a_watch,
        |mut host| {
            let _folder = listed(&mut host);
            press(&mut host, Key::Home, Mods::default());
            press(&mut host, Key::Down, Mods::default());
            press(&mut host, Key::Right, Mods::default());
            let entered = |host: &Offscreen| browser_holds(host).1.as_deref() == Some("notes.txt");
            assert!(
                host.run_until(PATIENCE, entered),
                "{:?}",
                browser_holds(&host)
            );
            while host.turn() {}

            press(&mut host, Key::ContextMenu, Mods::default());

            assert_eq!(open_menu(&host).as_deref(), Some("notes.txt"));
            assert_eq!(noted(), Vec::<String>::new());
        },
    );
}

mod partly_in_view {
    use super::the_menu_key_opens_the_menu_of_a_cursor_partly_in_view;

    in_each_view! {
        the_menu_key_opens_the_menu_of_a_cursor_partly_in_view:
        "columns" => the_menu_key_opens_the_menu_of_a_cursor_partly_in_view_in_columns,
        "icons" => the_menu_key_opens_the_menu_of_a_cursor_partly_in_view_in_icons,
        "list" => the_menu_key_opens_the_menu_of_a_cursor_partly_in_view_in_list,
    }
}

mod out_of_view {
    use super::the_menu_key_opens_nothing_for_a_cursor_out_of_view;

    in_each_view! {
        the_menu_key_opens_nothing_for_a_cursor_out_of_view:
        "columns" => the_menu_key_opens_nothing_for_a_cursor_out_of_view_in_columns,
        "icons" => the_menu_key_opens_nothing_for_a_cursor_out_of_view_in_icons,
        "list" => the_menu_key_opens_nothing_for_a_cursor_out_of_view_in_list,
    }
}

in_each_view! {
    the_menu_keys_open_the_cursors_menu_or_the_folders:
    "columns" => the_menu_keys_open_the_cursors_menu_or_the_folders_in_columns,
    "icons" => the_menu_keys_open_the_cursors_menu_or_the_folders_in_icons,
    "list" => the_menu_keys_open_the_cursors_menu_or_the_folders_in_list,
}

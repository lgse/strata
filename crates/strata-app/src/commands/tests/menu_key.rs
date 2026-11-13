// SPDX-License-Identifier: MIT

//! The Menu key and Shift+F10 in each registered view: which menu opens, by where the cursor
//! is. A menu is named by what it acts on (`menus.rs`).

use rune::{Key, Mods};
use strata_core::model::Location;

use super::{MODULE, listed, press};
use crate::{
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, folder_of,
        view::{browser_holds, drawn, hosted_in, open_menu, wheel},
    },
};

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
    hosted_in(MODULE, mode, &name, |mut host| {
        let folder = listed(&mut host);
        let title = Location::local(folder.path()).display_name();
        press(&mut host, Key::Home, Mods::default());
        let (selected, cursor) = browser_holds(&host);
        assert_eq!(selected, ["archive"]);

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

        press(&mut host, Key::ContextMenu, logo());
        assert_eq!(open_menu(&host), None, "Super with the Menu key");
        press(&mut host, Key::ContextMenu, shift());
        assert_eq!(open_menu(&host), None, "Shift with the Menu key");

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

/// GTK opens an entry's menu at the row it has drawn, and has none for a row scrolled out of
/// view (`shown_row_at`, `item_widget`): nothing opens, and nothing moves.
fn the_menu_key_opens_nothing_for_a_cursor_out_of_view(mode: &'static str, name: &str) {
    let name = format!("menu_key::out_of_view::{name}");
    hosted_in(MODULE, mode, &name, |mut host| {
        let names: Vec<String> = (0..400).map(|at| format!("file-{at:03}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let folder = folder_of(&names);
        host.app().open(Location::local(folder.path()));
        let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
        assert!(host.run_until(PATIENCE, listed), "the folder's listing");
        while host.turn() {}
        press(&mut host, Key::Home, Mods::default());
        let held = browser_holds(&host);
        assert_eq!(held.1.as_deref(), Some("file-000"));
        // Far enough that the list no longer builds the cursor's row.
        wheel(&mut host, -240.0, 30);
        let shown = drawn(&host);
        assert!(
            !shown.contains(&"file-000".to_owned()),
            "{:?}",
            shown.first()
        );

        press(&mut host, Key::ContextMenu, Mods::default());

        assert_eq!(open_menu(&host), None);
        assert_eq!(drawn(&host), shown, "the view scrolled");
        assert_eq!(browser_holds(&host), held);
    });
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

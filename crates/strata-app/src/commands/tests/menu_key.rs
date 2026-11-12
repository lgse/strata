// SPDX-License-Identifier: MIT

//! The Menu key and Shift+F10 in each registered view: which menu opens, by where the cursor
//! is. A menu is named by what it acts on (`menus.rs`).

use rune::{Key, Mods};
use strata_core::model::Location;

use super::{MODULE, listed, press};
use crate::test_support::view::{browser_holds, hosted_in, open_menu};

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

#[test]
fn the_menu_keys_open_the_cursors_menu_or_the_folders_in_columns() {
    let name = "the_menu_keys_open_the_cursors_menu_or_the_folders_in_columns";
    the_menu_keys_open_the_cursors_menu_or_the_folders("columns", name);
}

#[test]
fn the_menu_keys_open_the_cursors_menu_or_the_folders_in_icons() {
    let name = "the_menu_keys_open_the_cursors_menu_or_the_folders_in_icons";
    the_menu_keys_open_the_cursors_menu_or_the_folders("icons", name);
}

#[test]
fn the_menu_keys_open_the_cursors_menu_or_the_folders_in_list() {
    let name = "the_menu_keys_open_the_cursors_menu_or_the_folders_in_list";
    the_menu_keys_open_the_cursors_menu_or_the_folders("list", name);
}

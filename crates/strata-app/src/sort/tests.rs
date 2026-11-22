// SPDX-License-Identifier: MIT

//! The Sort by menu on the offscreen host: a menu-kind plain popover's keys, as GTK's
//! `PopoverMenu` moved when driven (the module docs of `crate::popover`), and a Columns
//! column's menu, which hangs from that column's button and sorts that column only.

use rune::{Key, KeyEvent, Mods, Tx};
use serde_json::Value;
use strata_core::model::{Location, SortKey};

use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, folder_of, send_key,
        view::{self, hosted_in},
    },
};

const MODULE: &str = "sort::tests";

const NAME: &str = "sort-field-name";
const SIZE: &str = "sort-field-size";
const FOLDERS_FIRST: &str = "sort-folders-first";

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn node<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    view::all(tree, &|node| node["key"] == key)
        .into_iter()
        .next()
}

fn drawn(host: &Offscreen, key: &str) -> bool {
    node(&host.tree(), key).is_some()
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

fn click(host: &mut Offscreen, key: &str) {
    let elements = host.runtime().elements();
    let found = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = found.unwrap_or_else(|| panic!("{key} is drawn")).visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

/// A folder of three files of different sizes, listed and drawn.
fn listed(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = folder_of(&["a.txt", "bb.txt", "ccc.txt"]);
    host.app().open(Location::local(folder.path()));
    let drawn = |host: &Offscreen| view::drawn(host).contains(&"ccc.txt".to_owned());
    assert!(host.run_until(PATIENCE, drawn), "the folder's files");
    idle(host);
    folder
}

/// The sort key of the browser's column at `depth`.
fn sorted_by(host: &Offscreen, depth: usize) -> Option<SortKey> {
    let browser = host.app().browser();
    browser
        .column_preferences(depth)
        .map(|prefs| prefs.sort_key)
}

fn opened(host: &mut Offscreen, button: &str) {
    click(host, button);
    let open = host.run_until(PATIENCE, |host| drawn(host, NAME));
    assert!(open, "the menu opened");
    assert_eq!(
        focused(host).as_deref(),
        Some(NAME),
        "the menu opens on its first row"
    );
}

/// Up and Down wrap, Tab and Shift+Tab move as Down and Up, Home and End go to the ends, and
/// Return runs a row with the menu left open; Escape closes it to its button.
#[test]
fn the_menus_keys_wrap_and_a_row_leaves_it_open() {
    let name = "the_menus_keys_wrap_and_a_row_leaves_it_open";
    hosted_in(MODULE, "icons", name, |mut host| {
        let _folder = listed(&mut host);
        opened(&mut host, "icons-sort");
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        let plain = Mods::default();
        let steps = [
            (Key::Up, plain, FOLDERS_FIRST),
            (Key::Down, plain, NAME),
            (Key::Tab, plain, SIZE),
            (Key::Tab, shift, NAME),
            (Key::Tab, shift, FOLDERS_FIRST),
            (Key::Home, plain, NAME),
            (Key::End, plain, FOLDERS_FIRST),
            (Key::Home, plain, NAME),
            (Key::Down, plain, SIZE),
        ];
        for (key, mods, lands) in steps {
            press(&mut host, key.clone(), mods);
            assert_eq!(
                focused(&mut host).as_deref(),
                Some(lands),
                "{key:?} {mods:?}"
            );
        }
        press(&mut host, Key::Enter, plain);
        let sized = host.run_until(PATIENCE, |host| sorted_by(host, 0) == Some(SortKey::Size));
        assert!(sized, "Return on Size sorts by size");
        assert!(drawn(&host, SIZE), "the menu stays open after a row ran");
        assert_eq!(focused(&mut host).as_deref(), Some(SIZE));

        press(&mut host, Key::Escape, plain);
        let closed = host.run_until(PATIENCE, |host| !drawn(host, NAME));
        assert!(closed, "Escape closed the menu");
        assert_eq!(focused(&mut host).as_deref(), Some("icons-sort"));
    });
}

/// In Columns the menu hangs from the targeted column's own button, keyed by its depth, and
/// sorts that column only; closing it gives the view the keyboard, the header's buttons being
/// no focus stops.
#[test]
fn a_columns_menu_sorts_its_own_column() {
    let name = "a_columns_menu_sorts_its_own_column";
    hosted_in(MODULE, "columns", name, |mut host| {
        let folder = folder_of(&[]);
        let inner = folder.path().join("inner");
        std::fs::create_dir(&inner).expect("folder");
        for name in ["a.txt", "bb.txt", "ccc.txt"] {
            std::fs::write(inner.join(name), name).expect("file");
        }
        host.app().open(Location::local(folder.path()));
        let parent = |host: &Offscreen| view::drawn(host).contains(&"inner".to_owned());
        assert!(host.run_until(PATIENCE, parent), "the parent's folder");
        idle(&mut host);
        // Right enters the selected folder in a column of its own.
        press(&mut host, Key::Right, Mods::default());
        let entered = |host: &Offscreen| {
            host.app().browser().active_depth() == Some(1)
                && view::drawn(host).contains(&"ccc.txt".to_owned())
        };
        assert!(
            host.run_until(PATIENCE, entered),
            "the inner folder's column"
        );
        idle(&mut host);
        let depth = 1;

        opened(&mut host, &format!("columns-sort-{depth}"));
        press(&mut host, Key::Down, Mods::default());
        press(&mut host, Key::Enter, Mods::default());
        let sized = host.run_until(PATIENCE, |host| {
            sorted_by(host, depth) == Some(SortKey::Size)
        });
        assert!(sized, "Size sorts the column the menu opened from");
        assert_eq!(
            sorted_by(&host, depth - 1),
            Some(SortKey::Name),
            "its parent keeps its sort"
        );

        press(&mut host, Key::Escape, Mods::default());
        let closed = host.run_until(PATIENCE, |host| !drawn(host, NAME));
        assert!(closed, "Escape closed the menu");
        let mut view = None;
        host.runtime()
            .update(|app: &mut App, _: &mut Tx| view = shell::view_focus(app));
        idle(&mut host);
        assert_eq!(
            focused(&mut host).as_deref(),
            view,
            "the Columns view has the keyboard"
        );
    });
}

/// Activating Make default or Reset to default sort hides that row while leaving the menu open
/// with focus on its surface (`sort-menu`), so Down lands on the first row and Up on the last.
#[test]
fn hiding_make_default_or_reset_keeps_the_menu_open_and_focused() {
    let name = "hiding_make_default_or_reset_keeps_the_menu_open_and_focused";
    hosted_in(MODULE, "icons", name, |mut host| {
        let _folder = listed(&mut host);
        opened(&mut host, "icons-sort");

        click(&mut host, SIZE);
        assert!(host.run_until(PATIENCE, |host| drawn(host, "sort-reset")));
        click(&mut host, "sort-reset");
        assert!(host.run_until(PATIENCE, |host| !drawn(host, "sort-reset")));
        assert!(drawn(&host, NAME), "the menu stays open after Reset");
        assert_eq!(focused(&mut host).as_deref(), Some("sort-menu"));

        press(&mut host, Key::Down, Mods::default());
        assert_eq!(focused(&mut host).as_deref(), Some(NAME));

        click(&mut host, SIZE);
        assert!(host.run_until(PATIENCE, |host| drawn(host, "sort-make-default")));
        press(&mut host, Key::End, Mods::default());
        press(&mut host, Key::Up, Mods::default());
        assert_eq!(focused(&mut host).as_deref(), Some("sort-make-default"));
        press(&mut host, Key::Enter, Mods::default());
        assert!(host.run_until(PATIENCE, |host| !drawn(host, "sort-make-default")));
        assert_eq!(focused(&mut host).as_deref(), Some("sort-menu"));

        press(&mut host, Key::Up, Mods::default());
        assert_eq!(focused(&mut host).as_deref(), Some(FOLDERS_FIRST));
    });
}

/// Optional Duration, Dimensions and Created sorts show in the Sort menu, display a List
/// toolbar sort label while their column is hidden, and hide that label once the column is
/// shown so clicking the heading sorts and toggles direction.
#[test]
fn optional_column_sorts_follow_column_visibility_and_headings() {
    let name = "optional_column_sorts_follow_column_visibility_and_headings";
    hosted_in(MODULE, "list", name, |mut host| {
        let _folder = listed(&mut host);
        opened(&mut host, "list-sort");
        assert!(drawn(&host, "sort-field-duration"));
        assert!(drawn(&host, "sort-field-dimensions"));

        click(&mut host, "sort-field-duration");
        let sorted = host.run_until(PATIENCE, |host| {
            sorted_by(host, 0) == Some(SortKey::Duration)
        });
        assert!(sorted, "Duration sort activates from the Sort menu");
        press(&mut host, Key::Escape, Mods::default());

        let label_shown = host.run_until(PATIENCE, |host| {
            node(&host.tree(), "list-sort")
                .is_some_and(|button| view::text(button, "name") == "Sort: Duration ↑")
        });
        assert!(
            label_shown,
            "List shows Sort: Duration ↑ while the Duration column is hidden"
        );

        // Opt Duration in via preferences and resync the List columns: the toolbar label clears.
        host.runtime().update(|app: &mut App, _| {
            shell::saved(app).change(|settings| {
                settings.list_shown_columns =
                    vec![strata_desktop::settings::ListColumnId::Duration];
            });
            let prefs = shell::preferences(app).clone();
            app.list.sync_columns(&prefs, false);
        });
        idle(&mut host);
        let label_cleared = host.run_until(PATIENCE, |host| {
            node(&host.tree(), "list-sort")
                .is_some_and(|button| view::text(button, "name") == "Choose sort field")
        });
        assert!(
            label_cleared,
            "showing the Duration column clears the toolbar sort label"
        );

        // Clicking the visible Duration column heading flips the direction to Descending.
        let heading_key = "head-duration";
        assert!(
            host.run_until(PATIENCE, |host| drawn(host, heading_key)),
            "Duration heading is drawn once enabled"
        );
        click(&mut host, heading_key);
        let flipped = host.run_until(PATIENCE, |host| {
            host.app()
                .browser()
                .column_preferences(0)
                .is_some_and(|prefs| {
                    prefs.sort_key == SortKey::Duration
                        && prefs.sort_direction == strata_core::model::SortDirection::Descending
                })
        });
        assert!(flipped, "clicking Duration heading toggles Descending");
    });
}

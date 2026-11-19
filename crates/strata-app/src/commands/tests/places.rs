// SPDX-License-Identifier: MIT

//! What runs a command on a place, a location that is no row of the view, or on a folder row's
//! middle click: the sidebar's menu rows and its middle click, a crumb's middle click, and a
//! folder row's middle click in every view, each on its location (`On::Place`). `open-in-tab`
//! is the tab strip's own, so each opens a tab there, and `customize` the Customize dialog's,
//! which opens on it; `open-in-window` is A21's (`crate::new_window`), which asks the process
//! for a window on it, read here from the launch queue.

use std::{cell::RefCell, path::Path, time::Duration};

use rune::{Key, KeyEvent, Mods, PointerButton};
use strata_components::collection::element::row_key;
use strata_core::model::Location;

use crate::{
    App,
    application::{self, Opening},
    host::offscreen::Offscreen,
    shell::{
        self,
        tabs::{shown_tab, tab_browser, tabs},
    },
    slots::{Entry, TabKey},
    test_support::{
        PATIENCE, fixture, hosted_over,
        view::{all, browser_holds, hosted_over_in, idle, press as press_key, text},
    },
};

const MODULE: &str = "commands::tests::places";

thread_local! {
    /// Each window `open-in-window` asked for so far, and the place it asked for.
    static RAN: RefCell<Vec<(&'static str, Location)>> = const { RefCell::new(Vec::new()) };
}

/// The registered parts: every command a place's routes name is one of theirs.
fn with_the_commands() -> Vec<Entry<App>> {
    crate::parts_and(Vec::new())
}

fn ran_so_far() -> Vec<(&'static str, Location)> {
    RAN.with_borrow_mut(|ran| {
        for opening in application::take() {
            if let Opening::Argument(location) = opening {
                ran.push(("open-in-window", location));
            }
        }
        ran.clone()
    })
}

fn home() -> Location {
    Location::local(std::env::var_os("HOME").expect("the test's home"))
}

/// The middle of the element keyed `key`, once it is drawn.
fn middle(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let drawn = host.run_until(PATIENCE, |host| {
        let tree = host.tree();
        !all(&tree, &|node| node["key"] == key).is_empty()
    });
    assert!(drawn, "{key} is drawn");
    let elements = host.runtime().elements();
    let element = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = element.expect("the element is mounted").visible;
    (x + w / 2.0, y + h / 2.0)
}

fn press(host: &mut Offscreen, key: &str, button: PointerButton, mods: Mods) {
    let (x, y) = middle(host, key);
    host.modifiers(mods);
    host.turn();
    let runtime = host.runtime();
    runtime.pointer_move(x, y);
    match button {
        PointerButton::Primary => runtime.pointer_down(x, y),
        PointerButton::Secondary => runtime.secondary_down(x, y),
        _ => runtime.button_down(x, y, button),
    }
    host.turn();
    let runtime = host.runtime();
    match button {
        PointerButton::Primary => runtime.pointer_up(x, y),
        PointerButton::Secondary => runtime.secondary_up(x, y),
        _ => runtime.button_up(x, y, button),
    }
    // A press is heard at its release, with the modifiers held then.
    host.turn();
    host.modifiers(Mods::default());
    idle(host);
}

fn opened(host: &mut Offscreen, folder: &Path) {
    let wanted = Location::local(folder);
    host.app().open(wanted.clone());
    let reached = host.run_until(PATIENCE, |host| {
        host.app().browser().active_location() == Some(wanted.clone())
    });
    assert!(reached, "the window opens {}", folder.display());
    idle(host);
}

/// A tab opened at the end, shown, at `at`, while tab `first` stays at `stays`.
fn opened_in_a_tab(
    host: &mut Offscreen,
    count: usize,
    at: &Location,
    (first, stays): (TabKey, &Location),
) {
    let order = tabs(host.app());
    assert_eq!(order.len(), count, "a tab for each");
    assert_eq!(
        order.last(),
        Some(&shown_tab(host.app())),
        "the new tab, shown"
    );
    let reached = host.run_until(PATIENCE, |host| {
        host.app().browser().active_location().as_ref() == Some(at)
    });
    assert!(reached, "the new tab opens {at:?}");
    let first = tab_browser(host.app(), first).expect("the first tab");
    assert_eq!(
        first.active_location().as_ref(),
        Some(stays),
        "the first tab stays"
    );
}

/// The key of the open menu's row named `name`.
fn menu_row(host: &mut Offscreen, name: &str) -> String {
    let row = |host: &Offscreen| {
        let tree = host.tree();
        let rows = all(&tree, &|node| {
            text(node, "role").starts_with("menu-item") && text(node, "name") == name
        });
        rows.first().map(|row| text(row, "key"))
    };
    assert!(
        host.run_until(PATIENCE, |host| row(host).is_some()),
        "the menu lists {name}"
    );
    row(host).expect("the row")
}

/// A middle click on a sidebar row opens its location in a new tab, as its menu's "Open in New
/// Tab" does, and its menu's other rows run their commands on the location the menu was opened
/// on.
#[test]
fn a_sidebar_rows_middle_click_and_menu_rows_run_on_its_location() {
    hosted_over(
        MODULE,
        "a_sidebar_rows_middle_click_and_menu_rows_run_on_its_location",
        with_the_commands,
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let first = (shown_tab(host.app()), &Location::local(folder.path()));
            press(
                &mut host,
                "sidebar-place-home",
                PointerButton::Middle,
                Mods::default(),
            );
            opened_in_a_tab(&mut host, 2, &home(), first);
            press(
                &mut host,
                "sidebar-place-home",
                PointerButton::Secondary,
                Mods::default(),
            );
            let row = menu_row(&mut host, "Open in New Tab");
            press(&mut host, &row, PointerButton::Primary, Mods::default());
            opened_in_a_tab(&mut host, 3, &home(), first);

            press(
                &mut host,
                "sidebar-place-home",
                PointerButton::Secondary,
                Mods::default(),
            );
            let row = menu_row(&mut host, "Customize…");
            press(&mut host, &row, PointerButton::Primary, Mods::default());
            assert_eq!(
                crate::customize::customizing(host.app()),
                home().native_path(),
                "Customize opens on the row's folder"
            );
        },
    );
}

/// The key of the crumb that names `folder`.
fn crumb_of(host: &Offscreen, folder: &Path) -> String {
    let name = folder
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    let tree = host.tree();
    let crumbs = all(&tree, &|node| {
        text(node, "key").starts_with("location-crumb-") && text(node, "name") == name
    });
    text(crumbs.first().expect("the folder's crumb"), "key")
}

/// A crumb's middle click opens its folder in a new tab, and with Ctrl or Shift runs
/// `open-in-window`, as GTK's crumbs do; the tab it was pressed in stays where it is.
#[test]
fn a_crumbs_middle_click_runs_open_in_tab_or_with_a_modifier_in_window() {
    hosted_over(
        MODULE,
        "a_crumbs_middle_click_runs_open_in_tab_or_with_a_modifier_in_window",
        with_the_commands,
        |mut host| {
            let folder = fixture();
            let documents = folder.path().join("documents");
            opened(&mut host, &documents);
            let first = (shown_tab(host.app()), &Location::local(&documents));
            let root = Location::local(folder.path());
            let crumb = crumb_of(&host, folder.path());
            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            for mods in [ctrl, shift] {
                press(&mut host, &crumb, PointerButton::Middle, mods);
            }
            assert_eq!(
                ran_so_far(),
                [
                    ("open-in-window", root.clone()),
                    ("open-in-window", root.clone())
                ]
            );
            assert_eq!(tabs(host.app()).len(), 1, "no tab for a new window");

            press(&mut host, &crumb, PointerButton::Middle, Mods::default());
            opened_in_a_tab(&mut host, 2, &root, first);
        },
    );
}

/// Two rapid middle clicks on `key` with `mods` held: only the first click of the run counts.
fn double_middle_press(host: &mut Offscreen, key: &str, mods: Mods) {
    let (x, y) = middle(host, key);
    host.modifiers(mods);
    host.turn();
    host.runtime().pointer_move(x, y);
    for _ in 0..2 {
        host.runtime().button_down(x, y, PointerButton::Middle);
        host.turn();
        host.runtime().button_up(x, y, PointerButton::Middle);
        host.turn();
    }
    host.modifiers(Mods::default());
    idle(host);
}

/// A folder row's middle click opens a tab, or a window with Ctrl or Shift, once per click run;
/// a file row's middle click selects the file and opens nothing.
fn middle_click_in_view(mode: &'static str, name: &'static str) {
    hosted_over_in((MODULE, name), mode, with_the_commands, |mut host| {
        let folder = fixture();
        opened(&mut host, folder.path());
        let root = Location::local(folder.path());
        let first = (shown_tab(host.app()), &root);
        let documents = Location::local(folder.path().join("documents"));
        let todo = Location::local(folder.path().join("todo.txt"));
        let folder_row = row_key(shell::id_of(host.app(), &documents));
        let file_row = row_key(shell::id_of(host.app(), &todo));
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };

        double_middle_press(&mut host, &folder_row, ctrl);
        assert_eq!(ran_so_far(), [("open-in-window", documents.clone())]);
        host.run_until(Duration::from_millis(600), |_| false);
        press(&mut host, &folder_row, PointerButton::Middle, shift);
        assert_eq!(
            ran_so_far(),
            [
                ("open-in-window", documents.clone()),
                ("open-in-window", documents.clone()),
            ]
        );
        assert_eq!(tabs(host.app()).len(), 1, "no tab for a new window");

        press(&mut host, &file_row, PointerButton::Middle, Mods::default());
        assert_eq!(browser_holds(&host).0, ["todo.txt"]);
        assert_eq!(tabs(host.app()).len(), 1, "a file opens no tab");

        press(
            &mut host,
            &folder_row,
            PointerButton::Middle,
            Mods::default(),
        );
        opened_in_a_tab(&mut host, 2, &documents, first);
    });
}

in_each_view! {
    middle_click_in_view:
    "columns" => a_folder_rows_middle_click_opens_in_columns,
    "list" => a_folder_rows_middle_click_opens_in_list,
    "icons" => a_folder_rows_middle_click_opens_in_icons,
}

fn filter_in_columns(host: &mut Offscreen, query: &str) {
    let view = shell::view_focus(host.app()).expect("the view's key");
    host.runtime().update(|_, tx| tx.focus(view, true));
    idle(host);
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    press_key(host, Key::Text("f".to_owned()), ctrl);
    for ch in query.chars() {
        host.runtime().key(KeyEvent {
            key: Key::Text(ch.to_string()),
            mods: Mods::default(),
        });
    }
    idle(host);
}

/// In a filtered Columns column (`RowContext::view_entry` in GTK), a middle click on a folder
/// hit from a subfolder opens that subfolder in a new tab.
#[test]
fn a_filtered_columns_folder_middle_click_opens_a_new_tab() {
    hosted_over(
        MODULE,
        "a_filtered_columns_folder_middle_click_opens_a_new_tab",
        with_the_commands,
        |mut host| {
            let folder = fixture();
            let drafts_path = folder.path().join("documents/drafts");
            std::fs::create_dir(&drafts_path).expect("subfolder");
            opened(&mut host, folder.path());
            let root = Location::local(folder.path());
            let first = (shown_tab(host.app()), &root);
            let drafts = Location::local(drafts_path);

            filter_in_columns(&mut host, "drafts");

            let hit_row = row_key(shell::id_of(host.app(), &drafts));
            press(&mut host, &hit_row, PointerButton::Middle, Mods::default());
            opened_in_a_tab(&mut host, 2, &drafts, first);
        },
    );
}

/// In Columns with a deeper column open (`open_middle_clicked` -> `MiddleClickAction::RevealParent`
/// in `src/ui/browser/columns/rows/click.rs`), middle-clicking a file in an ancestor column
/// activates that column, selects and focuses the file, and closes the deeper column.
#[test]
fn a_columns_file_middle_click_activates_its_column_and_closes_deeper_ones() {
    hosted_over(
        MODULE,
        "a_columns_file_middle_click_activates_its_column_and_closes_deeper_ones",
        with_the_commands,
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let documents = Location::local(folder.path().join("documents"));
            let todo = Location::local(folder.path().join("todo.txt"));

            host.runtime().update(|app, tx| {
                assert!(
                    app.browser
                        .select_entries_by_name_at(0, &["documents".into()])
                );
                app.browser.enter_focused_directory();
                shell::settle(app, tx);
            });
            let deeper = host.run_until(PATIENCE, |host| {
                host.app().browser.location_at(1).as_ref() == Some(&documents)
                    && host.app().columns.listed(1)
            });
            assert!(deeper, "documents opens in column 1");
            idle(&mut host);
            assert_eq!(host.app().browser.active_depth(), Some(1));

            let file_row = row_key(shell::id_of(host.app(), &todo));
            press(&mut host, &file_row, PointerButton::Middle, Mods::default());

            assert_eq!(host.app().browser.active_depth(), Some(0));
            assert_eq!(host.app().browser.location_at(1), None);
            assert_eq!(browser_holds(&host).0, ["todo.txt"]);
            assert_eq!(browser_holds(&host).1.as_deref(), Some("todo.txt"));
            assert_eq!(tabs(host.app()).len(), 1);
        },
    );
}

/// In filtered Columns (`open_middle_clicked` -> `state.reveal_locations`), middle-clicking a
/// same-folder file hit clears the filter and selects the file in place, and middle-clicking a
/// subfolder file hit navigates to its parent folder and reveals the file there.
#[test]
fn a_filtered_columns_file_middle_click_reveals_in_its_folder() {
    hosted_over(
        MODULE,
        "a_filtered_columns_file_middle_click_reveals_in_its_folder",
        with_the_commands,
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let root = Location::local(folder.path());
            let documents = Location::local(folder.path().join("documents"));
            let todo = Location::local(folder.path().join("todo.txt"));
            let notes = Location::local(folder.path().join("documents/notes.txt"));

            host.runtime().update(|app, tx| {
                assert!(
                    app.browser
                        .select_entries_by_name_at(0, &["documents".into()])
                );
                shell::settle(app, tx);
            });
            idle(&mut host);

            filter_in_columns(&mut host, "todo");
            assert!(host.app().columns.shows_hits(0));
            let todo_row = row_key(shell::id_of(host.app(), &todo));
            press(&mut host, &todo_row, PointerButton::Middle, Mods::default());

            assert!(!host.app().columns.shows_hits(0), "filter hits cleared");
            assert_eq!(host.app().browser.location_at(0).as_ref(), Some(&root));
            assert_eq!(browser_holds(&host).0, ["todo.txt"]);
            assert_eq!(browser_holds(&host).1.as_deref(), Some("todo.txt"));
            host.runtime()
                .update(|_, tx| assert_eq!(tx.focused(), Some("columns")));
            assert_eq!(tabs(host.app()).len(), 1);

            filter_in_columns(&mut host, "notes");
            assert!(host.app().columns.shows_hits(0));
            let notes_row = row_key(shell::id_of(host.app(), &notes));
            press(
                &mut host,
                &notes_row,
                PointerButton::Middle,
                Mods::default(),
            );

            let revealed = host.run_until(PATIENCE, |host| {
                host.app().browser.active_location().as_ref() == Some(&documents)
                    && host.app().columns.listed(0)
            });
            assert!(revealed, "navigates to documents");
            idle(&mut host);
            assert!(!host.app().columns.shows_hits(0));
            assert_eq!(browser_holds(&host).0, ["notes.txt"]);
            assert_eq!(browser_holds(&host).1.as_deref(), Some("notes.txt"));
            assert_eq!(tabs(host.app()).len(), 1);
        },
    );
}

// SPDX-License-Identifier: MIT

//! The strip on the registered window, where GTK was driven to say what it does: a tab hidden
//! with a rename open commits it, a child column its mirror was about to open is open when the
//! tab comes back, in that tab only, and a filter's field has the keyboard again in a pane,
//! where a Columns column's filter closes. A tab or a window with a file operation running is
//! not closed, and GTK's notice says so. Folders open in new tabs from a menu's row and from
//! Ctrl+Enter.

use std::{path::Path, time::Duration};

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use strata_platform::csd::Outcome as Sent;

use super::select;
use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    shell::{
        self,
        tabs::{shown_tab, tab_browser, tabs},
    },
    slots::TabKey,
    test_support::{
        PATIENCE, fixture, hosted, send_key,
        view::{all, field, hosted_in, idle, press, text},
    },
};

mod badges;
mod held;

const MODULE: &str = "tab_strip::tests";

pub(super) fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

/// The window opened at `folder`, listed.
pub(super) fn opened(host: &mut Offscreen, folder: &Path) {
    host.app().open(Location::local(folder));
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder's listing");
    idle(host);
}

/// The tabs' locations, in the strip's order, once each has opened.
fn located(host: &mut Offscreen, count: usize) -> Vec<Option<Location>> {
    let all_there = |host: &Offscreen| {
        let order = tabs(host.app());
        order.len() == count
            && order.iter().all(|&key| {
                tab_browser(host.app(), key).is_some_and(|tab| tab.active_location().is_some())
            })
    };
    assert!(
        host.run_until(PATIENCE, all_there),
        "{count} tabs, each open"
    );
    let app = host.app();
    let order = tabs(app);
    let at = |key: &TabKey| tab_browser(app, *key).and_then(|tab| tab.active_location());
    order.iter().map(at).collect()
}

pub(super) fn act<T>(host: &mut Offscreen, act: impl FnOnce(&mut App, &mut Tx) -> T) -> T {
    let mut said = None;
    host.runtime().update(|app, tx| said = Some(act(app, tx)));
    idle(host);
    said.expect("the update ran")
}

/// The window at `folder`, and a second tab opened at it with the header's New tab, shown.
/// Returns the first tab.
pub(super) fn two_tabs_at(host: &mut Offscreen, folder: &Path) -> TabKey {
    let at = Location::local(folder);
    host.app().open(at.clone());
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder's listing");
    idle(host);
    let first = shown_tab(host.app());
    let ran = act(host, |app, tx| run(app, "tab-new", On::Focus, tx));
    assert_eq!(ran, Outcome::Ran);
    assert_eq!(tabs(host.app()).len(), 2, "New tab opened a tab");
    let opened = |host: &Offscreen| {
        host.app().browser().active_location() == Some(at.clone())
            && host.app().browser().column_entry_counts(0).is_some()
    };
    assert!(
        host.run_until(PATIENCE, opened),
        "the new tab at the same folder"
    );
    idle(host);
    first
}

fn typed(host: &mut Offscreen, text: &str) {
    for character in text.chars() {
        host.runtime().key(KeyEvent {
            key: Key::Text(character.to_string()),
            mods: Mods::default(),
        });
    }
    idle(host);
}

/// GTK's rename field commits when the keyboard leaves it, and a tab switch takes the
/// keyboard: the name typed is the entry's, and the tab shown again has no edit open.
#[test]
fn a_rename_open_when_its_tab_is_hidden_is_committed() {
    hosted(
        MODULE,
        "a_rename_open_when_its_tab_is_hidden_is_committed",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            let second = shown_tab(host.app());
            let todo = Location::local(folder.path().join("todo.txt"));
            act(&mut host, |app, tx| {
                let row = On::Row((0, shell::id_of(app, &todo)));
                assert_eq!(run(app, "rename", row, tx), Outcome::Ran);
            });
            assert!(field(&host).is_some(), "the editor opened");
            typed(&mut host, "zz");

            assert!(act(&mut host, |app, tx| select(app, first, tx)));
            let renamed = folder.path().join("zz.txt");
            let committed = host.run_until(PATIENCE, |_| renamed.exists());
            assert!(committed, "the name typed was committed");
            assert!(!folder.path().join("todo.txt").exists());

            assert!(act(&mut host, |app, tx| select(app, second, tx)));
            assert_eq!(field(&host), None, "the tab shown again has no edit open");
        },
    );
}

/// A cursor moved onto a folder starts the Columns mirror. A tab hidden while it waits opens
/// no column in the tab shown, and has the folder's column open when it is shown again.
#[test]
fn a_mirror_due_in_a_hidden_tab_opens_its_column_in_that_tab_alone() {
    hosted(
        MODULE,
        "a_mirror_due_in_a_hidden_tab_opens_its_column_in_that_tab_alone",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            let second = shown_tab(host.app());
            let documents = Location::local(folder.path().join("documents"));
            assert_eq!(
                host.app().browser().location_at(1),
                None,
                "no child column yet"
            );

            let focus = shell::view_focus(host.app()).expect("the view's key");
            act(&mut host, |_, tx| tx.focus(focus, true));
            // The switch comes before the mirror's wait is over, or just after it.
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Down,
                    mods: Mods::default(),
                },
            );
            // One frame hands the key to the view and its cursor to the browser.
            host.turn();
            let moved = host
                .app()
                .browser()
                .focused_item()
                .map(|(depth, at, _)| (depth, at));
            assert_eq!(moved, Some((0, 1)), "Down moved the cursor onto documents");
            assert!(act(&mut host, |app, tx| select(app, first, tx)));
            host.run_until(Duration::from_millis(400), |_| false);
            let shown = host.app().browser().location_at(1);
            assert_eq!(shown, None, "the tab shown opened no column");

            assert!(act(&mut host, |app, tx| select(app, second, tx)));
            let opened =
                |host: &Offscreen| host.app().browser().location_at(1) == Some(documents.clone());
            let mirrored = host.run_until(PATIENCE, opened);
            assert!(mirrored, "the mirror opened documents in its own tab");
        },
    );
}

/// The entry menu's Open In → New Tab on a row of a selection: a tab for each folder the
/// selection holds, in its order, the last shown; the file in it opens none.
#[test]
fn a_menu_row_opens_each_folder_it_acts_on_in_a_tab() {
    hosted(
        MODULE,
        "a_menu_row_opens_each_folder_it_acts_on_in_a_tab",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let at = |name: &str| Location::local(folder.path().join(name));
            let names = ["documents", "pictures", "readme.md"].map(str::to_owned);
            act(&mut host, |app, _| {
                app.browser.select_entries_by_name_at(0, &names);
            });
            let ran = act(&mut host, |app, tx| {
                let row = On::Row((0, shell::id_of(app, &at("documents"))));
                run(app, "open-in-tab", row, tx)
            });
            assert_eq!(ran, Outcome::Ran);
            let first = Location::local(folder.path());
            assert_eq!(
                located(&mut host, 3),
                [Some(first), Some(at("documents")), Some(at("pictures"))]
            );
            assert_eq!(Some(&shown_tab(host.app())), tabs(host.app()).last());
        },
    );
}

/// Ctrl+Enter in the listing opens the cursor's folder in a new tab, shown; the tab it was
/// pressed in stays.
#[test]
fn ctrl_enter_opens_the_cursors_folder_in_a_new_tab() {
    hosted(
        MODULE,
        "ctrl_enter_opens_the_cursors_folder_in_a_new_tab",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let view = shell::view_focus(host.app()).expect("the view's key");
            act(&mut host, |_, tx| tx.focus(view, true));
            press(&mut host, Key::Down, Mods::default());
            press(&mut host, Key::Enter, ctrl());
            let documents = Location::local(folder.path().join("documents"));
            let first = Location::local(folder.path());
            assert_eq!(located(&mut host, 2), [Some(first), Some(documents)]);
            assert_eq!(Some(&shown_tab(host.app())), tabs(host.app()).last());
        },
    );
}

/// The name of the dialog shown, if one is.
pub(super) fn dialog(host: &Offscreen) -> Option<String> {
    let tree = host.tree();
    let dialogs = all(&tree, &|node| node["role"] == "dialog");
    dialogs.first().map(|dialog| text(dialog, "name"))
}

pub(super) const OPERATIONS_ACTIVE: &str = "File operations are still active";

/// GTK's `close` refuses a tab whose browser has a file operation running, and its notice
/// says so: the tab stays.
#[test]
fn a_tab_with_an_operation_running_stays_open_and_says_so() {
    hosted(
        MODULE,
        "a_tab_with_an_operation_running_stays_open_and_says_so",
        |mut host| {
            let folder = fixture();
            two_tabs_at(&mut host, folder.path());
            act(&mut host, |app, _| held::busy(&app.browser, "todo.txt"));
            act(&mut host, |app, tx| run(app, "tab-close", On::Focus, tx));
            let said = host.run_until(PATIENCE, |host| dialog(host).is_some());
            assert!(said, "a notice");
            assert_eq!(dialog(&host).as_deref(), Some(OPERATIONS_ACTIVE));
            assert_eq!(tabs(host.app()).len(), 2, "the tab stays");
        },
    );
}

/// GTK's close guard: the window's Close is refused while any tab, shown or hidden, has a
/// file operation running, with the same notice; no close is sent.
#[test]
fn the_window_is_not_closed_while_a_hidden_tab_has_an_operation_running() {
    hosted(
        MODULE,
        "the_window_is_not_closed_while_a_hidden_tab_has_an_operation_running",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            act(&mut host, |app, _| {
                let browser = tab_browser(app, first).expect("the first tab");
                held::busy(browser, "todo.txt");
            });
            click_key(&mut host, "window-close");
            let said = host.run_until(PATIENCE, |host| dialog(host).is_some());
            assert!(said, "a notice");
            assert_eq!(dialog(&host).as_deref(), Some(OPERATIONS_ACTIVE));
            let sent = act(&mut host, |app, _| {
                shell::csd_of(app).last_outcome().cloned()
            });
            assert!(
                !matches!(sent, Some(Sent::Sent { .. })),
                "no close was sent: {sent:?}"
            );
        },
    );
}

/// Presses the element keyed `key` with the pointer.
fn click_key(host: &mut Offscreen, key: &str) {
    let elements = host.runtime().elements();
    let element = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = element.expect("the element is mounted").visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    let runtime = host.runtime();
    runtime.pointer_move(x, y);
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    idle(host);
}

/// A query typed in the filter, a new tab opened, and the first tab shown again: the filter's
/// field is then `back`, as `field` reads it.
fn filter_across_a_switch(host: &mut Offscreen, folder: &Path, back: Option<(String, bool)>) {
    opened(host, folder);
    let view = shell::view_focus(host.app()).expect("the view's key");
    act(host, |_, tx| tx.focus(view, true));
    press(host, Key::Text("f".to_owned()), ctrl());
    typed(host, "photo");
    let typing = |host: &Offscreen| field(host) == Some(("photo".to_owned(), true));
    assert!(host.run_until(PATIENCE, typing), "the query in the field");
    let first = shown_tab(host.app());
    act(host, |app, tx| run(app, "tab-new", On::Focus, tx));
    assert_eq!(tabs(host.app()).len(), 2);
    assert!(act(host, |app, tx| select(app, first, tx)));
    let reached = host.run_until(PATIENCE, |host| field(host) == back);
    assert!(reached, "the field is {:?}, not {back:?}", field(host));
}

/// Driven on GTK: in List a tab comes back with the keyboard in its filter's field, query
/// kept.
#[test]
fn a_tab_left_from_a_panes_filter_field_gets_the_field_back() {
    hosted_in(
        MODULE,
        "list",
        "a_tab_left_from_a_panes_filter_field_gets_the_field_back",
        |mut host| {
            let folder = fixture();
            let back = Some(("photo".to_owned(), true));
            filter_across_a_switch(&mut host, folder.path(), back);
        },
    );
}

/// Driven on GTK: a Columns column's filter closes as the keyboard leaves the column for
/// another tab, so the tab comes back with none.
#[test]
fn a_tab_left_from_a_columns_filter_comes_back_with_it_closed() {
    hosted_in(
        MODULE,
        "columns",
        "a_tab_left_from_a_columns_filter_comes_back_with_it_closed",
        |mut host| {
            let folder = fixture();
            filter_across_a_switch(&mut host, folder.path(), None);
        },
    );
}

/// Whether the element keyed `key` is drawn inside the shown tab's content (`tab-<n>`).
fn drawn_in_shown_tab(host: &mut Offscreen, key: &str) -> bool {
    let tab = format!("tab-{}", shown_tab(host.app()).0);
    let elements = host.runtime().elements();
    let in_tab = |mut at: Option<usize>| {
        while let Some(index) = at {
            let element = &elements[index];
            if element.key.as_deref() == Some(tab.as_str()) {
                return !element.exiting;
            }
            at = element.parent;
        }
        false
    };
    let drawn =
        |element: &&rune::ElementInfo| !element.exiting && element.key.as_deref() == Some(key);
    elements
        .iter()
        .filter(drawn)
        .any(|element| in_tab(element.parent))
}

/// A new tab's first frame has its sidebar: a frame without it would lay the files out where
/// the sidebar goes, and their move back is a layout Rune may not report to the view
/// (finding lgse/strata#2477).
#[test]
fn a_new_tabs_first_frame_has_its_sidebar() {
    hosted(
        MODULE,
        "a_new_tabs_first_frame_has_its_sidebar",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            assert!(drawn_in_shown_tab(&mut host, "sidebar"), "the first tab's");
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Text("t".to_owned()),
                    mods: ctrl(),
                },
            );
            host.turn();
            assert_eq!(tabs(host.app()).len(), 2, "Ctrl+T opened a tab");
            assert!(drawn_in_shown_tab(&mut host, "sidebar"), "the new tab's");
        },
    );
}

/// What has the keyboard, as an update reads it.
fn keyboard(host: &mut Offscreen) -> Option<String> {
    act(host, |_, tx| tx.focused().map(str::to_owned))
}

/// Presses the strip's tab at `index` with the pointer.
fn click_tab(host: &mut Offscreen, index: usize) {
    let tree = host.tree();
    let tabs = crate::test_support::view::all(&tree, &|node| node["role"] == "tab");
    let bounds = &tabs.get(index).expect("the tab")["bounds"];
    let at = |i: usize| bounds[i].as_f64().expect("a bound");
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let (x, y) = ((at(0) + at(2) / 2.0) as f32, (at(1) + at(3) / 2.0) as f32);
    let runtime = host.runtime();
    runtime.pointer_move(x, y);
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    idle(host);
}

/// GTK's `select` keeps a tab's focus only in its files, its sidebar or its preview: a tab
/// left by a press on the strip, which took the keyboard, gets its view's back when it is
/// shown again.
#[test]
fn a_tab_left_by_a_press_on_the_strip_gets_the_keyboard_back_in_its_view() {
    hosted(
        MODULE,
        "a_tab_left_by_a_press_on_the_strip_gets_the_keyboard_back_in_its_view",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            let second = shown_tab(host.app());
            let view = shell::view_focus(host.app()).expect("the view's key");
            assert_eq!(keyboard(&mut host).as_deref(), Some(view));

            click_tab(&mut host, 0);
            assert_eq!(
                shown_tab(host.app()),
                first,
                "the press showed the first tab"
            );
            assert!(act(&mut host, |app, tx| select(app, second, tx)));
            assert_eq!(keyboard(&mut host).as_deref(), Some(view));
            assert!(act(&mut host, |app, tx| select(app, first, tx)));
            assert_eq!(
                keyboard(&mut host).as_deref(),
                Some(view),
                "the first tab's view"
            );
        },
    );
}

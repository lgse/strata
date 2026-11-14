// SPDX-License-Identifier: MIT

//! The search palette in a window: Ctrl+K opens it on Home with its field typing, a result's
//! menu holds the keyboard and gives it back to the field, a middle click opens nothing, and
//! the header's Search button closes it again. Ctrl+Shift+K opens it on the folders visited.

use rune::{Key, Mods, PointerButton, Tx};

use crate::{
    App,
    commands::{Holds, Situation},
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, fixture,
        view::{self, hosted_in, idle, press},
    },
};

const MODULE: &str = "palette::tests";

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

/// Three folders under Home whose names start `nav-`, and the fixture opened in Columns.
fn ready(host: &mut Offscreen) -> tempfile::TempDir {
    let home = crate::launch::home_directory();
    for name in ["nav-alpha", "nav-beta", "nav-gamma"] {
        std::fs::create_dir_all(home.join(name)).expect("a folder under Home");
    }
    let folder = fixture();
    host.app()
        .open(strata_core::model::Location::local(folder.path()));
    let listed = host.run_until(PATIENCE, |host| host.app().columns.listed(0));
    assert!(listed, "the fixture was not listed");
    folder
}

/// Ctrl+K, then `query` typed, and its three results.
fn searched(host: &mut Offscreen, query: &str) {
    press(host, Key::Text("k".to_owned()), ctrl());
    assert!(
        host.app().palette.state.is_open(),
        "Ctrl+K opened the palette"
    );
    for character in query.chars() {
        press(host, Key::Text(character.to_string()), Mods::default());
    }
    // Every result, and the index read to its end: no later batch reorders them.
    let found = host.run_until(PATIENCE, |host| {
        let state = &host.app().palette.state;
        state.items().len() == 3 && !state.busy()
    });
    assert!(
        found,
        "the three results did not come: {:?}",
        host.app().palette.state.items()
    );
    idle(host);
}

fn focused(host: &mut Offscreen) -> Option<String> {
    let mut said = None;
    host.runtime()
        .update(|_: &mut App, tx: &mut Tx| said = tx.focused().map(str::to_owned));
    idle(host);
    said
}

fn holds(host: &mut Offscreen) -> Option<Holds> {
    let mut held = None;
    host.runtime()
        .update(|app: &mut App, tx: &mut Tx| held = Situation::of(app, tx).holds());
    held
}

/// The middle of the element keyed `key`, in window points.
fn middle_of(host: &Offscreen, key: &str) -> (f32, f32) {
    let element = host
        .elements()
        .into_iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no element {key}"));
    let [x, y, w, h] = element.visible;
    (x + w / 2.0, y + h / 2.0)
}

fn highlighted(host: &Offscreen) -> Option<String> {
    let state = &host.app().palette.state;
    state.highlighted_item().map(|item| item.id.clone())
}

/// Ctrl+K opens the palette with its field typing, the window's modal layer; a query lists
/// what Home holds; Escape closes it and gives the keyboard back to the view.
#[test]
fn ctrl_k_searches_home_and_escape_closes_it() {
    hosted_in(
        MODULE,
        "columns",
        "ctrl_k_searches_home_and_escape_closes_it",
        |mut host| {
            let _folder = ready(&mut host);
            searched(&mut host, "nav-");
            assert_eq!(holds(&mut host), Some(Holds::Modal { field: true }));
            assert_eq!(focused(&mut host).as_deref(), Some("palette-field"));

            press(&mut host, Key::Escape, Mods::default());
            let closed = host.run_until(PATIENCE, |host| !host.app().palette.state.is_open());
            assert!(closed, "Escape closed the palette");
            assert_eq!(holds(&mut host), None);
            assert_eq!(focused(&mut host).as_deref(), Some(crate::columns::VIEW));
        },
    );
}

/// A right-click on a result opens its menu, which holds the keyboard; Escape closes it and
/// the field has the keyboard again, the result still highlighted (finding
/// lgse/strata#2468: GTK leaves it on its layer).
#[test]
fn a_results_menu_holds_the_keyboard_and_gives_it_back_to_the_field() {
    hosted_in(
        MODULE,
        "columns",
        "a_results_menu_holds_the_keyboard_and_gives_it_back_to_the_field",
        |mut host| {
            let _folder = ready(&mut host);
            searched(&mut host, "nav-");
            let second = host.app().palette.state.items()[1].id.clone();
            let (x, y) = middle_of(&host, "palette-option-1");
            host.runtime().secondary_down(x, y);
            host.runtime().secondary_up(x, y);
            idle(&mut host);
            let row = highlighted(&host);
            assert_eq!(
                row.as_deref(),
                Some(second.as_str()),
                "the right-click highlights"
            );
            assert!(
                host.app().palette.menu.is_open(),
                "the result's menu opened"
            );
            assert_eq!(holds(&mut host), Some(Holds::Modal { field: false }));
            assert!(
                focused(&mut host).is_some_and(|key| key.starts_with("palette-menu")),
                "the menu has the keyboard"
            );

            press(&mut host, Key::Text("k".to_owned()), ctrl());
            assert!(
                host.app().palette.menu.is_open(),
                "Ctrl+K leaves the menu open, as on GTK"
            );
            assert!(host.app().palette.state.is_open(), "and the palette");
            press(&mut host, Key::Command('k'), ctrl_shift());
            assert!(
                host.app().palette.menu.is_open() && host.app().palette.state.is_open(),
                "Ctrl+Shift+K leaves both open, as on GTK"
            );

            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().palette.menu.is_open(), "Escape closed the menu");
            assert!(host.app().palette.state.is_open(), "the palette stays open");
            assert_eq!(focused(&mut host).as_deref(), Some("palette-field"));
            assert_eq!(holds(&mut host), Some(Holds::Modal { field: true }));
            assert_eq!(highlighted(&host), row, "the highlight stays");
        },
    );
}

/// A middle click on a result opens nothing, and the palette keeps its query and highlight:
/// GTK's rows take no middle click (driven).
#[test]
fn a_middle_click_on_a_result_leaves_the_palette_open() {
    hosted_in(
        MODULE,
        "columns",
        "a_middle_click_on_a_result_leaves_the_palette_open",
        |mut host| {
            let _folder = ready(&mut host);
            searched(&mut host, "nav-");
            let before = highlighted(&host);
            let location = host.app().browser().location_at(0);
            let (x, y) = middle_of(&host, "palette-option-2");
            host.runtime().button_down(x, y, PointerButton::Middle);
            host.runtime().button_up(x, y, PointerButton::Middle);
            idle(&mut host);
            let state = &host.app().palette.state;
            assert!(state.is_open(), "the palette stays open");
            assert_eq!(state.edit.text, "nav-", "its query stays");
            assert_eq!(highlighted(&host), before, "its highlight stays");
            assert_eq!(host.app().browser().location_at(0), location);
            assert_eq!(host.app().browser().location_at(1), None, "nothing opened");
        },
    );
}

/// The header's Search button opens the palette, and a click on it while it is open closes
/// it, the button keeping the keyboard, as GTK's backdrop gives it back to its opener.
#[test]
fn the_search_button_opens_the_palette_and_closes_it() {
    hosted_in(
        MODULE,
        "columns",
        "the_search_button_opens_the_palette_and_closes_it",
        |mut host| {
            let _folder = ready(&mut host);
            let click = |host: &mut Offscreen| {
                let (x, y) = middle_of(host, "header-search");
                host.runtime().pointer_move(x, y);
                host.runtime().pointer_down(x, y);
                host.runtime().pointer_up(x, y);
                idle(host);
            };
            click(&mut host);
            assert!(host.app().palette.state.is_open(), "the button opened it");
            assert_eq!(focused(&mut host).as_deref(), Some("palette-field"));

            click(&mut host);
            let closed = host.run_until(PATIENCE, |host| !host.app().palette.state.is_open());
            assert!(closed, "a click on the button closed it");
            assert_eq!(focused(&mut host).as_deref(), Some("header-search"));
        },
    );
}

/// Alt+Return, the catalog's `open_containing_folder`, shows the highlighted result's folder
/// with it selected, and closes the palette.
#[test]
fn alt_return_reveals_the_highlighted_result_in_its_folder() {
    hosted_in(
        MODULE,
        "columns",
        "alt_return_reveals_the_highlighted_result_in_its_folder",
        |mut host| {
            let _folder = ready(&mut host);
            searched(&mut host, "nav-");
            let target = highlighted(&host).expect("a highlighted result");
            let alt = Mods {
                alt: true,
                ..Mods::default()
            };
            press(&mut host, Key::Enter, alt);
            let closed = host.run_until(PATIENCE, |host| !host.app().palette.state.is_open());
            assert!(closed, "Alt+Return closed the palette");
            let home = strata_core::model::Location::local(crate::launch::home_directory());
            let shown = host.run_until(PATIENCE, |host| {
                let browser = host.app().browser();
                browser
                    .active_depth()
                    .and_then(|depth| browser.location_at(depth))
                    == Some(home.clone())
            });
            assert!(shown, "Home is the folder shown");
            let (selected, _) = view::browser_holds(&host);
            let name = std::path::Path::new(&target)
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a name");
            assert_eq!(selected, [name], "the result is selected in its folder");
        },
    );
}

fn ctrl_shift() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        shift: true,
        ..Mods::default()
    }
}

/// The ids the palette shows.
fn listed(host: &Offscreen) -> Vec<String> {
    let state = &host.app().palette.state;
    state.items().iter().map(|item| item.id.clone()).collect()
}

/// Ctrl+Shift+K lists the folders the columns finished listing, at once and with the field
/// typing; a query narrows them; Enter opens the one highlighted. Ctrl+K closes the jump as
/// it closes its own mode.
#[test]
fn ctrl_shift_k_jumps_to_a_folder_visited_before() {
    hosted_in(
        MODULE,
        "columns",
        "ctrl_shift_k_jumps_to_a_folder_visited_before",
        |mut host| {
            let folder = ready(&mut host);
            let documents = folder.path().join("documents");
            let path = |path: &std::path::Path| path.to_string_lossy().into_owned();
            host.app()
                .open(strata_core::model::Location::local(&documents));
            let listed_documents = host.run_until(PATIENCE, |host| {
                host.app().columns.listed(0)
                    && host.app().browser().location_at(0)
                        == Some(strata_core::model::Location::local(&documents))
            });
            assert!(listed_documents, "documents was not listed");
            idle(&mut host);

            press(&mut host, Key::Command('k'), ctrl_shift());
            assert!(host.app().palette.state.is_open(), "Ctrl+Shift+K opened it");
            let mut shown = listed(&host);
            shown.sort();
            assert_eq!(shown, [path(folder.path()), path(&documents)]);
            assert_eq!(holds(&mut host), Some(Holds::Modal { field: true }));
            assert_eq!(focused(&mut host).as_deref(), Some("palette-field"));

            press(&mut host, Key::Text("k".to_owned()), ctrl());
            assert!(!host.app().palette.state.is_open(), "Ctrl+K closed the jump");

            press(&mut host, Key::Command('k'), ctrl_shift());
            let name = folder
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .expect("the fixture's name");
            for character in name.chars() {
                press(&mut host, Key::Text(character.to_string()), Mods::default());
            }
            assert_eq!(listed(&host).first(), Some(&path(folder.path())));
            press(&mut host, Key::Enter, Mods::default());
            let opened = host.run_until(PATIENCE, |host| {
                !host.app().palette.state.is_open()
                    && host.app().browser().location_at(0)
                        == Some(strata_core::model::Location::local(folder.path()))
            });
            assert!(opened, "Enter opened the folder");
        },
    );
}

// SPDX-License-Identifier: MIT

//! The sidebar in the registered window: what assistive technology hears of its rows, what a
//! click and Return open, where the keyboard goes in and out, and the pins the shared store
//! gives it.

use std::path::{Path, PathBuf};

use rune::{Key, Mods};
use serde_json::Value;
use strata_components::sidebar::{ItemId, Step};
use strata_core::model::Location;
use strata_fs::adapters::bookmarks;

use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE, fixture, hosted, hosted_after,
        view::{all, browser_holds, idle, press, shift, text},
    },
};

const MODULE: &str = "sidebar::tests";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("the test's home"))
}

fn keyed<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    all(tree, &|node| node["key"] == key).into_iter().next()
}

fn named_buttons(host: &Offscreen) -> Vec<String> {
    let tree = host.tree();
    let rows = all(&tree, &|node| {
        node["role"] == "button" && text(node, "key").starts_with("sidebar-")
    });
    rows.iter().map(|row| text(row, "name")).collect()
}

/// Waits for the sidebar's row keyed `key`.
fn shown(host: &mut Offscreen, key: &str) -> Value {
    let reached = host.run_until(PATIENCE, |host| keyed(&host.tree(), key).is_some());
    assert!(
        reached,
        "waited for {key}; the sidebar shows {:?}",
        named_buttons(host)
    );
    keyed(&host.tree(), key).expect("the row").clone()
}

/// The key of the element that has the keyboard, as the accessibility tree says.
fn focused(node: &Value) -> Option<String> {
    let states = node["states"].as_array();
    if states.is_some_and(|states| states.iter().any(|state| state == "focused")) {
        return node["key"].as_str().map(str::to_owned);
    }
    let children = node["children"].as_array().into_iter().flatten();
    children.filter_map(focused).next()
}

fn click(host: &mut Offscreen, key: &str) {
    shown(host, key);
    let elements = host.runtime().elements();
    let row = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = row.expect("the row is mounted").visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn wait_for_location(host: &mut Offscreen, folder: &Path) {
    let wanted = Location::local(folder);
    let reached = host.run_until(PATIENCE, |host| {
        host.app().browser().active_location() == Some(wanted.clone())
    });
    assert!(
        reached,
        "waited for the browser to open {}",
        folder.display()
    );
    idle(host);
}

fn opened(folder: &Path, host: &mut Offscreen) {
    host.app().open(Location::local(folder));
    wait_for_location(host, folder);
}

#[test]
fn places_are_buttons_named_and_described_as_gtk_names_its_rows() {
    hosted(
        MODULE,
        "places_are_buttons_named_and_described_as_gtk_names_its_rows",
        |mut host| {
            let row = shown(&mut host, "sidebar-place-home");
            let trash = shown(&mut host, "sidebar-place-trash");
            assert_eq!(text(&row, "role"), "button");
            assert_eq!(text(&row, "name"), "Home");
            let home_path = Location::local(home()).display_path();
            assert_eq!(text(&row, "description"), home_path);
            assert_eq!(text(&trash, "role"), "button");
            assert_eq!(text(&trash, "name"), "Trash");
            assert_eq!(text(&trash, "description"), "trash:///");
            let names = named_buttons(&host);
            let places = &names[..2];
            assert_eq!(places, ["Home", "Trash"], "the default order: {names:?}");
        },
    );
}

#[test]
fn a_click_opens_a_place_and_marks_its_row() {
    hosted(
        MODULE,
        "a_click_opens_a_place_and_marks_its_row",
        |mut host| {
            let folder = fixture();
            opened(folder.path(), &mut host);
            shown(&mut host, "sidebar-place-home");
            assert_eq!(host.app().sidebar.current, None, "the fixture has no row");

            click(&mut host, "sidebar-place-home");
            wait_for_location(&mut host, &home());
            assert_eq!(
                host.app().sidebar.current,
                Some(ItemId::Place("home".into()))
            );
            let (selected, _) = browser_holds(&host);
            assert!(selected.is_empty(), "a click selects nothing: {selected:?}");
        },
    );
}

#[test]
fn the_keyboard_comes_in_at_the_views_start_and_return_opens_a_place_with_its_first_item() {
    hosted(
        MODULE,
        "the_keyboard_comes_in_at_the_views_start_and_return_opens_a_place_with_its_first_item",
        |mut host| {
            for name in ["a.txt", "b.txt"] {
                std::fs::write(home().join(name), name).expect("a file at home");
            }
            let folder = fixture();
            opened(folder.path(), &mut host);
            shown(&mut host, "sidebar-place-home");

            press(&mut host, Key::Down, Mods::default());
            press(&mut host, Key::Left, Mods::default());
            assert_eq!(
                focused(&host.tree()).as_deref(),
                Some("sidebar-place-home"),
                "Left in the first column hands the keyboard to the sidebar's first row"
            );

            press(&mut host, Key::Enter, Mods::default());
            wait_for_location(&mut host, &home());
            let reached = host.run_until(PATIENCE, |host| browser_holds(host).0 == ["a.txt"]);
            assert!(
                reached,
                "Return opens Home with its first item selected: {:?}",
                browser_holds(&host)
            );
            assert_eq!(
                host.app().sidebar.current,
                Some(ItemId::Place("home".into()))
            );
        },
    );
}

fn bookmarks_file() -> PathBuf {
    bookmarks::pinned_places_path()
}

fn pin(path: &Path, name: &str) -> String {
    format!("file://{} {name}\n", path.display())
}

#[test]
fn pins_come_from_the_bookmarks_file_and_follow_the_shared_store() {
    hosted_after(
        MODULE,
        "pins_come_from_the_bookmarks_file_and_follow_the_shared_store",
        || {
            let file = bookmarks_file();
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("gtk-3.0");
            let pins = format!(
                "{}{}",
                pin(&home(), "Home again"),
                pin(Path::new("/srv/work"), "Work")
            );
            std::fs::write(file, pins).expect("the bookmarks file");
        },
        |mut host| {
            let work = shown(&mut host, "sidebar-pin-1");
            assert_eq!(text(&work, "role"), "button");
            assert_eq!(text(&work, "name"), "Work");
            let names = named_buttons(&host);
            assert!(
                !names.iter().any(|name| name == "Home again"),
                "a pin of a built-in place is not listed twice: {names:?}"
            );

            let other = pin(Path::new("/srv/other"), "Other");
            glib::MainContext::default().spawn_local(async move {
                let written = bookmarks::update(move |_| Some(other.clone().into_bytes())).await;
                written.expect("the bookmarks file written");
            });
            let reached = host.run_until(PATIENCE, |host| {
                let names = named_buttons(host);
                names.iter().any(|name| name == "Other") && !names.iter().any(|name| name == "Work")
            });
            assert!(
                reached,
                "the store's change reaches the sidebar: {:?}",
                named_buttons(&host)
            );
        },
    );
}

/// Writes `pins` as the bookmarks file, before the window starts.
fn seed_pins(pins: &str) {
    let file = bookmarks_file();
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("gtk-3.0");
    std::fs::write(file, pins).expect("the bookmarks file");
}

#[test]
fn left_comes_into_the_sidebar_on_the_open_places_row() {
    hosted_after(
        MODULE,
        "left_comes_into_the_sidebar_on_the_open_places_row",
        || {
            std::fs::create_dir_all(home().join("work/inner")).expect("a pinned folder");
            seed_pins(&pin(&home().join("work"), "Work"));
        },
        |mut host| {
            opened(&home().join("work"), &mut host);
            shown(&mut host, "sidebar-pin-0");
            let reached = host.run_until(PATIENCE, |host| {
                host.app().sidebar.current == Some(ItemId::Pin("0".into()))
            });
            assert!(reached, "the pin of the open folder is marked");

            press(&mut host, Key::Down, Mods::default());
            press(&mut host, Key::Left, Mods::default());
            assert_eq!(focused(&host.tree()).as_deref(), Some("sidebar-pin-0"));
        },
    );
}

fn settings_file() -> PathBuf {
    strata_core::storage::config_directory().join("settings.toml")
}

/// `sidebar_pinned_collapsed` and the sidebar's step start as saved, and the PINNED heading
/// saves its own state (`docs/preferences.md`, "Sidebar collapsed sections").
#[test]
fn the_saved_sections_and_step_start_the_sidebar_and_a_heading_saves_its_state() {
    hosted_after(
        MODULE,
        "the_saved_sections_and_step_start_the_sidebar_and_a_heading_saves_its_state",
        || {
            let file = settings_file();
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = "mode = \"theme\"\ntheme = \"azure-glow\"\n\
                         sidebar_pinned_collapsed = true\nsidebar_compact = true\n";
            std::fs::write(file, saved).expect("settings");
            seed_pins(&pin(Path::new("/srv/work"), "Work"));
        },
        |mut host| {
            shown(&mut host, "sidebar-heading-pinned");
            let state = &host.app().sidebar.state;
            assert!(state.pinned_collapsed, "PINNED starts collapsed, as saved");
            assert_eq!(
                state.width.chosen(),
                Step::Rail,
                "the sidebar starts as the rail"
            );

            click(&mut host, "sidebar-heading-pinned");
            let reached = host.run_until(PATIENCE, |_| {
                std::fs::read_to_string(settings_file())
                    .is_ok_and(|saved| saved.contains("sidebar_pinned_collapsed = false"))
            });
            assert!(reached, "the heading saves PINNED as expanded");
            assert!(!host.app().sidebar.state.pinned_collapsed);
        },
    );
}

/// The box of the row keyed `key`, as drawn.
fn bounds(host: &mut Offscreen, key: &str) -> [f32; 4] {
    shown(host, key);
    let elements = host.runtime().elements();
    let row = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    row.expect("the row is mounted").visible
}

/// A pointer drag from the middle of `from` to `to`, in steps, as a scenario's `drag` makes.
fn drag(host: &mut Offscreen, from: (f32, f32), to: (f32, f32)) {
    host.runtime().pointer_move(from.0, from.1);
    host.runtime().pointer_down(from.0, from.1);
    host.turn();
    for step in 1..=12 {
        let k = step as f32 / 12.0;
        let (x, y) = (from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k);
        host.runtime().pointer_move(x, y);
        host.turn();
    }
    host.runtime().pointer_up(to.0, to.1);
    idle(host);
}

fn saved_order() -> Vec<String> {
    let saved = std::fs::read_to_string(settings_file()).unwrap_or_default();
    let saved: toml::Table = toml::from_str(&saved).unwrap_or_default();
    let order = saved.get("sidebar_order").and_then(toml::Value::as_array);
    let names = order.into_iter().flatten().filter_map(toml::Value::as_str);
    names.map(str::to_owned).collect()
}

/// `builtin-places-reorder-survive-restart/columns-gap-after`: a place dropped in the gap below
/// another lands right after it, as Strata's sidebar drops it, though the pointer is past half
/// a row from where the drag began.
#[test]
fn a_place_dropped_in_the_gap_below_a_row_lands_after_it_and_is_saved() {
    hosted(
        MODULE,
        "a_place_dropped_in_the_gap_below_a_row_lands_after_it_and_is_saved",
        |mut host| {
            shown(&mut host, "sidebar-place-network");
            let [x, y, w, h] = bounds(&mut host, "sidebar-place-home");
            let [_, trash_y, _, trash_h] = bounds(&mut host, "sidebar-place-trash");
            drag(
                &mut host,
                (x + w / 2.0, y + h / 2.0),
                (x + w / 2.0, trash_y + trash_h + 1.0),
            );
            let reached = host.run_until(PATIENCE, |_| saved_order().len() > 2);
            assert!(reached, "the new order is saved");
            assert_eq!(saved_order()[..3], ["trash", "home", "network"]);
            let shown: Vec<&str> = host
                .app()
                .sidebar
                .listed
                .places
                .iter()
                .map(|place| place.id.as_str())
                .collect();
            assert_eq!(shown[..3], ["trash", "home", "network"]);
            assert_eq!(
                host.app().browser().active_location(),
                None,
                "the drop opens nothing"
            );
        },
    );
}

/// A drag that ends where it began opens nothing, as Strata's drag source claims the gesture:
/// the press Rune sends on that release is the drag's.
#[test]
fn a_drag_dropped_back_in_place_opens_nothing() {
    hosted(
        MODULE,
        "a_drag_dropped_back_in_place_opens_nothing",
        |mut host| {
            let [x, y, w, h] = bounds(&mut host, "sidebar-place-home");
            let middle = (x + w / 2.0, y + h / 2.0);
            drag(&mut host, middle, (middle.0, middle.1 + 10.0));
            host.run_until(std::time::Duration::from_millis(300), |_| false);
            assert_eq!(host.app().browser().active_location(), None);
            assert_eq!(host.app().sidebar.current, None);
            let order = saved_order();
            assert!(
                order.is_empty(),
                "a drop in place saves no order: {order:?}"
            );
        },
    );
}

/// Strata's sidebar rebuilds its rows when they change, which ends a press on one: a press
/// held while the sidebar lists its rows again opens nothing on its release.
#[test]
fn a_press_held_across_a_relist_opens_nothing_on_its_release() {
    hosted(
        MODULE,
        "a_press_held_across_a_relist_opens_nothing_on_its_release",
        |mut host| {
            let [x, y, w, h] = bounds(&mut host, "sidebar-place-home");
            let (x, y) = (x + w / 2.0, y + h / 2.0);
            host.runtime().pointer_move(x, y);
            host.runtime().pointer_down(x, y);
            host.turn();
            host.runtime().update(|app: &mut App, _| super::relist(app));
            host.turn();
            host.runtime().pointer_up(x, y);
            idle(&mut host);
            host.run_until(std::time::Duration::from_millis(300), |_| false);
            assert_eq!(host.app().browser().active_location(), None);
        },
    );
}

const TOGGLE: &str = "header-sidebar-toggle";

/// The saved `sidebar_expanded` and `sidebar_compact`, with their defaults where the file
/// leaves them out.
fn saved_width() -> (bool, bool) {
    let saved = std::fs::read_to_string(settings_file()).unwrap_or_default();
    let saved: toml::Table = toml::from_str(&saved).unwrap_or_default();
    let flag = |key: &str, default: bool| {
        saved
            .get(key)
            .and_then(toml::Value::as_bool)
            .unwrap_or(default)
    };
    (
        flag("sidebar_expanded", true),
        flag("sidebar_compact", false),
    )
}

/// Waits for the step the toggle took to be saved and to show, and for the toggle to name the
/// next step.
fn stepped_to(host: &mut Offscreen, step: Step, saved: (bool, bool), next: &str) {
    let reached = host.run_until(PATIENCE, |_| saved_width() == saved);
    assert!(
        reached,
        "{step:?} saved as {saved:?}, not {:?}",
        saved_width()
    );
    assert_eq!(host.app().sidebar.state.width.shown(), step);
    let toggle = shown(host, TOGGLE);
    assert_eq!(text(&toggle, "name"), "Toggle sidebar (Ctrl+B)");
    assert_eq!(text(&toggle, "description"), next, "at {step:?}");
    let states = toggle["states"].as_array().cloned().unwrap_or_default();
    let on = if step == Step::Hidden {
        "unchecked"
    } else {
        "checked"
    };
    assert!(states.iter().any(|state| state == on), "{states:?}");
    let rows = keyed(&host.tree(), "sidebar-place-home").is_some();
    assert_eq!(rows, step != Step::Hidden, "the places at {step:?}");
}

/// The header's toggle and Ctrl+B step the sidebar full, rail, hidden and full again, as GTK's
/// `SidebarMode::next` does; each step is saved as `bind_sidebar_modes` saves it, and the
/// toggle's description names the step the next press takes.
#[test]
fn the_toggle_and_its_key_step_the_width_and_save_each_step() {
    hosted(
        MODULE,
        "the_toggle_and_its_key_step_the_width_and_save_each_step",
        |mut host| {
            shown(&mut host, "sidebar-place-home");
            stepped_to(
                &mut host,
                Step::Full,
                (true, false),
                "Compact sidebar (Ctrl+B)",
            );

            click(&mut host, TOGGLE);
            stepped_to(&mut host, Step::Rail, (true, true), "Hide sidebar (Ctrl+B)");

            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            press(&mut host, Key::Command('b'), ctrl);
            stepped_to(
                &mut host,
                Step::Hidden,
                (false, true),
                "Show sidebar (Ctrl+B)",
            );

            click(&mut host, TOGGLE);
            stepped_to(
                &mut host,
                Step::Full,
                (true, false),
                "Compact sidebar (Ctrl+B)",
            );
        },
    );
}

/// A hidden sidebar saved from the rail (`sidebar_expanded = false`, `sidebar_compact =
/// true`) starts hidden, and the toggle shows the full sidebar, not the rail, as GTK's step to
/// `Expanded` saves both.
#[test]
fn a_sidebar_hidden_from_the_rail_starts_hidden_and_shows_full() {
    hosted_after(
        MODULE,
        "a_sidebar_hidden_from_the_rail_starts_hidden_and_shows_full",
        || {
            let file = settings_file();
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = "mode = \"theme\"\ntheme = \"azure-glow\"\n\
                         sidebar_expanded = false\nsidebar_compact = true\n";
            std::fs::write(file, saved).expect("settings");
        },
        |mut host| {
            stepped_to(
                &mut host,
                Step::Hidden,
                (false, true),
                "Show sidebar (Ctrl+B)",
            );
            click(&mut host, TOGGLE);
            stepped_to(
                &mut host,
                Step::Full,
                (true, false),
                "Compact sidebar (Ctrl+B)",
            );
        },
    );
}

/// Left at the start of a listed folder: the keyboard comes into the sidebar on Home's row.
fn into_the_sidebar(host: &mut Offscreen) {
    press(host, Key::Down, Mods::default());
    press(host, Key::Left, Mods::default());
    assert_eq!(focused(&host.tree()).as_deref(), Some("sidebar-place-home"));
}

/// Waits for `key` to have the keyboard after `pressed`: an arrow's other key reaches Rune's
/// focus navigation as the arrow, a frame later.
fn focus_reaches(host: &mut Offscreen, key: &str, pressed: &Key) {
    host.run_until(PATIENCE, |host| {
        focused(&host.tree()).as_deref() == Some(key)
    });
    let at = focused(&host.tree());
    assert_eq!(at.as_deref(), Some(key), "after {pressed:?}");
}

fn letter(letter: &str) -> Key {
    Key::Text(letter.to_owned())
}

/// The default map's arrows in the sidebar, and their other keys, as GTK's
/// `sidebar_navigation` reads them: Down and Up move between the rows, and Right gives the
/// keyboard back to the files.
#[test]
fn the_arrows_and_their_other_keys_move_between_the_rows_and_back_to_the_files() {
    hosted(
        MODULE,
        "the_arrows_and_their_other_keys_move_between_the_rows_and_back_to_the_files",
        |mut host| {
            std::fs::write(home().join("a.txt"), "a").expect("a file at home");
            let folder = fixture();
            opened(folder.path(), &mut host);
            shown(&mut host, "sidebar-place-trash");
            let view = crate::shell::view_focus(host.app()).expect("the view's key");
            for [down, up, right] in [
                [Key::Down, Key::Up, Key::Right],
                [letter("j"), letter("k"), letter("l")],
            ] {
                into_the_sidebar(&mut host);
                press(&mut host, down.clone(), Mods::default());
                focus_reaches(&mut host, "sidebar-place-trash", &down);
                press(&mut host, up.clone(), Mods::default());
                focus_reaches(&mut host, "sidebar-place-home", &up);
                press(&mut host, right.clone(), Mods::default());
                focus_reaches(&mut host, view, &right);
            }
        },
    );
}

/// `key` on the sidebar's first row reaches the header's toggle
/// (`TopBarNavigation::move_up_from_sidebar`).
fn up_from_the_top_reaches_the_toggle(mut host: Offscreen, key: Key) {
    let folder = fixture();
    opened(folder.path(), &mut host);
    shown(&mut host, "sidebar-place-home");
    into_the_sidebar(&mut host);
    press(&mut host, key.clone(), Mods::default());
    focus_reaches(&mut host, TOGGLE, &key);
}

#[test]
fn up_from_the_first_row_reaches_the_headers_toggle() {
    hosted(
        MODULE,
        "up_from_the_first_row_reaches_the_headers_toggle",
        |host| up_from_the_top_reaches_the_toggle(host, Key::Up),
    );
}

#[test]
fn k_from_the_first_row_reaches_the_headers_toggle() {
    hosted(
        MODULE,
        "k_from_the_first_row_reaches_the_headers_toggle",
        |host| up_from_the_top_reaches_the_toggle(host, letter("k")),
    );
}

/// Shift and Super with Up and Down move between the rows as the plain arrows do, as GTK's
/// `child_focus` moves them with both held; Shift+Up on the first row moves nothing, and
/// Super+Up there reaches the header's toggle, since only Shift keeps Up in the sidebar.
#[test]
fn shift_and_super_with_up_and_down_move_between_the_rows() {
    hosted(
        MODULE,
        "shift_and_super_with_up_and_down_move_between_the_rows",
        |mut host| {
            let folder = fixture();
            opened(folder.path(), &mut host);
            shown(&mut host, "sidebar-place-trash");
            let logo = Mods {
                logo: true,
                ..Mods::default()
            };
            into_the_sidebar(&mut host);
            for mods in [shift(), logo] {
                press(&mut host, Key::Down, mods);
                focus_reaches(&mut host, "sidebar-place-trash", &Key::Down);
                press(&mut host, Key::Up, mods);
                focus_reaches(&mut host, "sidebar-place-home", &Key::Up);
            }
            press(&mut host, Key::Up, shift());
            host.turn();
            idle(&mut host);
            let at = focused(&host.tree());
            assert_eq!(
                at.as_deref(),
                Some("sidebar-place-home"),
                "Shift+Up at the top"
            );
            press(&mut host, Key::Up, logo);
            focus_reaches(&mut host, TOGGLE, &Key::Up);
        },
    );
}

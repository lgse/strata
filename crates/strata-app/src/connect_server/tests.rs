// SPDX-License-Identifier: MIT

use std::path::Path;
use std::time::Instant;

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;
use strata_fs::adapters::bookmarks::{
    parse_pinned_places, pinned_places_path, serialize_pinned_places,
};

use super::{ConnectServer, MIN_CONNECTING, normalized_location, pin_label, view};
use crate::{
    App,
    commands::Holds,
    host::offscreen::Offscreen,
    shell,
    slots::Part,
    test_support::{PATIENCE, fixture, hosted, send_key, view::idle},
};

const MODULE: &str = "connect_server::tests";

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    idle(host);
}

fn center_of(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let mounted = |host: &Offscreen| {
        host.elements()
            .iter()
            .any(|el| !el.exiting && el.key.as_deref() == Some(key))
    };
    assert!(host.run_until(PATIENCE, mounted), "{key} is mounted");
    let elements = host.runtime().elements();
    let el = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(key))
        .expect("mounted element");
    let [x, y, w, h] = el.visible;
    (x + w / 2.0, y + h / 2.0)
}

fn click(host: &mut Offscreen, key: &str) {
    let (x, y) = center_of(host, key);
    click_at(host, x, y);
}

fn click_at(host: &mut Offscreen, x: f32, y: f32) {
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn right_click(host: &mut Offscreen, key: &str) {
    let (x, y) = center_of(host, key);
    host.runtime().pointer_move(x, y);
    host.runtime().secondary_down(x, y);
    host.turn();
    host.runtime().secondary_up(x, y);
    idle(host);
}

fn a11y_center(node: &serde_json::Value, role: &str, name: &str) -> Option<(f32, f32)> {
    if node["role"].as_str() == Some(role)
        && node["name"].as_str() == Some(name)
        && let Some([x, y, w, h]) = node["bounds"].as_array().map(Vec::as_slice)
    {
        return Some((
            (x.as_f64()? + w.as_f64()? / 2.0) as f32,
            (y.as_f64()? + h.as_f64()? / 2.0) as f32,
        ));
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|child| a11y_center(child, role, name))
}

fn has_a11y_node(node: &serde_json::Value, role: &str, name: &str, state: Option<&str>) -> bool {
    let matches_self = node["role"].as_str() == Some(role)
        && node["name"].as_str() == Some(name)
        && state.is_none_or(|want| {
            node["states"]
                .as_array()
                .is_some_and(|states| states.iter().any(|s| s.as_str() == Some(want)))
        });
    if matches_self {
        return true;
    }
    node["children"]
        .as_array()
        .is_some_and(|children| children.iter().any(|c| has_a11y_node(c, role, name, state)))
}

fn focused_key(host: &Offscreen) -> Option<String> {
    host.elements()
        .iter()
        .find(|el| el.focused)
        .and_then(|el| el.key.clone())
}

fn open_fixture(host: &mut Offscreen, path: &Path) {
    host.runtime().update(|app, _| app.look.animations = false);
    let root = Location::local(path);
    host.app().open(root.clone());
    assert!(
        host.run_until(PATIENCE, |h| {
            h.app().browser().active_location().as_ref() == Some(&root)
                && h.app().browser().column_entry_counts(0).is_some()
                && h.elements()
                    .iter()
                    .any(|el| el.key.as_deref() == Some("sidebar-place-network"))
        }),
        "fixture and Network sidebar row ready"
    );
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        app.browser
            .select_entries_by_name_at(0, &[String::from("readme.md")]);
        shell::settle(app, tx);
        tx.focus(crate::columns::VIEW, true);
    });
    idle(host);
}

fn open_via_network_menu(host: &mut Offscreen) {
    right_click(host, "sidebar-place-network");
    let (mx, my) = a11y_center(&host.tree(), "menu-item", "Connect to server…")
        .expect("Connect to server… row in Network menu");
    click_at(host, mx, my);
    assert!(host.app().connect_server.open);
}

fn select_all_and_type(host: &mut Offscreen, value: &str) {
    press(
        host,
        Key::Command('a'),
        Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        },
    );
    press(host, Key::Text(value.to_owned()), Mods::default());
}

#[test]
fn pinned_network_locations_survive_the_bookmark_round_trip() {
    for uri in [
        "smb://host/share",
        "smb://host/share/",
        "sftp://host/home/me",
        "davs://host/files",
    ] {
        let pinned = normalized_location(Location::uri(uri));
        let place = (pinned.clone(), pin_label(&pinned));
        let reloaded =
            parse_pinned_places(serialize_pinned_places(std::slice::from_ref(&place)).as_bytes());
        assert_eq!(reloaded, vec![place], "{uri}");
    }
}

#[test]
fn network_menu_opens_dialog_with_named_controls_keyboard_hold_and_escape_focus_return() {
    hosted(
        MODULE,
        "network_menu_opens_dialog_with_named_controls_keyboard_hold_and_escape_focus_return",
        |mut host| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());

            open_via_network_menu(&mut host);
            let tree = host.tree();
            for (role, name, state) in [
                ("dialog", "Connect to server", None),
                ("text-input", "Server address", Some("focused")),
                ("check-box", "Pin to sidebar", Some("unchecked")),
                ("default-button", "Connect", None),
                ("button", "Cancel", None),
                ("button", "Close dialog", None),
            ] {
                assert!(
                    has_a11y_node(&tree, role, name, state),
                    "{role} {name:?} {state:?}"
                );
            }
            assert_eq!(
                focused_key(&host).as_deref(),
                Some("connect-server-1-address")
            );
            assert_eq!(
                ConnectServer::keyboard(host.app()),
                Some(Holds::Modal { field: true })
            );

            // Empty form dismisses on backdrop click and returns focus to Columns.
            click_at(&mut host, 5.0, 5.0);
            assert!(!host.app().connect_server.open);
            assert!(
                focused_key(&host)
                    .as_deref()
                    .is_some_and(|key| key.starts_with(crate::columns::VIEW))
            );

            // Dirty form blocks backdrop click, while Escape closes and wipes the address.
            open_via_network_menu(&mut host);
            select_all_and_type(&mut host, "sftp://user:secret@example/share");
            click_at(&mut host, 5.0, 5.0);
            assert!(host.app().connect_server.open);

            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().connect_server.open);
            assert_eq!(
                host.app()
                    .connect_server
                    .form
                    .as_ref()
                    .expect("form")
                    .address
                    .text,
                ""
            );
            assert!(
                focused_key(&host)
                    .as_deref()
                    .is_some_and(|key| key.starts_with(crate::columns::VIEW))
            );
        },
    );
}

fn form_error(host: &Offscreen) -> Option<&str> {
    host.app()
        .connect_server
        .form
        .as_ref()
        .and_then(|form| form.error.as_deref())
}

#[test]
fn inline_validation_clears_on_edit_and_cancel_aborts_in_flight_connect() {
    hosted(
        MODULE,
        "inline_validation_clears_on_edit_and_cancel_aborts_in_flight_connect",
        |mut host| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());

            // Opening via the Menu key on focused Network row returns focus to Network on Escape.
            host.runtime()
                .update(|_: &mut App, tx: &mut Tx| tx.focus("sidebar-place-network", true));
            idle(&mut host);
            press(&mut host, Key::ContextMenu, Mods::default());
            let (mx, my) = a11y_center(&host.tree(), "menu-item", "Connect to server…")
                .expect("Connect to server… row");
            click_at(&mut host, mx, my);
            assert!(host.app().connect_server.open);

            select_all_and_type(&mut host, "user@host:path");
            press(&mut host, Key::Enter, Mods::default());
            assert!(host.app().connect_server.open);
            assert!(form_error(&host).is_some());

            select_all_and_type(&mut host, "smb://unreachable.invalid/share");
            assert!(form_error(&host).is_none());

            // Cancelling mid-connect bumps the browser generation and restores Network row focus.
            let generation_before = host.app().browser().navigation_generation();
            press(&mut host, Key::Enter, Mods::default());
            assert!(
                host.app()
                    .connect_server
                    .form
                    .as_ref()
                    .is_some_and(|form| form.connecting)
            );
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().connect_server.open);
            assert!(host.app().browser().navigation_generation() > generation_before);
            assert_eq!(focused_key(&host).as_deref(), Some("sidebar-place-network"));
        },
    );
}

#[test]
fn connecting_holds_spinner_shows_inline_error_and_pins_only_on_success() {
    hosted(
        MODULE,
        "connecting_holds_spinner_shows_inline_error_and_pins_only_on_success",
        |mut host| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());

            open_via_network_menu(&mut host);
            let generation = host
                .app()
                .connect_server
                .form
                .as_ref()
                .expect("form")
                .generation;
            select_all_and_type(&mut host, "smb://unreachable.invalid/share");
            click(&mut host, &view::pin_key(generation));
            let pins_before = std::fs::read_to_string(pinned_places_path()).unwrap_or_default();
            let clicked = Instant::now();
            click(&mut host, &view::primary_key(generation));

            assert!(
                host.app()
                    .connect_server
                    .form
                    .as_ref()
                    .is_some_and(|form| form.connecting)
            );
            let failed = host.run_until(PATIENCE, |h| {
                h.app()
                    .connect_server
                    .form
                    .as_ref()
                    .is_some_and(|form| !form.connecting && form.error.is_some())
            });
            assert!(failed, "failed connection shows inline error");
            assert!(clicked.elapsed() >= MIN_CONNECTING);
            assert!(!has_a11y_node(
                &host.tree(),
                "dialog",
                "Unable to open location",
                None
            ));
            assert_eq!(
                std::fs::read_to_string(pinned_places_path()).unwrap_or_default(),
                pins_before
            );
            assert_eq!(
                focused_key(&host).as_deref(),
                Some(view::address_key(generation).as_str())
            );

            // Retry with a reachable folder: closes dialog, navigates, focuses view, and pins.
            let target = tempfile::tempdir().expect("target folder");
            let opened = Location::local(target.path());
            select_all_and_type(&mut host, &target.path().to_string_lossy());
            press(&mut host, Key::Enter, Mods::default());

            let connected = host.run_until(PATIENCE, |h| {
                let bookmarks = std::fs::read_to_string(pinned_places_path()).unwrap_or_default();
                !h.app().connect_server.open
                    && h.app().browser().active_location().as_ref() == Some(&opened)
                    && parse_pinned_places(bookmarks.as_bytes())
                        .iter()
                        .any(|(loc, _)| *loc == opened)
                    && h.elements()
                        .iter()
                        .any(|el| el.key.as_deref() == Some("sidebar-pin-0"))
            });
            assert!(connected, "connected folder opens and is pinned to sidebar");
            assert!(
                focused_key(&host)
                    .as_deref()
                    .is_some_and(|key| key.starts_with(crate::columns::VIEW))
            );
        },
    );
}

// SPDX-License-Identifier: MIT

//! Behavioural tests for the Compress dialog (`super`): basename normalization, format and
//! password protection toggles, basename and password validation, backdrop dirty blocking,
//! archive creation + post-compression selection, and safe non-overwrite on collision.

use std::path::Path;

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::{ArchiveFormat, Location};
use strata_desktop::keys::bindings::ACTIONS_COMPRESS;

use super::{Compress, normalized_archive_name};
use crate::{
    App,
    commands::{self, Holds, On, Outcome},
    host::offscreen::Offscreen,
    shell,
    slots::Part,
    test_support::{PATIENCE, fixture, hosted, send_key, view::idle},
};

const MODULE: &str = "compress::tests";

fn center_of(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let mounted = |host: &Offscreen| {
        host.elements()
            .iter()
            .any(|element| !element.exiting && element.key.as_deref() == Some(key))
    };
    assert!(host.run_until(PATIENCE, mounted), "{key} is mounted");
    let elements = host.runtime().elements();
    let element = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key))
        .expect("the element");
    let [x, y, w, h] = element.visible;
    (x + w / 2.0, y + h / 2.0)
}

fn click(host: &mut Offscreen, key: &str) {
    let (x, y) = center_of(host, key);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn click_at(host: &mut Offscreen, x: f32, y: f32) {
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn select_all_and_type(host: &mut Offscreen, text: &str) {
    let ctrl_a = Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    };
    press(host, Key::Command('a'), ctrl_a);
    press(host, Key::Text(text.to_owned()), Mods::default());
}

fn open_fixture(host: &mut Offscreen, root: &Path) {
    host.app().open(Location::local(root));
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder is listed");
    while host.turn() {}
}

fn open_compress_on_row(host: &mut Offscreen, path: &Path) {
    let id = shell::id_of(host.app(), &Location::local(path));
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        assert_eq!(
            commands::run(app, "compress", On::Row((0, id)), tx),
            Outcome::Ran
        );
    });
    idle(host);
}

fn has_a11y_node(node: &serde_json::Value, role: &str, name: &str, state: Option<&str>) -> bool {
    let matches_self = node.get("role").and_then(|v| v.as_str()) == Some(role)
        && node.get("name").and_then(|v| v.as_str()) == Some(name)
        && state.is_none_or(|wanted| {
            node.get("states")
                .and_then(|v| v.as_array())
                .is_some_and(|states| states.iter().any(|s| s.as_str() == Some(wanted)))
        });
    matches_self
        || node
            .get("children")
            .and_then(|v| v.as_array())
            .is_some_and(|children| {
                children
                    .iter()
                    .any(|child| has_a11y_node(child, role, name, state))
            })
}

#[test]
fn normalized_archive_name_strips_only_the_selected_format_suffix() {
    let cases = [
        ("backup.zip", ArchiveFormat::Zip, "backup"),
        ("backup.zip", ArchiveFormat::SevenZ, "backup.zip"),
        ("backup.7z", ArchiveFormat::SevenZ, "backup"),
        ("backup.tar.gz", ArchiveFormat::TarGz, "backup"),
        ("backup.tar.gz", ArchiveFormat::Tar, "backup.tar.gz"),
        ("backup.tar", ArchiveFormat::Tar, "backup"),
    ];
    for (typed, format, expected) in cases {
        assert_eq!(normalized_archive_name(typed, format), expected);
    }
}

#[test]
fn format_and_protection_toggles_control_password_fields_and_keyboard_hold() {
    hosted(
        MODULE,
        "format_and_protection_toggles_control_password_fields_and_keyboard_hold",
        |mut host| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());
            open_compress_on_row(&mut host, &folder.path().join("readme.md"));

            assert!(host.app().compress.open);
            assert_eq!(
                <Compress as Part>::keyboard(host.app()),
                Some(Holds::Modal { field: true })
            );
            let form = host.app().compress.form.as_ref().expect("compress form");
            assert_eq!(form.title, "Compress 1 item");
            assert_eq!(form.subtitle, "1 file");
            assert_eq!(form.name.text, "readme.md");

            // Clicking the already-active ZIP toggle keeps it active.
            click(&mut host, "compress-1-format-zip");
            assert_eq!(
                host.app().compress.form.as_ref().expect("form").format,
                ArchiveFormat::Zip
            );

            // Enabling Password protected reveals and focuses Password, and TAR.GZ hides and
            // resets Protection.
            click(&mut host, "compress-1-protection-password");
            assert!(
                host.app()
                    .compress
                    .form
                    .as_ref()
                    .expect("form")
                    .shows_password()
            );
            assert!(
                host.app()
                    .compress
                    .form
                    .as_ref()
                    .expect("form")
                    .password
                    .focused
            );
            click(&mut host, "compress-1-password-reveal");
            assert!(
                host.app()
                    .compress
                    .form
                    .as_ref()
                    .expect("form")
                    .password_revealed
            );

            click(&mut host, "compress-1-format-tar-gz");
            let form = host.app().compress.form.as_ref().expect("form");
            assert_eq!(form.format, ArchiveFormat::TarGz);
            assert!(!form.password_protected);
            assert!(!form.shows_password());

            click(&mut host, "compress-1-format-7z");
            let form = host.app().compress.form.as_ref().expect("form");
            assert_eq!(form.format, ArchiveFormat::SevenZ);
            assert!(!form.password_protected);
        },
    );
}

#[test]
fn invalid_names_password_errors_and_dirty_backdrop_keep_the_dialog_open() {
    hosted(
        MODULE,
        "invalid_names_password_errors_and_dirty_backdrop_keep_the_dialog_open",
        |mut host| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());
            open_compress_on_row(&mut host, &folder.path().join("readme.md"));

            // Clean backdrop click dismisses the dialog.
            click_at(&mut host, 5.0, 5.0);
            assert!(!host.app().compress.open);

            // Reopen, enter an invalid traversal name, and verify Enter keeps Compress open.
            open_compress_on_row(&mut host, &folder.path().join("readme.md"));
            select_all_and_type(&mut host, "../escape");
            assert_eq!(
                host.app().compress.form.as_ref().expect("form").name_error,
                Some("Names cannot contain /")
            );
            press(&mut host, Key::Enter, Mods::default());
            assert!(host.app().compress.open);
            let rejected = host.app().compress.form.as_ref().expect("form");
            assert_eq!(
                (rejected.name.anchor, rejected.name.focus),
                (0, "../escape".len()),
                "refusal selects the invalid archive name"
            );
            assert!(!folder.path().join("escape.zip").exists());

            // Dirty backdrop click leaves the modified dialog open.
            click_at(&mut host, 5.0, 5.0);
            assert!(host.app().compress.open);

            // Valid name with empty password stacks "Password required"; Escape closes only it.
            click(&mut host, "compress-2-name");
            select_all_and_type(&mut host, "bundle");
            click(&mut host, "compress-2-protection-password");
            click(&mut host, "compress-2-primary");
            assert!(host.app().compress.open);
            assert!(host.app().compress.notice_open);
            assert_eq!(
                host.app().compress.notice.as_ref().expect("notice").title,
                "Password required"
            );
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().compress.notice_open);
            assert!(host.app().compress.open);

            // Mismatched passwords stack "Passwords do not match".
            click(&mut host, "compress-2-password");
            select_all_and_type(&mut host, "secret");
            click(&mut host, "compress-2-confirm");
            select_all_and_type(&mut host, "other");
            press(&mut host, Key::Enter, Mods::default());
            assert!(host.app().compress.notice_open);
            assert_eq!(
                host.app().compress.notice.as_ref().expect("notice").title,
                "Passwords do not match"
            );
        },
    );
}

fn bind_compress_shortcut_and_select_pair(host: &mut Offscreen) {
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        shell::saved(app).change(|settings| {
            settings.keymap.default.insert(
                ACTIONS_COMPRESS.id().to_owned(),
                toml::Value::Array(vec![toml::Value::String("<Control><Alt>c".to_owned())]),
            );
        });
        let names = [String::from("readme.md"), String::from("todo.txt")];
        app.browser.select_entries_by_name_at(0, &names);
        shell::settle(app, tx);
        tx.focus(crate::columns::VIEW, true);
    });
    idle(host);
}

fn set_tenxer_mode(host: &mut Offscreen, enabled: bool) {
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        shell::saved(app).change(|settings| settings.tenxer_mode = enabled);
        shell::settle(app, tx);
        tx.focus(crate::columns::VIEW, true);
    });
    idle(host);
}

#[test]
fn confirming_creates_and_selects_archive_while_collision_stops_without_overwriting() {
    hosted(
        MODULE,
        "confirming_creates_and_selects_archive_while_collision_stops_without_overwriting",
        |mut host| {
            let folder = fixture();
            let existing = folder.path().join("taken.zip");
            std::fs::write(&existing, "keep original").expect("seed existing archive");
            open_fixture(&mut host, folder.path());

            // Collision on an existing archive stops safely without overwriting.
            open_compress_on_row(&mut host, &folder.path().join("readme.md"));
            select_all_and_type(&mut host, "taken.zip");
            press(&mut host, Key::Enter, Mods::default());
            assert!(!host.app().compress.open);

            bind_compress_shortcut_and_select_pair(&mut host);
            let chord = Mods {
                ctrl: true,
                alt: true,
                ..Mods::default()
            };
            set_tenxer_mode(&mut host, true);
            press(&mut host, Key::Text("c".to_owned()), chord);
            assert!(!host.app().compress.open);

            set_tenxer_mode(&mut host, false);
            press(&mut host, Key::Text("c".to_owned()), chord);
            assert!(host.app().compress.open);
            let form = host.app().compress.form.as_ref().expect("form");
            assert_eq!(form.title, "Compress 2 items");
            assert_eq!(form.default_name, "archive");

            select_all_and_type(&mut host, "bundle");
            press(&mut host, Key::Enter, Mods::default());
            assert!(!host.app().compress.open);

            let archive = folder.path().join("bundle.zip");
            let selected = |host: &Offscreen| {
                archive.exists()
                    && host
                        .app()
                        .browser()
                        .selected_entries()
                        .iter()
                        .any(|entry| entry.display_name == "bundle.zip")
            };
            assert!(
                host.run_until(PATIENCE, selected),
                "bundle.zip is written and selected"
            );
            assert_eq!(
                std::fs::read_to_string(&existing).expect("existing archive intact"),
                "keep original"
            );
        },
    );
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

fn assert_compress_controls_named(a11y: &serde_json::Value) {
    let expected = [
        ("text-input", "Archive name", None),
        ("password-input", "Password", None),
        ("password-input", "Confirm password", None),
        ("button", "ZIP", Some("checked")),
        ("button", "TAR.GZ", Some("unchecked")),
        ("button", "Password protected", Some("checked")),
        ("button", "Show password", None),
    ];
    for (role, name, state) in expected {
        assert!(
            has_a11y_node(a11y, role, name, state),
            "{role} {name:?} {state:?}"
        );
    }
}

#[test]
fn accessible_controls_are_named_and_backdrop_dismissal_from_menu_returns_list_focus() {
    hosted(
        MODULE,
        "accessible_controls_are_named_and_backdrop_dismissal_from_menu_returns_list_focus",
        |mut host| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());

            let readme = folder.path().join("readme.md");
            let id = shell::id_of(host.app(), &Location::local(&readme));
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                assert_eq!(commands::run(app, "view-list", On::Focus, tx), Outcome::Ran);
                app.browser
                    .select_entries_by_name_at(0, &[String::from("readme.md")]);
                shell::settle(app, tx);
                tx.focus(crate::list::VIEW, true);
            });
            idle(&mut host);

            let row_key = strata_components::collection::element::row_key(id);
            let (rx, ry) = center_of(&mut host, &row_key);
            host.runtime().pointer_move(rx, ry);
            host.runtime().secondary_down(rx, ry);
            host.turn();
            host.runtime().secondary_up(rx, ry);
            idle(&mut host);
            let (mx, my) =
                a11y_center(&host.tree(), "menu-item", "Compress…").expect("Compress… menu row");
            click_at(&mut host, mx, my);
            assert!(host.app().compress.open);

            click(&mut host, "compress-1-protection-password");
            assert_compress_controls_named(&host.tree());

            click_at(&mut host, 5.0, 5.0);
            let closed = |h: &Offscreen| {
                !h.app().compress.open
                    && !h
                        .elements()
                        .iter()
                        .any(|el| el.key.as_deref() == Some("compress-1-dialog"))
            };
            assert!(host.run_until(PATIENCE, closed), "Compress dialog closed");
            press(&mut host, Key::Down, Mods::default());
            assert_eq!(
                host.app()
                    .browser()
                    .selected_entries()
                    .first()
                    .map(|entry| entry.display_name.as_str()),
                Some("todo.txt")
            );
        },
    );
}

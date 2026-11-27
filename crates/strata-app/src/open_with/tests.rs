// SPDX-License-Identifier: MIT

use std::{collections::HashSet, ffi::OsStr, os::unix::fs::PermissionsExt, path::Path};

use gio::prelude::*;
use rune::{Key, KeyEvent, Mods, Tx};
use strata_components::contract::Rect;
use strata_core::model::Location;

use super::{
    launch::command_line_at,
    recent::{MAX_PER_TYPE, State, load_from, parse_state, prune_unknown, record_in, save_to},
};
use crate::{
    App,
    commands::{self, On, Outcome},
    error_dialog,
    host::offscreen::Offscreen,
    menus, shell,
    startup::{self, UiThread},
    test_support::{PATIENCE, fixture, in_own_process, send_key},
};

const MODULE: &str = "open_with::tests";

fn types(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn known(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn recent_history_round_trips_and_matches_toml_crate_format() {
    let dir = tempfile::tempdir().expect("history dir");
    let path = dir.path().join("recent-apps.toml");
    let mut state = State::default();
    record_in(&mut state, &types(&["text/plain"]), "alpha.desktop");
    record_in(
        &mut state,
        &types(&["text/plain", "image/png"]),
        "beta.desktop",
    );
    for index in 0..MAX_PER_TYPE + 2 {
        record_in(
            &mut state,
            &types(&["application/pdf"]),
            &format!("pdf-{index}.desktop"),
        );
    }
    prune_unknown(
        &mut state,
        &known(&[
            "alpha.desktop",
            "beta.desktop",
            "pdf-6.desktop",
            "pdf-5.desktop",
        ]),
    );
    save_to(&path, &mut state);

    let raw = std::fs::read_to_string(&path).expect("saved history");
    let parsed_toml: toml::Value = toml::from_str(&raw).expect("valid TOML for GTK reader");
    assert_eq!(parsed_toml["version"].as_integer(), Some(1));

    let restored = load_from(&path);
    assert_eq!(
        restored.recent_ids(&types(&["text/plain", "image/png"])),
        ["beta.desktop", "alpha.desktop"]
    );
    assert_eq!(
        restored.recent_ids(&types(&["application/pdf"])),
        ["pdf-6.desktop", "pdf-5.desktop"]
    );

    let inline = format!(
        "version = 1\n[types]\n\"text/plain\" = [{{ app = \"beta.desktop\", seq = {} }}, {{ app = \"alpha.desktop\", seq = {} }}]\n",
        u64::MAX,
        u64::MAX - 1
    );
    let mut exhausted = parse_state(&inline).expect("inline table history");
    record_in(&mut exhausted, &types(&["text/plain"]), "gamma.desktop");
    assert_eq!(
        exhausted.recent_ids(&types(&["text/plain"])),
        ["gamma.desktop", "beta.desktop", "alpha.desktop"]
    );
    assert!(parse_state("not = [valid").is_err());
    std::fs::write(&path, "version = 99\n").expect("unsupported version");
    assert!(
        load_from(&path)
            .recent_ids(&types(&["text/plain"]))
            .is_empty()
    );
}

#[test]
fn timed_player_command_lines_insert_start_flags_before_placeholders() {
    for (exec, seconds, expected) in [
        ("mpv -- %U", 12.5, Some("mpv --start=12.500 -- %U")),
        ("vlc %f", 3.0, Some("vlc --start-time=3.000 %f")),
        (
            "env LC_ALL=C flatpak run io.mpv.Mpv @@u %U @@",
            7.25,
            Some("env LC_ALL=C flatpak run io.mpv.Mpv --start=7.250 @@u %U @@"),
        ),
        ("gedit %U", 5.0, None),
    ] {
        assert_eq!(
            command_line_at(exec, seconds).as_deref(),
            expected,
            "{exec}"
        );
    }
}

fn hosted_with_apps(name: &str, run: impl FnOnce(Offscreen, std::path::PathBuf)) {
    let data_dirs = tempfile::tempdir().expect("isolated XDG_DATA_DIRS");
    if Path::new("/usr/share/mime").exists() {
        std::os::unix::fs::symlink("/usr/share/mime", data_dirs.path().join("mime"))
            .expect("link mime database");
    }
    in_own_process(
        &format!("{MODULE}::{name}"),
        &[("XDG_DATA_DIRS", data_dirs.path().as_os_str())],
        || {
            let log = install_fake_apps();
            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            run(Offscreen::new(ui, settings), log);
        },
    );
}

fn install_fake_apps() -> std::path::PathBuf {
    let data = glib::user_data_dir();
    let apps = data.join("applications");
    std::fs::create_dir_all(&apps).expect("applications dir");
    let config = glib::user_config_dir();
    std::fs::create_dir_all(&config).expect("config dir");
    let bin = data.join("record-open");
    let log = data.join("opened.log");
    std::fs::write(
        &bin,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\n", log.display()),
    )
    .expect("recorder script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    for (id, display, mime) in [
        (
            "strata-alpha.desktop",
            "Alpha Viewer",
            "text/plain;text/markdown;",
        ),
        (
            "strata-beta.desktop",
            "Beta Editor",
            "text/plain;text/markdown;",
        ),
    ] {
        std::fs::write(
            apps.join(id),
            format!(
                "[Desktop Entry]\nType=Application\nName={display}\nExec={} {id} %U\nMimeType={mime}\n",
                bin.display()
            ),
        )
        .expect("desktop file");
    }
    std::fs::write(
        config.join("mimeapps.list"),
        "[Default Applications]\ntext/plain=strata-alpha.desktop;\ntext/markdown=strata-alpha.desktop;\n[Added Associations]\ntext/plain=strata-alpha.desktop;strata-beta.desktop;\ntext/markdown=strata-alpha.desktop;strata-beta.desktop;\n",
    )
    .expect("mimeapps.list");
    log
}

fn open_fixture(host: &mut Offscreen, root: &Path) {
    host.app().open(Location::local(root));
    assert!(
        host.run_until(PATIENCE, |h| h.app().columns.listed(0)),
        "fixture listed"
    );
    while host.turn() {}
}

fn open_chooser_for_todo(host: &mut Offscreen, root: &Path) {
    let todo_loc = Location::local(root.join("todo.txt"));
    let todo_id = shell::id_of(host.app(), &todo_loc);
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        menus::open(
            app,
            menus::Request {
                depth: 0,
                entry: Some(todo_id),
                anchor: Rect::new(12.0, 12.0, 0.0, 0.0),
            },
            tx,
        );
    });
    assert!(
        host.run_until(PATIENCE, |h| h.app().open_with.menu_selection.is_some()),
        "context menu MIME lookup settled"
    );
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        app.menu.state.close();
        assert_eq!(
            commands::run(app, "open-with", On::Row((0, todo_id)), tx),
            Outcome::Ran
        );
    });
    while host.turn() {}
    assert!(host.app().open_with.open);
}

fn press_key(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
    while host.turn() {}
}

#[test]
fn chooser_filters_clears_on_escape_and_saves_default_on_open() {
    hosted_with_apps(
        "chooser_filters_clears_on_escape_and_saves_default_on_open",
        |mut host, log| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());
            open_chooser_for_todo(&mut host, folder.path());

            for ch in "no-match".chars() {
                press_key(&mut host, Key::Text(ch.to_string()));
            }
            assert_eq!(
                host.app().open_with.form.as_ref().expect("form").selected,
                None
            );

            // First Escape clears the search query without closing the chooser.
            press_key(&mut host, Key::Escape);
            assert!(host.app().open_with.open);
            assert!(
                host.app()
                    .open_with
                    .form
                    .as_ref()
                    .expect("form")
                    .search
                    .text
                    .is_empty()
            );

            // Down selects Beta Editor; toggling Always use and pressing Enter saves default & launches.
            press_key(&mut host, Key::Down);
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                super::toggle_always_use(app);
            });
            press_key(&mut host, Key::Enter);
            assert!(!host.app().open_with.open);
            // The launcher writes its line after creating the log: wait for the line itself.
            let launched = |_: &Offscreen| {
                std::fs::read_to_string(&log)
                    .is_ok_and(|opened| opened.contains("strata-beta.desktop"))
            };
            assert!(
                host.run_until(PATIENCE, launched),
                "Beta Editor launched todo.txt: {:?}",
                std::fs::read_to_string(&log)
            );
            let info = gio::File::for_path(folder.path().join("todo.txt"))
                .query_info(
                    "standard::content-type",
                    gio::FileQueryInfoFlags::NONE,
                    gio::Cancellable::NONE,
                )
                .expect("todo.txt info");
            let mime = info.content_type().expect("todo.txt content type");
            let default = gio::AppInfo::default_for_type(&mime, false).expect("saved default");
            assert_eq!(default.id().as_deref(), Some("strata-beta.desktop"));
        },
    );
}

#[test]
fn multi_selection_menu_adds_shared_open_and_refuses_trash_open() {
    hosted_with_apps(
        "multi_selection_menu_adds_shared_open_and_refuses_trash_open",
        |mut host, log| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());

            let names = ["readme.md".to_owned(), "todo.txt".to_owned()];
            assert!(host.app().browser().select_entries_by_name_at(0, &names));
            while host.turn() {}
            let todo_id =
                shell::id_of(host.app(), &Location::local(folder.path().join("todo.txt")));
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                menus::open(
                    app,
                    menus::Request {
                        depth: 0,
                        entry: Some(todo_id),
                        anchor: Rect::new(12.0, 12.0, 0.0, 0.0),
                    },
                    tx,
                );
            });
            let has_open = host.run_until(PATIENCE, |h| {
                h.app().menu.state.model().is_some_and(|model| {
                    model
                        .sections
                        .iter()
                        .flat_map(|s| &s.items)
                        .any(|item| &*item.action == "open")
                })
            });
            assert!(
                has_open,
                "shared default added Open row to multi-selection menu"
            );

            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                app.menu.state.close();
                assert_eq!(
                    commands::run(app, "open", On::Row((0, todo_id)), tx),
                    Outcome::Ran
                );
            });
            let lines = |log: &Path| {
                std::fs::read_to_string(log).map_or(0, |opened| opened.lines().count())
            };
            assert!(
                host.run_until(PATIENCE, |_| lines(&log) >= 3),
                "shared default launched"
            );
            while host.turn() {}
            assert_eq!(lines(&log), 3, "{:?}", std::fs::read_to_string(&log));

            // Trash locations are refused synchronously with an error dialog.
            let trash = Location::uri("trash:///todo.txt");
            host.runtime().update(move |app: &mut App, _: &mut Tx| {
                super::open_location_at(app, &trash, None);
            });
            while host.turn() {}
            let (_, _, title, _, detail) =
                error_dialog::shown_details(host.app()).expect("trash refusal");
            assert_eq!(title, "Unable to open item");
            assert!(detail.contains("Trash"), "{detail}");
            let _ = OsStr::new("");
        },
    );
}

// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use rune::Tx;
use strata_core::{model::Location, storage::writer};
use strata_desktop::tabs_session::{self, RestoredSession};

use super::{persist_session, restore_session};
use crate::{
    App,
    host::offscreen::Offscreen,
    launch::{self, Initial},
    shell::{
        self,
        tabs::{close, located, move_to, open, show, shown_tab, tab_browser, tabs},
    },
    startup::{self, UiThread},
    tab_strip,
    test_support::{PATIENCE, in_own_process, view::idle},
};

const MODULE: &str = "shell::tabs::session::tests";

fn session_file() -> PathBuf {
    strata_core::storage::config_directory().join("tabs.toml")
}

fn write_settings(extra: &str) {
    let dir = strata_core::storage::config_directory();
    std::fs::create_dir_all(&dir).expect("config folder");
    let body = format!("mode = \"theme\"\ntheme = \"tokyo-night\"\n{extra}");
    std::fs::write(dir.join("settings.toml"), body).expect("settings.toml");
}

fn seed_session(locations: &[Location], active: usize) {
    tabs_session::save(locations, active);
    writer::flush_at_exit();
}

fn saved_session() -> Option<RestoredSession> {
    writer::flush_at_exit();
    tabs_session::load_restorable()
}

fn make_folders(root: &Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| {
            let folder = root.join(name);
            std::fs::create_dir_all(folder.join("sub")).expect("subfolder");
            std::fs::write(folder.join("file.txt"), "file").expect("file");
            folder
        })
        .collect()
}

fn plain_launch(name: &str, prepare: impl FnOnce(), run: impl FnOnce(Offscreen)) {
    in_own_process(&format!("{MODULE}::{name}"), &[], || {
        prepare();
        let settings = startup::load_settings();
        let initial = launch::initial(None, &settings).expect("plain launch");
        let ui = UiThread::claim().expect("the UI thread");
        let mut host = Offscreen::new(ui, settings);
        host.start(initial);
        run(host);
    });
}

fn target_launch(name: &str, prepare: impl FnOnce() -> PathBuf, run: impl FnOnce(Offscreen)) {
    in_own_process(&format!("{MODULE}::{name}"), &[], || {
        let target = prepare();
        let settings = startup::load_settings();
        let initial = launch::initial(Some(target.as_os_str()), &settings).expect("target launch");
        let ui = UiThread::claim().expect("the UI thread");
        let mut host = Offscreen::new(ui, settings);
        host.start(initial);
        run(host);
    });
}

fn act<T>(host: &mut Offscreen, f: impl FnOnce(&mut App, &mut Tx) -> T) -> T {
    let mut out = None;
    host.runtime().update(|app, tx| out = Some(f(app, tx)));
    idle(host);
    out.expect("update ran")
}

fn browser_locations(host: &Offscreen) -> Vec<Option<Location>> {
    tabs(host.app())
        .iter()
        .map(|&key| tab_browser(host.app(), key).and_then(|b| b.active_location()))
        .collect()
}

fn wait_locations(host: &mut Offscreen, expected: &[Option<Location>]) {
    let reached = host.run_until(PATIENCE, |host| browser_locations(host) == expected);
    assert!(
        reached,
        "expected {expected:?}, got {:?}",
        browser_locations(host)
    );
    idle(host);
}

fn focused_key(host: &mut Offscreen) -> Option<String> {
    let mut focused = None;
    host.runtime()
        .update(|_, tx| focused = tx.focused().map(str::to_owned));
    focused
}

#[test]
fn plain_launch_without_saved_session_opens_default_folder_and_forgets_missing_default() {
    plain_launch(
        "plain_launch_without_saved_session_opens_default_folder_and_forgets_missing_default",
        || {
            let default = strata_core::storage::config_directory().join("workspace");
            std::fs::create_dir_all(&default).expect("default folder");
            write_settings(&format!(
                "default_directory = {:?}\n",
                default.to_string_lossy()
            ));
        },
        |mut host| {
            let default = strata_core::storage::config_directory().join("workspace");
            let wanted = Location::local(&default);
            wait_locations(&mut host, &[Some(wanted.clone())]);
            assert_eq!(tabs(host.app()).len(), 1);
            assert_eq!(
                focused_key(&mut host).as_deref(),
                shell::view_focus(host.app())
            );
            let saved = saved_session().expect("plain launch saves once opened");
            assert_eq!((saved.tabs, saved.active), (vec![wanted], 0));

            let gone = strata_core::storage::config_directory().join("deleted-default");
            let home = Location::local(launch::home_directory());
            act(&mut host, |app, tx| {
                shell::saved(app).change(|p| p.default_directory = Some(gone));
                let empty = open(app, None, tabs(app).len(), tx);
                super::open_default_folder(app, empty, tx);
            });
            let reached = host.run_until(PATIENCE, |host| {
                host.app().browser().active_location().as_ref() == Some(&home)
            });
            assert!(reached, "missing default falls back to home");
            assert_eq!(shell::preferences(host.app()).default_directory, None);
        },
    );
}

#[test]
fn saved_session_restores_tabs_in_order_selects_active_and_focuses_each_listing() {
    plain_launch(
        "saved_session_restores_tabs_in_order_selects_active_and_focuses_each_listing",
        || {
            let root = strata_core::storage::config_directory().join("session-root");
            let folders = make_folders(&root, &["first", "second", "third"]);
            seed_session(
                &[
                    Location::local(&folders[0]),
                    Location::local(&folders[1]),
                    Location::local(&folders[2]),
                ],
                2,
            );
        },
        |mut host| {
            let root = strata_core::storage::config_directory().join("session-root");
            let expected = [
                Some(Location::local(root.join("first"))),
                Some(Location::local(root.join("second"))),
                Some(Location::local(root.join("third"))),
            ];
            wait_locations(&mut host, &expected);
            let order = tabs(host.app()).to_vec();
            assert_eq!(order.len(), 3);
            assert_eq!(shown_tab(host.app()), order[2]);
            assert_eq!(host.app().title(), "third — Strata");
            assert_eq!(
                focused_key(&mut host).as_deref(),
                shell::view_focus(host.app())
            );
            #[cfg(feature = "qa")]
            for (_, accounted) in shell::hidden_tabs(host.app()) {
                assert!(accounted, "hidden restored tabs ran nothing");
            }

            // Showing each previously hidden restored tab for the first time focuses its listing.
            for &key in &order[..2] {
                assert!(act(&mut host, |app, tx| tab_strip::select(app, key, tx)));
                assert_eq!(
                    focused_key(&mut host).as_deref(),
                    shell::view_focus(host.app())
                );
            }
        },
    );
}

#[test]
fn plain_launch_persists_tab_changes_in_strip_order() {
    plain_launch(
        "plain_launch_persists_tab_changes_in_strip_order",
        || {
            let root = strata_core::storage::config_directory().join("persist-root");
            let folders = make_folders(&root, &["alpha", "beta", "gamma"]);
            write_settings(&format!(
                "default_directory = {:?}\n",
                folders[0].to_string_lossy()
            ));
        },
        |mut host| {
            let root = strata_core::storage::config_directory().join("persist-root");
            let alpha = Location::local(root.join("alpha"));
            let beta = Location::local(root.join("beta"));
            let gamma = Location::local(root.join("gamma"));
            wait_locations(&mut host, &[Some(alpha.clone())]);

            act(&mut host, |app, tx| {
                tab_strip::open_in_new_tab(app, beta.clone(), tx);
                tab_strip::open_in_new_tab(app, gamma.clone(), tx);
            });
            wait_locations(
                &mut host,
                &[Some(alpha.clone()), Some(beta.clone()), Some(gamma.clone())],
            );
            let keys = tabs(host.app()).to_vec();

            // Select middle tab (`beta`) and reorder `alpha` to the end -> `[beta, gamma, alpha]`.
            act(&mut host, |app, tx| {
                show(app, keys[1], tx);
                move_to(app, keys[0], 3);
            });
            let saved = saved_session().expect("reordered session");
            assert_eq!(saved.tabs, vec![beta.clone(), gamma.clone(), alpha]);
            assert_eq!(saved.active, 0);

            // Closing `beta` selects `gamma` and persists `[gamma, alpha]` with `active = 0`.
            act(&mut host, |app, tx| close(app, keys[1], tx));
            let saved = saved_session().expect("after close");
            assert_eq!((saved.tabs.len(), saved.active), (2, 0));
            assert_eq!(saved.tabs[0], gamma);
        },
    );
}

#[test]
fn a_later_bare_launch_restores_the_session_and_persists_its_tabs() {
    in_own_process(
        &format!("{MODULE}::a_later_bare_launch_restores_the_session_and_persists_its_tabs"),
        &[],
        || {
            let root = strata_core::storage::config_directory().join("present-root");
            let folders = make_folders(&root, &["first", "second", "third"]);
            let saved: Vec<_> = folders[..2].iter().map(Location::local).collect();
            seed_session(&saved, 1);
            let settings = startup::load_settings();
            let initial = launch::initial_for(crate::application::Opening::Present, &settings);
            let ui = UiThread::claim().expect("the UI thread");
            let mut host = Offscreen::new(ui, settings);

            host.start(initial);
            wait_locations(&mut host, &[Some(saved[0].clone()), Some(saved[1].clone())]);
            let third = Location::local(&folders[2]);
            act(&mut host, |app, tx| {
                tab_strip::open_in_new_tab(app, third.clone(), tx);
            });
            wait_locations(
                &mut host,
                &[
                    Some(saved[0].clone()),
                    Some(saved[1].clone()),
                    Some(third.clone()),
                ],
            );

            let session = saved_session().expect("the window's tabs saved");
            assert_eq!(
                session.tabs,
                vec![saved[0].clone(), saved[1].clone(), third]
            );
        },
    );
}

#[test]
fn explicit_target_windows_do_not_restore_or_clobber_the_saved_session() {
    target_launch(
        "explicit_target_windows_do_not_restore_or_clobber_the_saved_session",
        || {
            let root = strata_core::storage::config_directory().join("explicit-root");
            let folders = make_folders(&root, &["kept-a", "kept-b", "other", "extra"]);
            seed_session(
                &[Location::local(&folders[0]), Location::local(&folders[1])],
                1,
            );
            folders[2].clone()
        },
        |mut host| {
            let root = strata_core::storage::config_directory().join("explicit-root");
            let other = Location::local(root.join("other"));
            let extra = Location::local(root.join("extra"));
            wait_locations(&mut host, &[Some(other.clone())]);
            assert_eq!(tabs(host.app()).len(), 1);

            act(&mut host, |app, tx| {
                tab_strip::open_in_new_tab(app, extra.clone(), tx);
            });
            wait_locations(&mut host, &[Some(other), Some(extra)]);
            let keys = tabs(host.app()).to_vec();
            act(&mut host, |app, tx| {
                move_to(app, keys[0], 2);
                close(app, keys[1], tx);
            });

            let saved = saved_session().expect("original session untouched");
            assert_eq!(
                (saved.tabs, saved.active),
                (
                    vec![
                        Location::local(root.join("kept-a")),
                        Location::local(root.join("kept-b")),
                    ],
                    1,
                )
            );
        },
    );
}

#[test]
fn restoring_does_not_save_midway_and_empty_tabs_never_clobber_session() {
    in_own_process(
        &format!("{MODULE}::restoring_does_not_save_midway_and_empty_tabs_never_clobber_session"),
        &[],
        || {
            let root = strata_core::storage::config_directory().join("midway-root");
            let folders = make_folders(&root, &["first", "second", "third", "sentinel"]);
            let expected: Vec<_> = folders[..3].iter().map(Location::local).collect();
            let sentinel = vec![Location::local(&folders[3])];
            seed_session(&expected, 1);

            let settings = startup::load_settings();
            let Initial::Plain(Some(restored)) =
                launch::initial(None, &settings).expect("plain launch")
            else {
                panic!("expected restored session at start-up");
            };
            let ui = UiThread::claim().expect("the UI thread");
            let mut host = Offscreen::new(ui, settings);

            seed_session(&sentinel, 0);
            host.runtime().update(|app, tx| {
                app.window.tabs.persist = true;
                open(app, None, 1, tx);
                persist_session(app);
            });
            assert!(
                tabs(host.app())
                    .iter()
                    .all(|&k| located(host.app(), k).is_none())
            );
            assert_eq!(saved_session().map(|s| s.tabs), Some(sentinel.clone()));

            host.runtime().update(|app, tx| {
                let extra = tabs(app)[1];
                close(app, extra, tx);
                restore_session(app, restored, tx);
            });
            assert_eq!(tabs(host.app()).len(), 3);
            assert_eq!(
                saved_session().map(|s| s.tabs),
                Some(sentinel),
                "no partial session was written during restore_session"
            );

            // Once GLib delivers the folder probes, the full restored session persists cleanly.
            let wanted: Vec<_> = expected.iter().cloned().map(Some).collect();
            wait_locations(&mut host, &wanted);
            let saved = saved_session().expect("restored session saved after probes");
            assert_eq!((saved.tabs, saved.active), (expected, 1));
        },
    );
}

#[test]
fn disabled_restore_ignores_and_removes_saved_session_and_live_toggle_clears_file() {
    plain_launch(
        "disabled_restore_ignores_and_removes_saved_session_and_live_toggle_clears_file",
        || {
            let root = strata_core::storage::config_directory().join("disabled-root");
            let folders = make_folders(&root, &["kept-a", "kept-b"]);
            write_settings("restore_tabs = false\n");
            seed_session(
                &[Location::local(&folders[0]), Location::local(&folders[1])],
                1,
            );
        },
        |mut host| {
            let home = Location::local(launch::home_directory());
            wait_locations(&mut host, &[Some(home.clone())]);
            assert_eq!(tabs(host.app()).len(), 1);
            assert!(saved_session().is_none());
            assert!(
                !session_file().exists(),
                "disabled restore removes tabs.toml"
            );

            let root = strata_core::storage::config_directory().join("disabled-root");
            let second = Location::local(root.join("kept-a"));
            act(&mut host, |app, tx| {
                shell::saved(app).change(|p| p.restore_tabs = true);
                tab_strip::open_in_new_tab(app, second.clone(), tx);
            });
            wait_locations(&mut host, &[Some(home), Some(second)]);
            assert!(session_file().exists(), "enabled restore wrote tabs.toml");

            act(&mut host, |app, _| {
                shell::saved(app).change(|p| p.restore_tabs = false);
            });
            assert!(saved_session().is_none());
            assert!(
                !session_file().exists(),
                "turning restore_tabs off live removes tabs.toml immediately"
            );
        },
    );
}

#[test]
fn missing_restored_folders_drop_their_tabs_without_reopen_history() {
    plain_launch(
        "missing_restored_folders_drop_their_tabs_without_reopen_history",
        || {
            let root = strata_core::storage::config_directory().join("missing-some");
            let folders = make_folders(&root, &["first", "second"]);
            let file = root.join("not-a-folder");
            std::fs::write(&file, "content").expect("regular file");
            seed_session(
                &[
                    Location::local(root.join("gone")),
                    Location::local(&folders[0]),
                    Location::local(&file),
                    Location::local(&folders[1]),
                ],
                3,
            );
        },
        |mut host| {
            let root = strata_core::storage::config_directory().join("missing-some");
            let first = Location::local(root.join("first"));
            let second = Location::local(root.join("second"));
            wait_locations(&mut host, &[Some(first.clone()), Some(second.clone())]);
            let keys = tabs(host.app()).to_vec();
            assert_eq!(keys.len(), 2);
            assert_eq!(shown_tab(host.app()), keys[1]);

            let saved = saved_session().expect("remaining tabs saved");
            assert_eq!((saved.tabs, saved.active), (vec![first, second], 1));

            // Discarded missing tabs were not closed by the user: reopen restores nothing.
            act(&mut host, tab_strip::reopen);
            assert_eq!(tabs(host.app()).len(), 2);
        },
    );
}

#[test]
fn when_every_restored_folder_is_missing_the_last_tab_opens_default_folder() {
    plain_launch(
        "when_every_restored_folder_is_missing_the_last_tab_opens_default_folder",
        || {
            let root = strata_core::storage::config_directory().join("missing-all");
            let fallback = make_folders(&root, &["fallback"]).pop().expect("fallback");
            write_settings(&format!(
                "default_directory = {:?}\n",
                fallback.to_string_lossy()
            ));
            seed_session(
                &[
                    Location::local(root.join("gone-one")),
                    Location::local(root.join("gone-two")),
                ],
                1,
            );
        },
        |mut host| {
            let fallback = Location::local(
                strata_core::storage::config_directory().join("missing-all/fallback"),
            );
            wait_locations(&mut host, &[Some(fallback.clone())]);
            assert_eq!(tabs(host.app()).len(), 1);
            let saved = saved_session().expect("fallback saved");
            assert_eq!((saved.tabs, saved.active), (vec![fallback], 0));
        },
    );
}

#[test]
fn unsupported_session_version_is_refused_and_overwritten_with_version_one() {
    plain_launch(
        "unsupported_session_version_is_refused_and_overwritten_with_version_one",
        || {
            let root = strata_core::storage::config_directory().join("version-root");
            let fallback = make_folders(&root, &["alpha"]).pop().expect("alpha");
            write_settings(&format!(
                "default_directory = {:?}\n",
                fallback.to_string_lossy()
            ));
            std::fs::write(
                session_file(),
                concat!(
                    "version = 2\n",
                    "active = 0\n",
                    "[[tabs]]\n",
                    "kind = \"path\"\n",
                    "value = \"/tmp/future-format\"\n",
                ),
            )
            .expect("v2 tabs.toml");
        },
        |mut host| {
            let alpha = Location::local(
                strata_core::storage::config_directory().join("version-root/alpha"),
            );
            wait_locations(&mut host, &[Some(alpha.clone())]);
            assert_eq!(tabs(host.app()).len(), 1);

            let saved = saved_session().expect("version 1 written over version 2");
            assert_eq!((saved.tabs, saved.active), (vec![alpha], 0));
            let raw: toml::Value =
                toml::from_str(&std::fs::read_to_string(session_file()).expect("tabs.toml"))
                    .expect("valid toml");
            assert_eq!(
                raw.get("version").and_then(toml::Value::as_integer),
                Some(1)
            );
        },
    );
}

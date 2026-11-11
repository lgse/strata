// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use strata_core::model::{Location, NameComparison, SortDirection, SortKey, ViewPreferences};

use super::Offscreen;
use crate::{
    launch,
    startup::{self, UiThread},
};

const PATIENCE: Duration = Duration::from_secs(30);

/// Runs the test `name` on a host in a process of its own: the default GLib context, the
/// start-up sequence and the saved settings belong to one process. `prepare` runs there
/// first, before anything is read.
fn hosted_after(name: &str, prepare: impl FnOnce(), run: impl FnOnce(Offscreen)) {
    strata_test_support::isolated_test(
        &format!("host::offscreen::tests::{name}"),
        std::iter::empty::<(&str, &str)>(),
        || true,
        || {
            prepare();
            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            run(Offscreen::new(ui, &settings));
        },
    );
}

fn hosted(name: &str, run: impl FnOnce(Offscreen)) {
    hosted_after(name, || {}, run);
}

fn folder_of(names: &[&str]) -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    for name in names {
        std::fs::write(folder.path().join(name), name).expect("file");
    }
    folder
}

fn listed(host: &Offscreen) -> usize {
    host.app()
        .browser()
        .with_column_entries(0, <[_]>::len)
        .unwrap_or(0)
}

#[test]
fn a_glib_idle_callback_reaches_the_ui_state_within_one_frame() {
    hosted(
        "a_glib_idle_callback_reaches_the_ui_state_within_one_frame",
        |mut host| {
            let folder = folder_of(&[]);
            let location = Location::local(folder.path());
            let browser = host.app().browser().clone();
            let asked = location.clone();
            glib::idle_add_local_once(move || browser.navigate(asked));
            assert_eq!(host.app().location(), None);

            host.turn();

            assert_eq!(host.app().location(), Some(&location));
        },
    );
}

// The guard is a `debug_assert!`: a release build would pass this without checking anything.
#[cfg(debug_assertions)]
#[test]
fn the_first_listing_after_start_up_stays_off_the_ui_thread_guard() {
    hosted(
        "the_first_listing_after_start_up_stays_off_the_ui_thread_guard",
        |mut host| {
            let folder = folder_of(&["one", "two", "three"]);
            host.app().open(Location::local(folder.path()));

            let complete = host.run_until(PATIENCE, |app| {
                app.browser().with_column_entries(0, <[_]>::len) == Some(3)
            });

            assert!(complete, "listed {} of 3 entries", listed(&host));
        },
    );
}

#[test]
fn a_launch_opens_its_directory_and_titles_the_window_for_it() {
    hosted(
        "a_launch_opens_its_directory_and_titles_the_window_for_it",
        |mut host| {
            let folder = folder_of(&[]);
            let nested = folder.path().join("Reports");
            std::fs::create_dir(&nested).expect("nested folder");
            let home = std::env::var_os("HOME").expect("the isolated home");
            std::env::set_current_dir(folder.path()).expect("launch folder");
            let uri = format!("file://{}", nested.display());
            let cases = [
                (Some(nested.as_os_str()), nested.clone(), "Reports — Strata"),
                (Some("Reports".as_ref()), nested.clone(), "Reports — Strata"),
                (Some(uri.as_ref()), nested.clone(), "Reports — Strata"),
                (None, home.clone().into(), "home — Strata"),
            ];
            for (argument, directory, title) in cases {
                host.app().open(Location::local(folder.path()));
                let expected = Location::local(directory);

                host.app()
                    .open(launch::location(argument).expect("a location"));
                let shown = host.run_until(PATIENCE, |app| app.location() == Some(&expected));

                assert!(shown, "{argument:?} never showed");
                assert_eq!(
                    host.app().browser().active_location(),
                    Some(expected),
                    "{argument:?}"
                );
                assert_eq!(host.app().title(), title, "{argument:?}");
            }
        },
    );
}

#[test]
fn waiting_ends_when_glib_has_work_or_a_rune_timer_is_due() {
    hosted(
        "waiting_ends_when_glib_has_work_or_a_rune_timer_is_due",
        |mut host| {
            while host.turn() {}
            let limit = Duration::from_secs(20);

            let context = glib::MainContext::default();
            let sender = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                context.invoke(|| {});
            });
            let started = Instant::now();
            host.wait(limit);
            assert!(started.elapsed() < limit, "GLib work did not end the wait");
            sender.join().expect("sender");

            host.runtime
                .update(|_, tx| tx.wake_after(Duration::from_millis(50)));
            host.turn();
            let started = Instant::now();
            host.wait(limit);
            assert!(
                started.elapsed() < limit,
                "Rune's timer did not end the wait"
            );
        },
    );
}

#[test]
fn saved_settings_reach_the_browser_and_the_language() {
    hosted_after(
        "saved_settings_reach_the_browser_and_the_language",
        || {
            let folder = strata_core::storage::config_directory();
            std::fs::create_dir_all(&folder).expect("settings folder");
            std::fs::write(
                folder.join("settings.toml"),
                concat!(
                    "mode = \"theme\"\n",
                    "theme = \"tokyo-night\"\n",
                    "language = \"fr\"\n",
                    "show_hidden = true\n",
                    "folders_first = false\n",
                    "sort_key = \"size\"\n",
                    "sort_direction = \"descending\"\n",
                    "natural_sort = false\n",
                    "case_sensitive_sort = true\n",
                    // Not a unit system: the store drops this entry and keeps the rest.
                    "unit_system = 7\n",
                ),
            )
            .expect("settings");
        },
        |host| {
            assert_eq!(
                host.app().browser().preferences(),
                ViewPreferences {
                    show_hidden: true,
                    folders_first: false,
                    sort_key: SortKey::Size,
                    sort_direction: SortDirection::Descending,
                    name_comparison: NameComparison {
                        natural: false,
                        case_sensitive: true,
                    },
                }
            );
            assert_eq!(&*rust_i18n::locale(), "fr");
        },
    );
}

// SPDX-License-Identifier: MIT

use std::time::Duration;

use strata_core::model::{Location, SortDirection, SortKey, SortOptions};
use strata_desktop::settings::{
    folder_views::{FolderViews, key_for_location},
    folder_views_path,
};

use super::{Tick, Timer};
use crate::test_support::{PATIENCE, hosted, hosted_after};

const SAVE: Duration = Duration::from_millis(500);
const USE: Duration = Duration::from_secs(60);

/// GTK's one timer (`schedule_folder_views_write`): a pending write keeps its time unless a
/// shorter wait is asked for, a longer or equal wait asked meanwhile changes nothing, and the
/// time is stamped when the write is asked.
#[test]
fn a_pending_write_keeps_its_time_unless_a_shorter_wait_is_asked() {
    let mut timer = Timer::default();
    assert_eq!(timer.tick(0.0), Tick::Idle, "nothing asked");

    assert_eq!(
        timer.ask(1.0, USE),
        Some(61.0),
        "a first write, stamped when asked"
    );
    assert_eq!(timer.tick(1.5), Tick::Wait(Duration::from_secs_f64(59.5)));
    assert_eq!(
        timer.ask(2.0, SAVE),
        Some(2.5),
        "a shorter wait replaces it"
    );
    assert_eq!(timer.ask(2.1, USE), None, "a longer wait keeps the time");
    assert_eq!(timer.ask(2.2, SAVE), None, "the same wait keeps the time");
    assert_eq!(timer.tick(2.25), Tick::Wait(Duration::from_millis(250)));
    assert_eq!(timer.tick(2.5), Tick::Due, "due");
    assert_eq!(timer.tick(3.0), Tick::Idle, "a write starts once");

    assert!(timer.ask(4.0, SAVE).is_some());
    timer.clear();
    assert_eq!(
        timer.tick(5.0),
        Tick::Idle,
        "a write started by hand clears the timer"
    );
}

/// The last write before the process ends: a change made just before it is in
/// `folder-views.toml` once the settings writer is flushed, with the 500 ms timer still
/// pending, and a second call (a quit after a close) starts no second write.
#[test]
fn the_exit_write_saves_a_change_the_timer_has_not_written() {
    hosted(
        "folder_views::tests",
        "the_exit_write_saves_a_change_the_timer_has_not_written",
        |mut host| {
            let folder = tempfile::tempdir().expect("folder");
            let key = key_for_location(&Location::local(folder.path())).expect("a key");
            let changed = key.clone();
            host.runtime().update(move |app, tx| {
                super::change(
                    app,
                    |views| views.update(&changed, |view| view.icons_size = Some(144)),
                    tx,
                );
            });
            let kept = host.app().window.folder_views();
            assert!(
                kept.timer.due.is_some(),
                "the timer's write is still pending"
            );

            super::flush_at_exit(&mut host.runtime().state);
            assert!(host.app().window.folder_views().exiting);
            super::flush_at_exit(&mut host.runtime().state);
            strata_core::storage::writer::flush_at_exit();

            let text = std::fs::read_to_string(folder_views_path()).expect("folder-views.toml");
            let (views, _) = FolderViews::parse(&text).expect("folder views");
            assert_eq!(views.view(&key).icons_size, Some(144));
        },
    );
}

/// A folder of the test's own configuration directory, so that start-up can find what was
/// saved for it: made before the window is.
fn sorted_folder() -> std::path::PathBuf {
    strata_core::storage::config_directory().join("sorted")
}

/// Three files whose sizes grow with their names, so a sort by size descending lists them the
/// other way round from a sort by name.
fn make_sorted_folder() {
    let folder = sorted_folder();
    std::fs::create_dir_all(&folder).expect("folder");
    for (name, size) in [("a.txt", 10), ("b.txt", 20), ("c.txt", 30)] {
        std::fs::write(folder.join(name), "x".repeat(size)).expect("file");
    }
}

/// Writes `folder-views.toml` with `folder`'s sort, as an earlier session saved it.
fn seed_folder_sort(folder: &std::path::Path, sort: SortOptions) {
    let key = key_for_location(&Location::local(folder)).expect("a key");
    let mut views = FolderViews::default();
    views.update(&key, |view| view.sort = Some(sort));
    let path = folder_views_path();
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("config folder");
    std::fs::write(path, views.to_toml().expect("toml")).expect("folder views");
}

/// The sort `folder-views.toml` holds for `folder`.
fn folder_sort_saved(folder: &std::path::Path) -> Option<Option<SortOptions>> {
    let text = std::fs::read_to_string(folder_views_path()).ok()?;
    let (views, _) = FolderViews::parse(&text).ok()?;
    let key = key_for_location(&Location::local(folder))?;
    Some(views.view(&key).sort)
}

/// The names column `depth` lists, in its order.
fn listed(host: &crate::host::offscreen::Offscreen, depth: usize) -> Vec<String> {
    let browser = &host.app().browser;
    (0..)
        .map_while(|position| browser.entry_at(depth, position))
        .map(|entry| entry.display_name)
        .collect()
}

/// A folder's sort saved by an earlier session is its sort from the moment its column is
/// made, before anything is listed, so its rows never show in another order first (GTK's
/// `set_folder_sorts` binding, bound before the first folder opens). Opening it counts as
/// using it, and a write is owed for that.
#[test]
fn a_saved_folder_sort_applies_on_its_first_column() {
    let size_down = (SortKey::Size, SortDirection::Descending, Default::default());
    hosted_after(
        "folder_views::tests",
        "a_saved_folder_sort_applies_on_its_first_column",
        || {
            make_sorted_folder();
            seed_folder_sort(&sorted_folder(), size_down);
        },
        |mut host| {
            host.app().open(Location::local(sorted_folder()));
            let made = host
                .app()
                .browser
                .column_preferences(0)
                .expect("the column");
            assert_eq!(
                made.sort_options(),
                size_down,
                "the column is made in its sort"
            );

            let first = host.run_until(PATIENCE, |host| !listed(host, 0).is_empty());
            assert!(first, "nothing listed");
            let rows = listed(&host, 0);
            let order = ["c.txt", "b.txt", "a.txt"];
            assert!(
                order
                    .iter()
                    .filter(|name| rows.iter().any(|row| row == *name))
                    .eq(rows.iter()),
                "the first rows listed are in the saved order: {rows:?}"
            );
            let whole = host.run_until(PATIENCE, |host| listed(host, 0).len() == 3);
            assert!(whole, "the folder listed whole");
            assert_eq!(listed(&host, 0), ["c.txt", "b.txt", "a.txt"]);
            let owed = host.run_until(PATIENCE, |host| {
                host.app().window.folder_views().timer.due.is_some()
            });
            assert!(owed, "opening a remembered folder owes a write");
        },
    );
}

/// A sort chosen in a remembered folder is kept for that folder in `folder-views.toml`, and
/// the default sort is left alone (`FolderSortChosen`, `set_folder_sort_options`).
#[test]
fn a_sort_chosen_in_a_remembered_folder_is_kept_for_it() {
    let size_down = (SortKey::Size, SortDirection::Descending, Default::default());
    hosted_after(
        "folder_views::tests",
        "a_sort_chosen_in_a_remembered_folder_is_kept_for_it",
        make_sorted_folder,
        |mut host| {
            host.app().open(Location::local(sorted_folder()));
            let whole = host.run_until(PATIENCE, |host| listed(host, 0).len() == 3);
            assert!(whole, "the folder listed whole");

            let browser = host.app().browser.clone();
            browser.set_sort(0, SortKey::Size, SortDirection::Descending);

            let kept = host.run_until(PATIENCE, |_| {
                folder_sort_saved(&sorted_folder()) == Some(Some(size_down))
            });
            assert!(
                kept,
                "folder-views.toml holds {:?}",
                folder_sort_saved(&sorted_folder())
            );
            let default = crate::shell::preferences(host.app());
            assert_eq!(
                strata_desktop::settings::sort_preferences(default).sort_key,
                SortKey::Name,
                "the default sort is left alone"
            );
        },
    );
}

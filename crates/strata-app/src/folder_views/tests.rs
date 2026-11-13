// SPDX-License-Identifier: MIT

use std::time::Duration;

use strata_core::model::Location;
use strata_desktop::settings::{
    folder_views::{FolderViews, key_for_location},
    folder_views_path,
};

use super::{Tick, Timer};
use crate::test_support::hosted;

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

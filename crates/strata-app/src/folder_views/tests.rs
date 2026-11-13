// SPDX-License-Identifier: MIT

use std::time::Duration;

use super::{Tick, Timer};

const SAVE: Duration = Duration::from_millis(500);
const USE: Duration = Duration::from_secs(60);

/// GTK's one timer (`schedule_folder_views_write`): a pending write keeps its time unless a
/// shorter wait is asked for, a longer wait asked meanwhile changes nothing, and the poll's
/// clock says when it is due.
#[test]
fn a_pending_write_keeps_its_time_unless_a_shorter_wait_is_asked() {
    let mut timer = Timer::default();
    assert_eq!(timer.tick(0.0), Tick::Idle, "nothing asked");

    assert!(timer.ask(USE), "a first write");
    assert_eq!(
        timer.tick(1.0),
        Tick::Wait(USE),
        "timed from the poll that saw it"
    );
    assert!(timer.ask(SAVE), "a shorter wait");
    assert_eq!(
        timer.tick(2.0),
        Tick::Wait(SAVE),
        "the shorter wait replaces it"
    );
    assert!(!timer.ask(USE), "a longer wait keeps the time");
    assert!(!timer.ask(SAVE), "the same wait keeps the time");
    assert_eq!(timer.tick(2.25), Tick::Wait(Duration::from_millis(250)));
    assert_eq!(timer.tick(2.5), Tick::Due, "due");
    assert_eq!(timer.tick(3.0), Tick::Idle, "a write starts once");

    assert!(timer.ask(SAVE));
    timer.clear();
    assert_eq!(
        timer.tick(4.0),
        Tick::Idle,
        "a write started by hand clears the timer"
    );
}

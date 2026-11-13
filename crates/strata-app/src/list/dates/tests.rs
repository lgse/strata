// SPDX-License-Identifier: MIT

use std::time::Duration;

use super::{Dates, Tick};

fn wake(seconds: f64) -> Option<Duration> {
    Some(Duration::from_secs_f64(seconds))
}

/// On the poll's clock: while dates are relative and rows show, the dates read again every
/// 30 s, the poll waking for it; once not (another format, or no row), nothing wakes and a
/// new period starts when they are again.
#[test]
fn relative_dates_read_again_every_thirty_seconds_and_nothing_wakes_otherwise() {
    let mut dates = Dates::default();
    // (clock, live, moved, wake)
    let polls = [
        (0.0, true, false, wake(30.0)),
        (12.0, true, false, wake(18.0)),
        (30.0, true, true, wake(30.0)),
        (45.0, true, false, wake(15.0)),
        (60.0, true, true, wake(30.0)),
        (61.0, false, false, None),
        (100.0, false, false, None),
        (101.0, true, false, wake(30.0)),
        (131.0, true, true, wake(30.0)),
    ];
    let mut epochs = Vec::new();
    for (clock, live, moved, wake) in polls {
        assert_eq!(
            dates.tick(live, clock),
            Tick { moved, wake },
            "at {clock} s"
        );
        epochs.push(dates.epoch());
    }
    assert_eq!(epochs, [0, 0, 1, 1, 2, 2, 2, 2, 3]);
}

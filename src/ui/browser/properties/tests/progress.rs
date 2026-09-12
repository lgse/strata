// SPDX-License-Identifier: MIT

use super::super::*;

#[test]
fn progress_throttle_reports_the_first_update_and_limits_subsequent_bursts() {
    let throttle = SizeProgressThrottle::default();
    let started = Instant::now();
    assert!(throttle.should_update(started));
    for milliseconds in 1..150 {
        assert!(!throttle.should_update(started + Duration::from_millis(milliseconds)));
    }
    assert!(throttle.should_update(started + SIZE_PROGRESS_INTERVAL));
    assert!(!throttle.should_update(started + SIZE_PROGRESS_INTERVAL));
    assert!(!throttle.should_update(started + Duration::from_millis(299)));
    assert!(throttle.should_update(started + Duration::from_millis(300)));
    assert!(throttle.should_update(started + Duration::from_secs(2)));
    assert!(SizeProgressThrottle::default().should_update(started));
}

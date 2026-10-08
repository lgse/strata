// SPDX-License-Identifier: MIT

use super::{Hyprland, Shake, ShakeListener};
use std::time::Duration;

#[test]
fn horizontal_shake_opens_once_but_normal_motion_does_not() {
    let mut shake = Shake::default();
    for (index, x) in [0, 60, 15, 80].into_iter().enumerate() {
        assert!(!shake.sample_drag(x, 100, Duration::from_millis(index as u64 * 80), true));
    }
    assert!(shake.sample_drag(20, 100, Duration::from_millis(320), true));
    assert!(!shake.sample_drag(80, 100, Duration::from_millis(400), true));
    assert!(!shake.sample_drag(30, 100, Duration::from_millis(480), true));
}

#[test]
fn dropping_an_idle_listener_stops_its_worker() {
    let dir = tempfile::tempdir().expect("shake worker fixture");
    let hyprland = Hyprland {
        socket: dir.path().join("missing.sock"),
    };
    let ShakeListener { positions, _stop } = hyprland.listen();
    drop(_stop);
    assert!(matches!(
        positions.recv_timeout(Duration::from_secs(2)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn idle_shaking_never_opens_a_shelf() {
    let mut shake = Shake::default();
    for (index, x) in [0, 60, 15, 80, 20, 90, 0].into_iter().enumerate() {
        assert!(!shake.sample_drag(x, 100, Duration::from_millis(index as u64 * 80), false));
    }
}

#[test]
fn releasing_a_drag_discards_an_incomplete_shake() {
    let mut shake = Shake::default();
    for (index, x) in [0, 60, 15, 80].into_iter().enumerate() {
        assert!(!shake.sample_drag(x, 100, Duration::from_millis(index as u64 * 80), true));
    }
    assert!(!shake.sample_drag(20, 100, Duration::from_millis(320), false));
    assert!(!shake.sample_drag(80, 100, Duration::from_millis(400), true));
}

#[test]
fn vertical_and_slow_changes_do_not_trigger() {
    let mut shake = Shake::default();
    for (index, x) in [0, 50, 10, 60, 0, 80].into_iter().enumerate() {
        assert!(!shake.sample_drag(
            x,
            index as i32 * 100,
            Duration::from_millis(index as u64 * 220),
            true,
        ));
    }
}

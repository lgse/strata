// SPDX-License-Identifier: MIT

//! Which of Rune's ways to wake its loop still run while a Wayland window is covered
//! (upstream #139), measured with `examples/wake-probe.rs`. The `glib_loop` shim reaches the
//! loop through `AppHandle::update_if` because of what this finds.
//!
//! At Rune `2a44156`, with something due every 16 ms and a window that owes a frame it cannot
//! draw under the cover: a `Waker` call never runs `poll`, a `wake_after` timer runs every
//! 250 ms, and `update_if` runs on time. Shown, all three run on time. Only what the shim
//! relies on is asserted; the test prints the rest, to read again when the Rune rev moves.

use std::time::Duration;

use crate::support::{Display, Proc, Session, example};

const PERIOD_MS: u32 = 16;
const SHOWN: Duration = Duration::from_secs(2);
const COVERED: Duration = Duration::from_secs(3);

/// How late each run in `during` was, in milliseconds.
fn lateness(app: &Proc, during: Duration) -> Vec<f64> {
    let from = app.line_count();
    std::thread::sleep(during);
    app.lines()[from..]
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|v| v["late_ms"].as_f64())
        .collect()
}

fn summary(late: &[f64]) -> String {
    let mut sorted = late.to_vec();
    sorted.sort_by(f64::total_cmp);
    match sorted.as_slice() {
        [] => "no runs".to_owned(),
        [.., max] => format!(
            "{} runs, median {:.1} ms, max {:.1} ms",
            sorted.len(),
            sorted[sorted.len() / 2],
            max
        ),
    }
}

/// Runs the probe in `mode` shown, then under a full-screen cover.
fn shown_then_covered(mode: &str) -> (Vec<f64>, Vec<f64>) {
    let s = Session::start();
    let mut command = s.command(example("wake-probe"), Display::Wayland);
    command.arg(mode).arg(PERIOD_MS.to_string());
    let app = s.spawn("wake-probe", command);
    s.wait_window(app.pid());
    app.wait_json("a first run", |v| v["ev"] == "ran");
    let shown = lateness(&app, SHOWN);
    let cover = s.peer(Display::Wayland);
    cover.map("cover");
    let covering = s.wait_window(cover.pid());
    s.sway_command(&format!("[con_id={}] fullscreen enable", covering.id));
    s.wait_window_where("the cover in front", |w| w.id == covering.id && w.focused);
    // Let the frame in flight when the cover arrived finish.
    std::thread::sleep(Duration::from_millis(300));
    let covered = lateness(&app, COVERED);
    eprintln!(
        "{mode} every {PERIOD_MS} ms: shown {}; covered {}",
        summary(&shown),
        summary(&covered)
    );
    (shown, covered)
}

/// Half of the runs a period allows, so a loaded machine passes and a stalled loop does not.
fn runs_expected(during: Duration) -> usize {
    (during.as_millis() / u128::from(PERIOD_MS) / 2) as usize
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn an_update_reaches_runes_loop_while_the_window_is_covered() {
    for mode in ["waker", "timer", "update"] {
        let (shown, covered) = shown_then_covered(mode);

        assert!(
            shown.len() >= runs_expected(SHOWN),
            "{mode}, shown: {}",
            summary(&shown)
        );
        if mode == "update" {
            assert!(
                covered.len() >= runs_expected(COVERED),
                "{mode}, covered: {}",
                summary(&covered)
            );
        }
    }
}

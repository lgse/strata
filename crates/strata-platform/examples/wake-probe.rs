// SPDX-License-Identifier: MIT

//! Which of Rune's ways to wake its loop still run while a window is covered, and how late:
//! `wake-probe waker|timer|update [PERIOD_MS]`. Something is due every period, 100 ms unless given:
//!
//! - `waker`: a thread calls the `Waker` that `AppSpec::setup` hands out, and `poll` logs how
//!   long after the call it ran.
//! - `timer`: `poll` asks for `Tx::wake_after`, and logs how long after the deadline it ran.
//! - `update`: a thread calls `AppHandle::update_if`, and the closure logs how long after the
//!   call it ran.
//!
//! Each run is one JSON line on stdout: `{"ev":"ran","late_ms":0.4}`. Upstream Rune #139 is a
//! covered Wayland window whose `poll` and timers stop; `tests/session/wake.rs` reads these
//! lines with the window shown and then covered.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rune::{App, AppSpec, Cx, El, Tx, Waker, col, text};

/// The second argument, in milliseconds; 100 without one.
fn period() -> Duration {
    let millis = std::env::args().nth(2).and_then(|text| text.parse().ok());
    Duration::from_millis(millis.unwrap_or(100))
}

const EARLY: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Waker,
    Timer,
    Update,
}

struct Probe {
    mode: Mode,
    /// When the thread last called the waker, until `poll` answers it.
    called: Arc<Mutex<Option<Instant>>>,
    /// When `poll`'s timer is due.
    due: Option<Instant>,
    /// Whether the window has keyboard focus. A cover takes it, so the window owes a frame it
    /// cannot draw while covered, as any app that shows its focus does.
    focused: bool,
}

fn ran(since: Instant) {
    let late_ms = since.elapsed().as_secs_f64() * 1000.0;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{{\"ev\":\"ran\",\"late_ms\":{late_ms:.3}}}");
    let _ = out.flush();
}

fn mode() -> Mode {
    match std::env::args().nth(1).as_deref() {
        Some("waker") => Mode::Waker,
        Some("timer") => Mode::Timer,
        Some("update") => Mode::Update,
        _ => {
            eprintln!("usage: wake-probe waker|timer|update [PERIOD_MS]");
            std::process::exit(2);
        }
    }
}

fn init() -> Probe {
    Probe {
        mode: mode(),
        called: Arc::default(),
        due: None,
        focused: true,
    }
}

fn setup(probe: &mut Probe, waker: Waker) {
    if probe.mode != Mode::Waker {
        return;
    }
    let called = probe.called.clone();
    let period = period();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(period);
            *called.lock().expect("called") = Some(Instant::now());
            waker();
        }
    });
}

/// Logs how long after the thread's call this `poll` ran.
fn answer_waker(probe: &Probe) {
    if let Some(called) = probe.called.lock().expect("called").take() {
        ran(called);
    }
}

/// Logs how late the timer is, and asks for the next one.
fn answer_timer(probe: &mut Probe, tx: &mut Tx) {
    let now = Instant::now();
    // Rune counts a deadline from its own clock, read before this function's, so a wake can
    // come a moment early. Anything else that runs `poll` comes here too, and must not end
    // the chain.
    if let Some(due) = probe.due.filter(|due| now + EARLY < *due) {
        tx.wake_after(due - now);
        return;
    }
    if let Some(due) = probe.due {
        ran(due);
    }
    probe.due = Some(now + period());
    tx.wake_after(period());
}

fn poll(probe: &mut Probe, _: f64, tx: &mut Tx) -> bool {
    match probe.mode {
        Mode::Waker => answer_waker(probe),
        Mode::Timer => answer_timer(probe, tx),
        Mode::Update => {}
    }
    false
}

fn view(probe: &Probe, _: &mut Cx<Probe>) -> El {
    let focus = if probe.focused {
        "focused"
    } else {
        "unfocused"
    };
    col()
        .w_full()
        .h_full()
        .items_center()
        .justify_center()
        .child(text(format!("wake-probe, {focus}")))
        .on_window_focus(|probe: &mut Probe, focused, _: &mut Tx| probe.focused = focused)
}

fn main() {
    let updating = mode() == Mode::Update;
    let app = App::new(AppSpec {
        setup: Some(setup),
        poll: Some(poll),
        ..AppSpec::new("wake-probe", (480.0, 320.0), init, view)
    })
    .args(Vec::new());
    if updating {
        let handle = app.handle();
        let period = period();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(period);
                let called = Instant::now();
                handle.update_if(move |_, _| {
                    ran(called);
                    false
                });
            }
        });
    }
    app.run();
}

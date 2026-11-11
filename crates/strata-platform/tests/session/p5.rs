//! P5's session tests: the `settings` example against a fake Settings portal on the session's
//! private bus (`p5/fake_portal.rs`), served from this test process. Most need only the bus;
//! the last two open the example's window under headless sway, to check that changes reach
//! Rune's loop, also while another window covers it (upstream #139: a covered Wayland
//! window's `poll` and timers stop).
//!
//! The fake connects with `Session::bus().address()` only. The no-bus test points
//! `DBUS_SESSION_BUS_ADDRESS` at a socket that doesn't exist inside the session's own runtime
//! directory, rather than unsetting it, so nothing can fall back to another bus.
//!
//! Feature map: `settings/themes/portal-text-scaling` ("changing `text-scaling-factor` while
//! the window is open rescales it without a restart", as far as the change reaching the app;
//! "only `text-scaling-factor` in `org.gnome.desktop.interface` with a floating-point value is
//! applied"; "a factor of 0, below 0, NaN, or infinite leaves it unchanged"; "without a session
//! bus, or when `ReadAll` fails, the value stays and a log records it"), and the double-click
//! interval and animations switch that `browser/selection/click-modes` and the motion bullets
//! read, as data.
//!
//! Work package: P5, `docs/work/phase-1-leaves/P5-desktop-settings.md`.

mod fake_portal;

use std::time::{Duration, Instant};

use serde_json::Value;
use zbus::Variant;

use crate::support::{Display, Options, Proc, Session, example, timeout};
use fake_portal::FakePortal;

const INTERFACE: &str = "org.gnome.desktop.interface";
const MOUSE: &str = "org.gnome.desktop.peripherals.mouse";
const PRIVACY: &str = "org.gnome.desktop.privacy";
const APPEARANCE: &str = "org.freedesktop.appearance";

const ALL_KEYS: [&str; 9] = [
    "text-scaling-factor",
    "double-click",
    "drag-threshold",
    "cursor-blink",
    "cursor-blink-time",
    "cursor-blink-timeout",
    "enable-animations",
    "reduced-motion",
    "remember-recent-files",
];

/// Every key, each away from its default, plus a key the module doesn't read.
fn desktop() -> Vec<(&'static str, &'static str, Variant<'static>)> {
    vec![
        (INTERFACE, "text-scaling-factor", Variant::F64(1.25)),
        (MOUSE, "double-click", Variant::I32(1200)),
        (MOUSE, "drag-threshold", Variant::I32(12)),
        (INTERFACE, "cursor-blink", Variant::Bool(false)),
        (INTERFACE, "cursor-blink-time", Variant::I32(900)),
        (INTERFACE, "cursor-blink-timeout", Variant::I32(4)),
        (INTERFACE, "enable-animations", Variant::Bool(false)),
        (APPEARANCE, "reduced-motion", Variant::U32(1)),
        (PRIVACY, "remember-recent-files", Variant::Bool(false)),
        (INTERFACE, "font-name", Variant::from("Sans 11")),
    ]
}

fn bus_only() -> Session {
    Session::with(Options {
        compositor: false,
        ..Options::default()
    })
}

/// The example without a window, logging its events.
fn watch(s: &Session) -> Proc {
    let mut cmd = s.command(example("settings"), Display::None);
    cmd.arg("--no-window");
    let app = s.spawn("settings", cmd);
    app.wait_json("the watcher started", |v| v["ev"] == "started");
    app
}

fn snapshot(app: &Proc) -> Value {
    app.wait_json("the snapshot", |v| v["ev"] == "snapshot")["settings"].clone()
}

fn known(settings: &Value) -> Vec<String> {
    settings["known"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|k| k.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn assert_defaults(settings: &Value) {
    assert_eq!(settings["text_scale"], 1.0, "{settings}");
    assert_eq!(settings["double_click_ms"], 400, "{settings}");
    assert_eq!(settings["drag_threshold"], 8, "{settings}");
    assert_eq!(settings["cursor_blink"], true, "{settings}");
    assert_eq!(settings["cursor_blink_time_ms"], 1200, "{settings}");
    assert_eq!(settings["cursor_blink_timeout_s"], 10, "{settings}");
    assert_eq!(settings["enable_animations"], true, "{settings}");
    assert_eq!(settings["reduced_motion"], false, "{settings}");
    assert_eq!(settings["remember_recent_files"], true, "{settings}");
    assert!(known(settings).is_empty(), "{settings}");
}

fn assert_desktop(settings: &Value) {
    assert_eq!(settings["text_scale"], 1.25, "{settings}");
    assert_eq!(settings["double_click_ms"], 1200, "{settings}");
    assert_eq!(settings["drag_threshold"], 12, "{settings}");
    assert_eq!(settings["cursor_blink"], false, "{settings}");
    assert_eq!(settings["cursor_blink_time_ms"], 900, "{settings}");
    assert_eq!(settings["cursor_blink_timeout_s"], 4, "{settings}");
    assert_eq!(settings["enable_animations"], false, "{settings}");
    assert_eq!(settings["reduced_motion"], true, "{settings}");
    assert_eq!(settings["remember_recent_files"], false, "{settings}");
    assert_eq!(known(settings), ALL_KEYS, "{settings}");
}

/// The JSON lines from `from` on whose `ev` is `ev`.
fn events(app: &Proc, from: usize, ev: &str) -> Vec<Value> {
    app.lines()
        .iter()
        .skip(from)
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["ev"] == ev)
        .collect()
}

/// Waits for the next `changed` line for `key`, from line `from` on; returns the line after it.
fn wait_changed(app: &Proc, from: usize, key: &str) -> (usize, Value) {
    let (at, v) = app.wait_json_from(from, &format!("a change to {key}"), |v| {
        v["ev"] == "changed" && v["key"] == key
    });
    (at + 1, v["settings"].clone())
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn reads_the_snapshot() {
    let s = bus_only();
    let _portal = FakePortal::start(s.bus().address(), &desktop(), true);
    let app = watch(&s);

    assert_desktop(&snapshot(&app));
    assert!(events(&app, 0, "fallback").is_empty(), "{:?}", app.lines());
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn follows_every_change() {
    let s = bus_only();
    let portal = FakePortal::start(s.bus().address(), &desktop(), true);
    let app = watch(&s);
    snapshot(&app);
    let mut at = app.line_count();

    let changes: [(&str, &str, Variant<'static>, &str, Value); 9] = [
        (
            INTERFACE,
            "text-scaling-factor",
            Variant::F64(1.5),
            "text_scale",
            1.5.into(),
        ),
        (
            MOUSE,
            "double-click",
            Variant::I32(250),
            "double_click_ms",
            250.into(),
        ),
        (
            MOUSE,
            "drag-threshold",
            Variant::I32(3),
            "drag_threshold",
            3.into(),
        ),
        (
            INTERFACE,
            "cursor-blink",
            Variant::Bool(true),
            "cursor_blink",
            true.into(),
        ),
        (
            INTERFACE,
            "cursor-blink-time",
            Variant::I32(600),
            "cursor_blink_time_ms",
            600.into(),
        ),
        (
            INTERFACE,
            "cursor-blink-timeout",
            Variant::I32(20),
            "cursor_blink_timeout_s",
            20.into(),
        ),
        (
            INTERFACE,
            "enable-animations",
            Variant::Bool(true),
            "enable_animations",
            true.into(),
        ),
        (
            APPEARANCE,
            "reduced-motion",
            Variant::U32(0),
            "reduced_motion",
            false.into(),
        ),
        (
            PRIVACY,
            "remember-recent-files",
            Variant::Bool(true),
            "remember_recent_files",
            true.into(),
        ),
    ];
    for (namespace, key, value, field, expected) in changes {
        portal.set(namespace, key, value);
        let (next, settings) = wait_changed(&app, at, key);
        assert_eq!(settings[field], expected, "{key}: {settings}");
        at = next;
    }
    assert_eq!(
        events(&app, at - 1, "changed")
            .last()
            .map(|v| v["settings"]["animations"].clone()),
        Some(true.into())
    );

    // Noise: another namespace, a key the module doesn't read, the value it already has, and
    // unusable values (twice: one fallback line), then a real change that ends it.
    let noise_from = app.line_count();
    portal.emit(
        "org.example.other",
        "text-scaling-factor",
        Variant::F64(3.0),
    );
    portal.emit(INTERFACE, "font-name", Variant::from("Serif 12"));
    portal.set(INTERFACE, "text-scaling-factor", Variant::F64(1.5));
    portal.emit(INTERFACE, "text-scaling-factor", Variant::I32(2));
    portal.emit(INTERFACE, "text-scaling-factor", Variant::F64(0.0));
    portal.emit(INTERFACE, "text-scaling-factor", Variant::F64(f64::NAN));
    portal.emit(
        INTERFACE,
        "text-scaling-factor",
        Variant::F64(f64::INFINITY),
    );
    portal.emit(INTERFACE, "text-scaling-factor", Variant::F64(-1.0));
    portal.set(MOUSE, "double-click", Variant::I32(700));
    let (_, settings) = wait_changed(&app, noise_from, "double-click");

    let changed = events(&app, noise_from, "changed");
    assert_eq!(changed.len(), 1, "only the last change: {changed:?}");
    assert_eq!(settings["text_scale"], 1.5, "{settings}");
    let fallbacks = events(&app, noise_from, "fallback");
    assert_eq!(
        fallbacks.len(),
        1,
        "one report for the bad values: {fallbacks:?}"
    );
    assert_eq!(fallbacks[0]["kind"], "invalid");
    assert_eq!(fallbacks[0]["key"], "text-scaling-factor");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn no_portal_gives_the_defaults() {
    let s = bus_only();
    let app = watch(&s);

    assert_defaults(&snapshot(&app));
    let (_, fallback) = app.wait_json_from(0, "the no-portal fallback", |v| v["ev"] == "fallback");
    assert_eq!(fallback["kind"], "no-portal", "{fallback}");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_portal_that_cannot_start_gives_the_defaults_at_once() {
    let s = bus_only();
    s.bus()
        .add_service("org.freedesktop.portal.Desktop", "/bin/false");
    let started = Instant::now();
    let app = watch(&s);

    assert_defaults(&snapshot(&app));
    let (_, fallback) = app.wait_json_from(0, "the no-portal fallback", |v| v["ev"] == "fallback");
    assert_eq!(fallback["kind"], "no-portal", "{fallback}");
    // The activation fails fast; the watcher's 10 s bound isn't what ends the wait.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn no_session_bus_gives_the_defaults() {
    let s = bus_only();
    // A socket that doesn't exist, in the session's own runtime directory: never unset, so
    // nothing can fall back to another bus.
    let missing = s.runtime_dir().join("no-bus");
    assert!(!missing.exists());
    let mut cmd = s.command(example("settings"), Display::None);
    cmd.arg("--no-window").env(
        "DBUS_SESSION_BUS_ADDRESS",
        format!("unix:path={}", missing.display()),
    );
    let app = s.spawn("settings", cmd);

    assert_defaults(&snapshot(&app));
    let (_, fallback) = app.wait_json_from(0, "the no-bus fallback", |v| v["ev"] == "fallback");
    assert_eq!(fallback["kind"], "no-bus", "{fallback}");
}

/// utime + stime of `pid`, in clock ticks.
fn cpu_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("the example's stat");
    // Fields after the parenthesised name: state is field 3, utime 14 and stime 15.
    let rest = &stat[stat.rfind(')').expect("stat's name") + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    fields[11].parse::<u64>().unwrap_or(0) + fields[12].parse::<u64>().unwrap_or(0)
}

/// The voluntary context switches of `pid`'s thread named `name` (as `/proc` shows it, at most
/// 15 bytes): how often it went to sleep and was woken.
fn thread_wakeups(pid: u32, name: &str) -> u64 {
    let tasks = std::fs::read_dir(format!("/proc/{pid}/task")).expect("the example's threads");
    for task in tasks.flatten() {
        let comm = std::fs::read_to_string(task.path().join("comm")).unwrap_or_default();
        if comm.trim_end() != name {
            continue;
        }
        let status = std::fs::read_to_string(task.path().join("status")).unwrap_or_default();
        return status
            .lines()
            .find_map(|l| l.strip_prefix("voluntary_ctxt_switches:"))
            .and_then(|n| n.trim().parse().ok())
            .expect("voluntary_ctxt_switches");
    }
    panic!("no thread named {name} in {pid}");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn waits_without_polling() {
    let s = bus_only();
    let portal = FakePortal::start(s.bus().address(), &desktop(), true);
    let app = watch(&s);
    snapshot(&app);
    // Past the first read's own wakeups.
    std::thread::sleep(Duration::from_millis(300));
    let ticks = cpu_ticks(app.pid());
    let wakeups = thread_wakeups(app.pid(), strata_platform::settings::THREAD_NAME);

    std::thread::sleep(timeout() / 5);

    let reads: Vec<_> = portal.calls();
    assert_eq!(reads.len(), 1, "one ReadAll, then nothing: {reads:?}");
    assert_eq!(reads[0].method, "ReadAll");
    assert!(reads[0].sender.starts_with(':'), "{reads:?}");
    let spent = cpu_ticks(app.pid()) - ticks;
    // A busy loop would spend about every tick of the wait (100 a second).
    assert!(spent < 10, "{spent} ticks idle");
    // A timer or a poll would wake the watcher's thread; nothing on the bus does.
    let woken = thread_wakeups(app.pid(), strata_platform::settings::THREAD_NAME) - wakeups;
    assert_eq!(woken, 0, "the watcher woke {woken} times while idle");
}

/// The example without a window, started while `portal` delays its answers, once the read is
/// in flight.
fn watch_slow(s: &Session, portal: &FakePortal, args: &[&str]) -> Proc {
    portal.set_delay(Duration::from_millis(1500));
    let mut cmd = s.command(example("settings"), Display::None);
    cmd.args(["--no-window", "--first-answer-ms", "0"])
        .args(args);
    let app = s.spawn("settings", cmd);
    app.wait_json("the watcher started", |v| v["ev"] == "started");
    let deadline = Instant::now() + timeout();
    while portal.calls().is_empty() {
        assert!(
            Instant::now() < deadline,
            "the watcher never read the portal"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    app
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_burst_of_changes_during_a_slow_read_still_lands() {
    let s = bus_only();
    let portal = FakePortal::start(s.bus().address(), &desktop(), true);
    let app = watch_slow(&s, &portal, &[]);
    let started = Instant::now();

    // More than zbus queues per match rule (64) while the reply is outstanding, then one real
    // change, stored before it is announced, so the reply holds it.
    for i in 0..100 {
        portal.emit(INTERFACE, "font-name", Variant::from(format!("Sans {i}")));
    }
    portal.set(INTERFACE, "text-scaling-factor", Variant::F64(1.75));

    let settings = snapshot(&app);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the reply waited for the read's timeout: {:?}",
        started.elapsed()
    );
    assert_eq!(settings["text_scale"], 1.75, "{settings}");
    assert_eq!(settings["double_click_ms"], 1200, "{settings}");
    assert_eq!(known(&settings), ALL_KEYS, "{settings}");
    // The change, received before the reply that holds it, doesn't replay after it.
    settle(&app, Duration::from_millis(300));
    assert!(events(&app, 0, "changed").is_empty(), "{:?}", app.lines());
    assert!(events(&app, 0, "fallback").is_empty(), "{:?}", app.lines());
}

/// A portal that leaves without answering: the changes it sent during the read still apply.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn changes_from_a_portal_that_left_mid_read_still_apply() {
    let s = bus_only();
    let portal = FakePortal::start(s.bus().address(), &desktop(), true);
    portal.hold();
    let app = watch_with(&s, &["--first-answer-ms", "0"]);
    wait_reads(&portal, 1);
    portal.set(MOUSE, "drag-threshold", Variant::I32(21));
    // Past the bus delivering the change, before the portal goes.
    std::thread::sleep(Duration::from_millis(100));

    drop(portal);

    let snap = snapshot(&app);
    assert_defaults(&snap);
    let (_, settings) = wait_changed(&app, 0, "drag-threshold");
    assert_eq!(settings["drag_threshold"], 21, "{settings}");
    let fallback = app.wait_json("why", |v| v["ev"] == "fallback");
    assert_eq!(fallback["kind"], "no-portal", "{fallback}");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_portal_slower_than_the_timeout_still_lands() {
    let s = bus_only();
    let portal = FakePortal::start(s.bus().address(), &desktop(), true);
    let app = watch_slow(&s, &portal, &["--timeout-ms", "300"]);

    // The timeout gives the defaults and says why...
    assert_defaults(&snapshot(&app));
    let (_, fallback) = app.wait_json_from(0, "the timeout", |v| v["ev"] == "fallback");
    assert_eq!(fallback["kind"], "no-portal", "{fallback}");
    // ...and the reply, when it lands, arrives as changes.
    let (_, settings) = wait_changed(&app, 0, "remember-recent-files");
    assert_desktop(&settings);
    assert_eq!(events(&app, 0, "changed").len(), ALL_KEYS.len());
    assert_eq!(
        portal.calls().len(),
        1,
        "waited for the one read: {:?}",
        portal.calls()
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn follows_a_portal_that_starts_late_and_restarts() {
    let s = bus_only();
    let app = watch(&s);
    assert_defaults(&snapshot(&app));
    app.wait_json("the no-portal fallback", |v| {
        v["ev"] == "fallback" && v["kind"] == "no-portal"
    });

    // A portal takes the name: it is read, and every key arrives as a change.
    let late = app.line_count();
    let first = FakePortal::start(s.bus().address(), &desktop(), true);
    // The last key in the module's order.
    let (at, settings) = wait_changed(&app, late, "remember-recent-files");
    assert_desktop(&settings);
    assert_eq!(events(&app, late, "changed").len(), ALL_KEYS.len());

    // It exits: the values stay.
    drop(first);
    let (lost, _) = app.wait_json_from(at, "the portal-lost fallback", |v| {
        v["ev"] == "fallback" && v["kind"] == "portal-lost"
    });

    // Another portal process, under a new unique name, with one value changed: that one
    // change arrives.
    let mut values = desktop();
    values[0] = (INTERFACE, "text-scaling-factor", Variant::F64(2.0));
    let second = FakePortal::start(s.bus().address(), &values, true);
    let (end, settings) = wait_changed(&app, lost, "text-scaling-factor");
    assert_eq!(settings["text_scale"], 2.0, "{settings}");
    assert_eq!(settings["double_click_ms"], 1200, "{settings}");
    second.set(MOUSE, "double-click", Variant::I32(300));
    let (_, last) = wait_changed(&app, end, "double-click");
    assert_eq!(last["double_click_ms"], 300);
    assert_eq!(events(&app, at, "changed").len(), 2, "{:?}", app.lines());
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_restarted_portal_without_a_key_resets_it() {
    let s = bus_only();
    let first = FakePortal::start(s.bus().address(), &desktop(), true);
    let app = watch(&s);
    assert_desktop(&snapshot(&app));
    let at = app.line_count();

    drop(first);
    let mut values = desktop();
    values.retain(|(_, key, _)| *key != "drag-threshold");
    let _second = FakePortal::start(s.bus().address(), &values, true);

    let (_, settings) = wait_changed(&app, at, "drag-threshold");
    assert_eq!(
        settings["drag_threshold"], 8,
        "back to the default: {settings}"
    );
    assert!(
        !known(&settings).iter().any(|k| k == "drag-threshold"),
        "{settings}"
    );
    assert_eq!(
        settings["double_click_ms"], 1200,
        "the rest stay: {settings}"
    );
    app.wait_json_from(at, "the unset report", |v| {
        v["ev"] == "fallback" && v["kind"] == "unset" && v["key"] == "drag-threshold"
    });
    assert_eq!(events(&app, at, "changed").len(), 1, "{:?}", app.lines());
}

/// The example without a window, with `args`.
fn watch_with(s: &Session, args: &[&str]) -> Proc {
    let mut cmd = s.command(example("settings"), Display::None);
    cmd.arg("--no-window").args(args);
    let app = s.spawn("settings", cmd);
    app.wait_json("the watcher started", |v| v["ev"] == "started");
    app
}

/// Waits until `portal` has been asked `ReadAll` `n` times.
fn wait_reads(portal: &FakePortal, n: usize) {
    let deadline = Instant::now() + timeout();
    while portal
        .calls()
        .iter()
        .filter(|c| c.method == "ReadAll")
        .count()
        < n
    {
        assert!(Instant::now() < deadline, "{n} reads: {:?}", portal.calls());
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn reads(portal: &FakePortal) -> usize {
    portal
        .calls()
        .iter()
        .filter(|c| c.method == "ReadAll")
        .count()
}

/// The settings the last snapshot or change reported.
fn last_settings(app: &Proc) -> Value {
    app.lines()
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|v| v["ev"] == "snapshot" || v["ev"] == "changed")
        .map(|v| v["settings"].clone())
        .unwrap_or(Value::Null)
}

/// A read that times out, then answers with drag-threshold 12 and changes it to 99 right
/// after: the answer must not overwrite the change, whichever the watcher's loop sees first.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_late_answer_never_overwrites_a_later_change() {
    let s = bus_only();
    let runs = 30;
    let mut stale = Vec::new();
    for run in 0..runs {
        let portal = FakePortal::start(s.bus().address(), &desktop(), true);
        portal.hold();
        portal.change_after_answer(MOUSE, "drag-threshold", Variant::I32(99));
        let app = watch_with(&s, &["--first-answer-ms", "0", "--timeout-ms", "150"]);
        app.wait_json("the timeout", |v| {
            v["ev"] == "fallback" && v["kind"] == "no-portal"
        });

        portal.answer();

        wait_changed(&app, 0, "remember-recent-files");
        app.wait_json("the later change", |v| {
            v["settings"]["drag_threshold"] == 99
        });
        settle(&app, Duration::from_millis(150));
        if last_settings(&app)["drag_threshold"] != 99 {
            stale.push(run);
        }
    }
    assert!(
        stale.is_empty(),
        "runs ending on the answer's older value: {stale:?} of {runs}"
    );
}

/// A portal that answers its late read and leaves at once is reported lost, whichever the
/// watcher's loop sees first.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_portal_that_leaves_right_after_a_late_answer_is_lost() {
    let s = bus_only();
    let runs = 20;
    let mut unreported = Vec::new();
    for run in 0..runs {
        let portal = FakePortal::start(s.bus().address(), &desktop(), true);
        portal.hold();
        portal.leave_after_answer();
        let app = watch_with(&s, &["--first-answer-ms", "0", "--timeout-ms", "150"]);
        app.wait_json("the timeout", |v| {
            v["ev"] == "fallback" && v["kind"] == "no-portal"
        });

        portal.answer();

        let (at, _) = wait_changed(&app, 0, "remember-recent-files");
        settle(&app, Duration::from_millis(150));
        if events(&app, at, "fallback")
            .iter()
            .all(|v| v["kind"] != "portal-lost")
        {
            unreported.push(run);
        }
    }
    assert!(
        unreported.is_empty(),
        "runs with no portal-lost: {unreported:?} of {runs}"
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn changes_during_a_slow_read_never_go_backwards() {
    let s = bus_only();
    let portal = FakePortal::start(s.bus().address(), &desktop(), true);
    let app = watch_slow(&s, &portal, &[]);

    // Each is stored before it is announced; then one more is stored unannounced, so the
    // answer holds a value no held change has.
    for i in 0..500 {
        portal.set(MOUSE, "drag-threshold", Variant::I32(100 + i));
    }
    portal.set_quietly(MOUSE, "drag-threshold", Variant::I32(600));

    assert_eq!(snapshot(&app)["drag_threshold"], 600);
    settle(&app, Duration::from_millis(500));
    let changed = events(&app, 0, "changed");
    assert!(
        changed.is_empty(),
        "replayed after the answer: {} changes, first {:?}",
        changed.len(),
        changed.first()
    );
    assert_eq!(last_settings(&app)["drag_threshold"], 600);
}

/// The portal's values, with the scale at 2.0 and the threshold at 30.
fn replacement() -> Vec<(&'static str, &'static str, Variant<'static>)> {
    let mut values = desktop();
    values[0] = (INTERFACE, "text-scaling-factor", Variant::F64(2.0));
    values[2] = (MOUSE, "drag-threshold", Variant::I32(30));
    values
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_portal_replaced_during_a_read_is_read_again_after_it() {
    let s = bus_only();
    let old = FakePortal::start(s.bus().address(), &desktop(), false);
    old.take_over();
    old.hold();
    let app = watch_with(&s, &["--first-answer-ms", "0"]);
    wait_reads(&old, 1);

    // A new portal takes the name while the first read is out; the old one stays on the bus
    // and answers that read afterwards.
    let new = FakePortal::start(s.bus().address(), &replacement(), false);
    new.take_over();
    old.answer();

    assert_desktop(&snapshot(&app));
    app.wait_json("the new portal's values", |v| {
        v["ev"] == "changed" && v["settings"]["text_scale"] == 2.0
    });
    settle(&app, Duration::from_millis(300));
    let last = last_settings(&app);
    assert_eq!(last["text_scale"], 2.0, "{last}");
    assert_eq!(last["drag_threshold"], 30, "{last}");
    assert_eq!(reads(&new), 1, "{:?}", new.calls());
    assert!(events(&app, 0, "fallback").is_empty(), "{:?}", app.lines());
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_new_portal_replaces_a_read_that_never_answered() {
    let s = bus_only();
    let silent = FakePortal::start(s.bus().address(), &desktop(), false);
    silent.take_over();
    silent.hold();
    let app = watch_with(&s, &["--first-answer-ms", "0", "--timeout-ms", "300"]);
    wait_reads(&silent, 1);
    // One portal takes the name before the timeout, the read still out...
    let passing = FakePortal::start(s.bus().address(), &desktop(), false);
    passing.take_over();
    passing.hold();
    app.wait_json("the timeout", |v| {
        v["ev"] == "fallback" && v["kind"] == "no-portal"
    });

    // ...and another after it: the read that never answered gives way to the newest portal.
    let newest = FakePortal::start(s.bus().address(), &replacement(), false);
    newest.take_over();

    let (_, settings) = wait_changed(&app, 0, "remember-recent-files");
    assert_eq!(settings["text_scale"], 2.0, "{settings}");
    assert_eq!(settings["drag_threshold"], 30, "{settings}");
    settle(&app, Duration::from_millis(300));
    assert_eq!(reads(&newest), 1, "read once: {:?}", newest.calls());
    assert_eq!(reads(&passing), 0, "{:?}", passing.calls());
}

/// The example's window under sway, fed by `settings::follow`; returns once its first frame is
/// drawn, its view ran with the portal's values, and the snapshot's update ran on Rune's loop.
fn window(s: &Session) -> Proc {
    let app = s.spawn("settings", s.command(example("settings"), Display::Wayland));
    app.wait_json("ready", |v| v["ev"] == "ready");
    app.wait_json("the first frame", |v| v["ev"] == "frame");
    // The first view has them when the portal answered within the first-answer wait;
    // otherwise a later one does, once the snapshot's update lands.
    let view = app.wait_json("a view with the portal's values", |v| {
        v["ev"] == "view"
            && v["settings"]["known"]
                .as_array()
                .is_some_and(|k| !k.is_empty())
    });
    assert_desktop(&view["settings"]);
    app.wait_json("the snapshot's update", |v| v["ev"] == "landed");
    s.wait_window(app.pid());
    app
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn changes_reach_runes_loop() {
    let s = Session::start();
    let portal = FakePortal::start(s.bus().address(), &desktop(), true);
    let app = window(&s);
    let at = app.line_count();

    portal.set(INTERFACE, "text-scaling-factor", Variant::F64(2.0));

    let (landed, _) = app.wait_json_from(at, "the update on Rune's loop", |v| v["ev"] == "landed");
    let (_, view) = app.wait_json_from(landed, "the view with the new value", |v| {
        v["ev"] == "view" && v["settings"]["text_scale"] == 2.0
    });
    assert_eq!(view["settings"]["double_click_ms"], 1200);
}

/// Waits until `app` has logged nothing new for `quiet`.
fn settle(app: &Proc, quiet: Duration) {
    let deadline = Instant::now() + timeout();
    let mut count = app.line_count();
    loop {
        std::thread::sleep(quiet);
        let now = app.line_count();
        if now == count {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the example never settled: {:?}",
            app.lines()
        );
        count = now;
    }
}

/// The window, with the portal's answer held back until the window has drawn and gone quiet:
/// `follow`'s snapshot then lands on a settled window. Returns the line after that update.
fn window_with_late_answer(
    s: &Session,
    values: &[(&'static str, &'static str, Variant<'static>)],
) -> (Proc, usize) {
    let portal = FakePortal::start(s.bus().address(), values, true);
    portal.hold();
    let app = s.spawn("settings", s.command(example("settings"), Display::Wayland));
    app.wait_json("the first frame", |v| v["ev"] == "frame");
    s.wait_window(app.pid());
    settle(&app, Duration::from_millis(500));
    assert!(events(&app, 0, "landed").is_empty(), "{:?}", app.lines());
    let at = app.line_count();

    portal.answer();

    let (landed, _) = app.wait_json_from(at, "the late snapshot's update", |v| v["ev"] == "landed");
    // Long enough for a re-run the update asked for to show.
    std::thread::sleep(Duration::from_millis(700));
    (app, landed + 1)
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_snapshot_with_new_values_runs_the_view() {
    let s = Session::start();
    let (app, after) = window_with_late_answer(&s, &desktop());

    let views = events(&app, after, "view");
    assert!(!views.is_empty(), "{:?}", app.lines());
    assert_desktop(&views[0]["settings"]);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_snapshot_that_changes_nothing_runs_no_view() {
    let s = Session::start();
    // A portal with none of the keys: its snapshot is the defaults the window started with,
    // so `follow`'s update returns false.
    let (app, after) = window_with_late_answer(&s, &[]);

    let views = events(&app, after, "view");
    assert!(views.is_empty(), "the view re-ran: {views:?}");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn changes_reach_a_covered_window() {
    let s = Session::start();
    let portal = FakePortal::start(s.bus().address(), &desktop(), true);
    let app = window(&s);
    let cover = s.peer(Display::Wayland);
    cover.map("cover");
    let covering = s.wait_window(cover.pid());
    s.sway_command(&format!("[con_id={}] fullscreen enable", covering.id));
    let fullscreen = |tree: &Value| -> bool {
        fn find(node: &Value, id: i64) -> Option<&Value> {
            if node["id"] == id {
                return Some(node);
            }
            ["nodes", "floating_nodes"]
                .iter()
                .filter_map(|k| node[*k].as_array())
                .flatten()
                .find_map(|n| find(n, id))
        }
        find(tree, covering.id).is_some_and(|n| n["fullscreen_mode"] == 1)
    };
    assert!(fullscreen(&s.tree()), "the cover is full screen");
    let at = app.line_count();

    portal.set(INTERFACE, "text-scaling-factor", Variant::F64(1.75));

    // The update runs on Rune's loop while the window is covered.
    let (landed, _) = app.wait_json_from(at, "the update while covered", |v| v["ev"] == "landed");
    std::thread::sleep(Duration::from_millis(500));
    let covered_views = events(&app, landed, "view").len();
    // Uncovered, the view shows the value.
    s.sway_command(&format!("[con_id={}] fullscreen disable", covering.id));
    app.wait_json_from(landed, "the view with the new value", |v| {
        v["ev"] == "view" && v["settings"]["text_scale"] == 1.75
    });
    eprintln!("views while covered after the update: {covered_views}");
}

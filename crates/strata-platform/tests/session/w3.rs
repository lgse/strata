//! W3's own checks of the harness: a Rune window under headless sway and Xwayland, virtual
//! input reaching it, the peer's clipboard, drag and drop (with an `XdndProxy` too) and export
//! on both backends, the private bus, and teardown, orderly and after a SIGKILL.
//!
//! Work package: W3, `docs/work/phase-0-groundwork/W3-compositor-harness.md`.

use strata_test_peer::protocol::{Action, Event, Offer, Payload, Selection, Via, fnv1a64, pattern};

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::support::{
    Display, Options, Proc, Session, Shell, Window, alive, example, processes_of, report, timeout,
};

const TEXT: &str = "héllo from the peer ✓";
const UTF8: &str = "text/plain;charset=utf-8";

/// Opens the example window and waits until it has drawn and laid out at the size sway gave
/// it: the app, its window in sway's tree, and the kind of native handle Rune reported.
///
/// The layout wait matters on X11: the window maps at its own size and sway tiles it after,
/// so input sent before Rune lays out again can land outside its old size.
fn harness_window(s: &Session, display: Display, title: &str) -> (Proc, Window, String) {
    let mut cmd = s.command(example("harness-window"), display);
    cmd.args(["--title", title]);
    let app = s.spawn("harness-window", cmd);
    let ready = app.wait_json("ready", |v| v["ev"] == "ready");
    app.wait_json("the first frame", |v| v["ev"] == "frame");
    let window = s.wait_window(app.pid());
    let (w, h) = (window.content.width, window.content.height);
    app.wait_json(&format!("a layout at {w}x{h}"), |v| {
        v["ev"] == "layout"
            && (v["w"].as_f64().unwrap_or(0.0) - w).abs() < 1.0
            && (v["h"].as_f64().unwrap_or(0.0) - h).abs() < 1.0
    });
    let handle = ready["handle"].as_str().unwrap_or_default().to_owned();
    (app, window, handle)
}

fn no_errors(app: &Proc) {
    let errors: Vec<String> = app
        .lines()
        .into_iter()
        .filter(|l| l.contains("exit-error"))
        .collect();
    assert!(errors.is_empty(), "the window reported errors: {errors:?}");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn rune_window_opens_under_sway() {
    let s = Session::start();
    let (app, window, handle) = harness_window(&s, Display::Wayland, "w3 wayland");
    assert_eq!(handle, "wayland");
    assert_eq!(window.shell, Shell::XdgShell);
    assert_eq!(window.app_id.as_deref(), Some("strata-harness-window"));
    assert_eq!(window.title, "w3 wayland");
    assert!(!window.floating, "sway tiles a new window");
    no_errors(&app);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn rune_window_opens_under_xwayland() {
    let s = Session::start();
    let (app, window, handle) = harness_window(&s, Display::X11, "w3 x11");
    assert!(
        handle == "xlib" || handle == "xcb",
        "an X11 window, not {handle:?}"
    );
    assert_eq!(window.shell, Shell::Xwayland);
    assert_eq!(window.app_id.as_deref(), Some("strata-harness-window"));
    assert_eq!(window.title, "w3 x11");
    no_errors(&app);
}

/// Focus, a click and keys from the input peer all reach the window.
fn input_reaches(display: Display) {
    let s = Session::start();
    let (app, window, _) = harness_window(&s, display, "w3 input");
    s.focus(&window);
    app.wait_json("focus", |v| v["ev"] == "focus" && v["focused"] == true);
    let input = s.input();
    // Each step waits for its own line, after the last step's.
    input.click_at(window.at(0.5, 0.5));
    let (at, _) = app.wait_json_from(0, "a press", |v| v["ev"] == "press" && v["n"] == 1);
    input.right_click_at(window.at(0.5, 0.5));
    let (at, _) = app.wait_json_from(at + 1, "a secondary press", |v| {
        v["ev"] == "secondary-press"
    });
    input.key("Return");
    let (at, _) = app.wait_json_from(at + 1, "Return", |v| {
        v["ev"] == "key" && v["key"] == "Enter"
    });
    input.key("ctrl+a");
    let (at, _) = app.wait_json_from(at + 1, "Ctrl+A", |v| v["ev"] == "key" && v["ctrl"] == true);
    input.key_down("shift");
    input.key("b");
    input.key_up("shift");
    app.wait_json_from(at + 1, "B with Shift held", |v| {
        v["ev"] == "key" && v["shift"] == true && v["key"] == "Text(\"B\")"
    });
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn virtual_input_reaches_a_wayland_window() {
    input_reaches(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn virtual_input_reaches_an_x11_window() {
    input_reaches(Display::X11);
}

/// A peer that holds keyboard focus on its own window, as the data device needs.
fn focused_peer(s: &Session, display: Display, title: &str) -> crate::support::Peer {
    let peer = s.peer(display);
    peer.map(title);
    let window = s.wait_window(peer.pid());
    s.focus(&window);
    peer.wait_event("focus", |e| matches!(e, Event::Focus { focused: true }));
    peer
}

fn text_offers() -> [Offer; 2] {
    [Offer::text(UTF8, TEXT), Offer::text("text/plain", TEXT)]
}

fn round_trip(peer: &crate::support::Peer, selection: Selection, via: Via) {
    peer.set_selection(selection, via, &text_offers());
    let mimes = peer.targets(selection, via).expect("an owner");
    assert!(
        mimes.contains(&UTF8.to_owned()) && mimes.contains(&"text/plain".to_owned()),
        "{mimes:?}"
    );
    let got = peer.read(selection, via, UTF8);
    assert_eq!(got.text.as_deref(), Some(TEXT));
    peer.wait_event(
        "its source served",
        |e| matches!(e, Event::SourceSend { mime } if mime == UTF8),
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_round_trips_text_through_the_wayland_data_device() {
    let s = Session::start();
    let peer = focused_peer(&s, Display::Wayland, "w3 data device");
    round_trip(&peer, Selection::Clipboard, Via::DataDevice);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_round_trips_text_through_data_control() {
    // Data control needs no window and no focus, and has the primary selection too.
    let s = Session::start();
    let peer = s.peer(Display::Wayland);
    round_trip(&peer, Selection::Clipboard, Via::DataControl);
    round_trip(&peer, Selection::Primary, Via::DataControl);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_round_trips_text_through_x11_selections() {
    let s = Session::start();
    let peer = s.peer(Display::X11);
    round_trip(&peer, Selection::Clipboard, Via::X11);
    round_trip(&peer, Selection::Primary, Via::X11);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_reads_a_large_x11_selection_in_increments() {
    // 4 MiB is far above the 64 KiB the peer sends at once, so both ends run INCR.
    let (len, seed) = (4 * 1024 * 1024, 7);
    let s = Session::start();
    let peer = s.peer(Display::X11);
    let mime = "application/x-strata-w3";
    peer.set_selection(
        Selection::Clipboard,
        Via::X11,
        &[Offer {
            mime: mime.into(),
            data: Payload::Pattern { len, seed },
        }],
    );
    let got = peer.read(Selection::Clipboard, Via::X11, mime);
    assert_eq!(got.len, len);
    assert_eq!(got.fnv1a64, fnv1a64(&pattern(len, seed)));
    let file = got.file.expect("large data arrives in a file");
    assert_eq!(std::fs::metadata(file).map(|m| m.len()).ok(), Some(len));
}

/// A second peer reads the first's clipboard, then takes it, and the first hears it lost it.
fn second_peer_reads_then_takes(s: &Session, display: Display, via: Via) {
    let a = s.peer(display);
    let b = s.peer(display);
    a.set_selection(Selection::Clipboard, via, &text_offers());
    b.sync();
    let got = b.read(Selection::Clipboard, via, UTF8);
    assert_eq!(got.text.as_deref(), Some(TEXT));
    b.set_selection(Selection::Clipboard, via, &[Offer::text(UTF8, "b's turn")]);
    a.wait_event("losing the clipboard", |e| {
        matches!(
            e,
            Event::SelectionCancelled {
                selection: Selection::Clipboard,
                ..
            }
        )
    });
    a.sync();
    assert_eq!(
        a.read(Selection::Clipboard, via, UTF8).text.as_deref(),
        Some("b's turn")
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_second_wayland_peer_reads_and_takes_the_clipboard() {
    let s = Session::start();
    second_peer_reads_then_takes(&s, Display::Wayland, Via::DataControl);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_second_x11_peer_reads_and_takes_the_clipboard() {
    let s = Session::start();
    second_peer_reads_then_takes(&s, Display::X11, Via::X11);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn the_session_bus_is_private() {
    let s = Session::with(Options {
        compositor: false,
        ..Options::default()
    });
    assert!(
        s.bus()
            .address()
            .contains(&*s.runtime_dir().to_string_lossy()),
        "{} is outside the session",
        s.bus().address()
    );
    // A service added to the session's bus becomes activatable; nothing else is.
    s.bus().add_service("org.strata.W3Probe", "/bin/false");
    assert!(
        s.bus()
            .service_dir()
            .join("org.strata.W3Probe.service")
            .is_file()
    );
    let mut cmd = s.command("dbus-send", Display::None);
    cmd.args([
        "--session",
        "--print-reply",
        "--dest=org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus.ListActivatableNames",
    ]);
    let mut send = s.spawn("dbus-send", cmd);
    send.wait_line(0, "dbus-send's reply", |l| {
        l.trim_start().starts_with("array [")
    });
    assert!(send.wait_exit().success(), "dbus-send failed");
    let names: Vec<String> = send
        .lines()
        .iter()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("string \"")?
                .strip_suffix('"')
                .map(str::to_owned)
        })
        .collect();
    let mut names = names;
    names.sort();
    assert_eq!(names, ["org.freedesktop.DBus", "org.strata.W3Probe"]);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn teardown_leaves_no_processes_or_files() {
    let s = Session::start();
    let (app, _, _) = harness_window(&s, Display::X11, "w3 teardown");
    let peer = s.peer(Display::Wayland);
    let xwayland = s.xwayland().expect("Xwayland runs for the X11 window");
    let dir = s.runtime_dir().to_owned();
    drop(peer);
    drop(app);
    drop(s);
    let left = leftovers(&dir, Some(xwayland));
    assert!(left.is_empty(), "left behind: {left:?}");
}

/// What a session left behind: processes started with its directory, Xwayland, the directory
/// itself, and Xwayland's lock file and socket.
fn leftovers(dir: &Path, xwayland: Option<(u32, u32)>) -> Vec<String> {
    let mut left: Vec<String> = processes_of(dir)
        .into_iter()
        .map(|p| format!("process {p}"))
        .collect();
    let mut paths = vec![dir.to_owned()];
    if let Some((pid, n)) = xwayland {
        if alive(pid) {
            left.push(format!("Xwayland {pid}"));
        }
        paths.push(PathBuf::from(format!("/tmp/.X{n}-lock")));
        paths.push(PathBuf::from(format!("/tmp/.X11-unix/X{n}")));
    }
    left.extend(
        paths
            .into_iter()
            .filter(|p| p.exists())
            .map(|p| p.display().to_string()),
    );
    left
}

/// The X lock files in /tmp, with what they say: the pid of the server that holds the display.
fn x_locks() -> BTreeMap<PathBuf, String> {
    let Ok(entries) = std::fs::read_dir("/tmp") else {
        return BTreeMap::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(".X") && n.ends_with("-lock"))
        })
        .map(|p| {
            let owner = std::fs::read_to_string(&p).unwrap_or_default();
            (p, owner.trim().to_owned())
        })
        .collect()
}

/// Locks that weren't in `before` and name a process that is gone: what a killed session
/// leaked. Locks that were there already (another X server's, a stray empty one) don't count,
/// nor do the live sessions of tests running alongside.
fn leaked_locks(before: &BTreeMap<PathBuf, String>) -> Vec<String> {
    x_locks()
        .into_iter()
        .filter(|(path, owner)| before.get(path) != Some(owner))
        .filter(|(_, owner)| owner.parse::<u32>().is_ok_and(|pid| !alive(pid)))
        .map(|(path, owner)| format!("{} (pid {owner})", path.display()))
        .collect()
}

/// The child of the SIGKILL and SIGINT tests: with `STRATA_SESSION_REPORT` set, it starts a
/// session, opens an X11 window (so Xwayland runs), reports, and waits to be stopped. Without
/// it, it does nothing.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn sigkill_child() {
    if std::env::var_os("STRATA_SESSION_REPORT").is_none() {
        return;
    }
    let s = Session::start();
    let (_app, _, _) = harness_window(&s, Display::X11, "w3 sigkill");
    if let Some((pid, display)) = s.xwayland() {
        report(&format!("xwayland {pid} {display}"));
    }
    report("window");
    std::thread::sleep(timeout() * 6);
}

/// A child test binary, killed if the test fails before it stops it.
struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// How a child test binary is stopped.
#[derive(Clone, Copy, Debug)]
enum Stop {
    Kill,
    /// A signal to its whole process group, as Ctrl-C sends SIGINT.
    Group(&'static str),
}

/// Runs `w3::sigkill_child` in a test binary of its own, in a process group of its own, stops it
/// `delay_ms` after it reports `point`, and checks the watchdog leaves nothing behind: no
/// process started with the session's directory, no directory, no new lock file naming a dead
/// server. With `decoy`, a process outside the session, whose pid sits in the session's `pids`
/// file as a reused number would, must survive.
fn stop_child(point: &str, delay_ms: u64, stop: Stop, decoy: bool) {
    let what = format!("{stop:?} at {point:?} +{delay_ms} ms");
    // Both kill tests run children at once: each child gets its own report and log names.
    let tag = format!(
        "{}-{}-{delay_ms}",
        point,
        match stop {
            Stop::Kill => "kill",
            Stop::Group(signal) => signal,
        }
    )
    .to_lowercase();
    let reports = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("session");
    std::fs::create_dir_all(&reports).expect("report directory");
    let report = reports.join(format!("child-{tag}.report"));
    let _ = std::fs::remove_file(&report);
    let locks = x_locks();
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", "w3::sigkill_child", "--ignored", "--nocapture"])
            .env("STRATA_SESSION_REPORT", &report)
            .env("STRATA_SESSION_NAME", format!("w3-child-{tag}"))
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the child test binary"),
    );
    let read = || std::fs::read_to_string(&report).unwrap_or_default();
    let deadline = Instant::now() + timeout();
    while !read().lines().any(|l| l == point) {
        assert!(
            Instant::now() < deadline,
            "{what}: the child never reported it: {}",
            read()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(delay_ms));
    let text = read();
    let dir = PathBuf::from(
        text.lines()
            .find_map(|l| l.strip_prefix("dir "))
            .expect("the session directory"),
    );
    let mut decoy = decoy.then(|| {
        let decoy = Command::new("sleep")
            .arg("300")
            .env_clear()
            .env("XDG_RUNTIME_DIR", "/run/user/not-this-session")
            .spawn()
            .expect("a decoy process");
        let mut pids = std::fs::File::options()
            .append(true)
            .open(dir.join("pids"))
            .expect("the session's pids file");
        writeln!(pids, "{}", decoy.id()).expect("planting the decoy's pid");
        decoy
    });
    match stop {
        Stop::Kill => child.0.kill().expect("SIGKILL"),
        Stop::Group(signal) => {
            let sent = Command::new("kill")
                .arg(format!("-{signal}"))
                .arg("--")
                .arg(format!("-{}", child.0.id()))
                .status()
                .expect("kill");
            assert!(sent.success(), "{what}: kill failed");
        }
    }
    child.0.wait().expect("reaping the child");

    let xwayland = read().lines().find_map(|l| {
        let mut parts = l.strip_prefix("xwayland ")?.split(' ');
        Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
    });
    let deadline = Instant::now() + timeout();
    loop {
        let mut left = leftovers(&dir, xwayland);
        left.extend(leaked_locks(&locks));
        if left.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "{what}: left behind: {left:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(decoy) = decoy.as_mut() {
        let survived = alive(decoy.id());
        let _ = decoy.kill();
        let _ = decoy.wait();
        assert!(
            survived,
            "{what}: the watchdog killed a process outside the session"
        );
    }
}

/// SIGKILLs a test binary while sway starts (at four points in its first 120 ms, where it has
/// made Xwayland's lock and socket but not yet said which display they are), and once its
/// session is up with an X11 window, with a decoy pid in its `pids` file.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_killed_test_leaves_no_processes_or_files() {
    for delay in [0, 40, 80, 120] {
        stop_child("sway-starting", delay, Stop::Kill, false);
    }
    stop_child("window", 0, Stop::Kill, true);
}

/// Ctrl-C (SIGINT) and SIGTERM to a test binary's whole process group: the session's processes
/// go with it, and the watchdog, in a group of its own that ignores both, cleans up.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn an_interrupted_test_leaves_no_processes_or_files() {
    stop_child("window", 0, Stop::Group("INT"), false);
    stop_child("window", 0, Stop::Group("TERM"), false);
}

// ---- drag and drop, and export ----

/// The input peer drags from peer `a`'s window to peer `b`'s; `b` reads the drop. With
/// `proxy` (X11), `b`'s drag messages go through its `XdndProxy`.
fn drag_between_peers(display: Display, proxy: bool) {
    let s = Session::start();
    let a = s.peer(display);
    let b = s.peer(display);
    // sway tiles them side by side. The syncs make sure both have drawn at their tiled size;
    // point_at makes sure the press reaches `a` (docs/testing.md, "Point, then press").
    a.map("w3 drag source");
    b.map("w3 drop target");
    let [from, to] = <[_; 2]>::try_from(s.wait_windows(&[a.pid(), b.pid()])).expect("two windows");
    a.sync();
    b.sync();
    b.accept_drops(
        &["text/plain", UTF8],
        &[Action::Copy, Action::Move],
        Action::Copy,
    );
    if proxy {
        b.drop_proxy();
    }

    let input = s.input();
    s.point_at(&a, from.at(0.5, 0.5));
    input.button(true);
    a.wait_event("the press", |e| {
        matches!(e, Event::PointerButton { pressed: true })
    });
    let offers = [
        Offer::text("text/uri-list", "file:///tmp/w3\r\n"),
        Offer::text("text/plain", TEXT),
        Offer::text("application/x-strata-w3", "private"),
    ];
    a.drag_start(&offers, &[Action::Copy, Action::Move]);
    input.path(to.at(0.5, 0.5));
    let entered = b.wait_event("the drag entering", |e| {
        matches!(e, Event::DropEnter { .. })
    });
    let Event::DropEnter { mimes, proxied } = entered else {
        unreachable!()
    };
    assert_eq!(mimes.len(), 3, "{mimes:?}");
    assert_eq!(proxied, proxy, "the drag's messages came the wrong way");
    b.wait_event("the drag moving", |e| matches!(e, Event::DropMotion { .. }));
    // Release only once the source knows the target takes it, with Copy.
    a.wait_event("the target accepting Copy", |e| {
        matches!(
            e,
            Event::DragAction {
                action: Some(Action::Copy)
            }
        )
    });
    input.button(false);
    let dropped = b.wait_event("the drop", |e| matches!(e, Event::Dropped { .. }));
    let Event::Dropped { mime, action, data } = dropped else {
        unreachable!()
    };
    assert_eq!(mime, "text/plain");
    assert_eq!(action, Some(Action::Copy));
    assert_eq!(data.text.as_deref(), Some(TEXT));
    a.wait_event("the drag finishing", |e| {
        matches!(
            e,
            Event::DragFinished {
                action: Some(Action::Copy)
            }
        )
    });
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_drags_to_peer_on_wayland() {
    drag_between_peers(Display::Wayland, false);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_drags_to_peer_on_x11() {
    drag_between_peers(Display::X11, false);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_drags_to_peer_through_an_xdnd_proxy() {
    drag_between_peers(Display::X11, true);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peers_export_their_windows() {
    let s = Session::start();
    let wayland = s.peer(Display::Wayland);
    wayland.map("w3 export wayland");
    let handle = wayland.export();
    assert!(
        handle
            .strip_prefix("wayland:")
            .is_some_and(|h| !h.is_empty()),
        "{handle:?}"
    );
    let x11 = s.peer(Display::X11);
    let xid = x11.map("w3 export x11").xid.expect("an X window");
    assert_eq!(x11.export(), format!("x11:{xid:x}"));
}

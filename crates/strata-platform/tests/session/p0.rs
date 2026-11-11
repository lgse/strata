//! P0's session tests: offers in and sources out, through the peer, on Wayland and X11, plus
//! drags into a Wayland window and the substrate's lifecycle (a hidden window, hide and show,
//! quit and close). The app side is `examples/transfer-probe.rs`.
//!
//! Tests marked "shared" need winit's own display, which P0 reaches only on libwayland 1.23
//! or later. On an older host they check that the probe reports that route Unsupported (the
//! specified result there) and say so on stderr; nothing else of theirs ran.
//!
//! Work package: P0, `docs/work/phase-1-leaves/P0-transfer-substrate.md`.

use std::path::Path;

use serde_json::{Value, json};
use strata_platform::transfer::mime;
use strata_test_peer::protocol::{Action, Event, Offer, Payload, Selection, Via, fnv1a64};

use crate::support::{Display, Peer, Proc, Session, Window, example, timeout};

const UTF8: &str = "text/plain;charset=utf-8";
const TEXT: &str = "héllo from P0 ✓";
const PNG: &str = "image/png";

/// Three types: a file list made with P0's encoder from a path that isn't UTF-8, text, and
/// 4 MiB, so X11 runs INCR both ways and Wayland pipes fill.
fn three() -> Vec<Offer> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let uris = [
        mime::file_uri(Path::new(OsStr::from_bytes(b"/tmp/p0 \xff\xfe 100%.txt"))),
        mime::file_uri(Path::new("/tmp/café#2")),
    ];
    let list = String::from_utf8(mime::encode_uri_list(&uris)).expect("URIs are ASCII");
    vec![
        Offer::text(mime::URI_LIST, &list),
        Offer::text(UTF8, TEXT),
        Offer {
            mime: PNG.into(),
            data: Payload::Pattern {
                len: 4 << 20,
                seed: 23,
            },
        },
    ]
}

fn mimes(offers: &[Offer]) -> Vec<String> {
    offers.iter().map(|o| o.mime.clone()).collect()
}

fn fnv(offer: &Offer) -> String {
    fnv1a64(&offer.data.bytes().expect("offer bytes"))
}

fn via_name(via: Via) -> &'static str {
    match via {
        Via::DataDevice => "data-device",
        Via::DataControl => "data-control",
        Via::X11 => "x11",
    }
}

/// Whether this host's libwayland-client has `wl_proxy_get_display` (1.23 or later): checked
/// here, apart from the probe, so a probe that wrongly reports Unsupported can't pass.
#[expect(unsafe_code, reason = "dlopen and dlsym of libwayland-client")]
fn libwayland_has_get_display() -> bool {
    // SAFETY: a NUL-terminated name; loading libwayland-client runs no code of ours.
    let lib = unsafe {
        libc::dlopen(
            c"libwayland-client.so.0".as_ptr(),
            libc::RTLD_NOW | libc::RTLD_LOCAL,
        )
    };
    if lib.is_null() {
        return false;
    }
    // SAFETY: `lib` is a live handle and the symbol name is NUL-terminated. Never closed.
    let sym = unsafe { libc::dlsym(lib, c"wl_proxy_get_display".as_ptr()) };
    !sym.is_null()
}

/// The probe app and its window.
struct Probe {
    app: Proc,
    window: Window,
    attached: Value,
}

impl Probe {
    fn start(s: &Session, display: Display, args: &[&str]) -> Probe {
        Probe::start_with(s, display, args, &[])
    }

    /// As [`Probe::start`], with more of the session's environment (`DISPLAY` beside
    /// `WAYLAND_DISPLAY`, say).
    fn start_with(s: &Session, display: Display, args: &[&str], also: &[Display]) -> Probe {
        let mut cmd = s.command(example("transfer-probe"), display);
        for d in also {
            cmd.envs(s.env(*d));
        }
        cmd.args(["--title", "p0 probe"]).args(args);
        let app = s.spawn("probe", cmd);
        let attached = app.wait_json("attach", |v| {
            v["ev"] == "attached" || v["ev"] == "attach-error"
        });
        assert_eq!(attached["ev"], "attached", "{attached}");
        app.wait_json("the first frame", |v| v["ev"] == "frame");
        let window = s.wait_window(app.pid());
        let probe = Probe {
            app,
            window,
            attached,
        };
        probe.wait_layout();
        probe
    }

    /// Waits until the app has laid out at the size sway gave its window.
    fn wait_layout(&self) {
        let (w, h) = (self.window.content.width, self.window.content.height);
        self.app.wait_json(&format!("a layout at {w}x{h}"), |v| {
            v["ev"] == "layout"
                && (v["w"].as_f64().unwrap_or(0.0) - w).abs() < 1.0
                && (v["h"].as_f64().unwrap_or(0.0) - h).abs() < 1.0
        });
    }

    fn send(&self, cmd: Value) -> usize {
        let mark = self.app.line_count();
        self.app.send_line(&cmd.to_string());
        mark
    }

    fn wait(&self, from: usize, what: &str, pred: impl Fn(&Value) -> bool) -> (usize, Value) {
        self.app.wait_json_from(from, what, pred)
    }

    /// Takes a selection; panics unless the probe reports it owned.
    fn own(&self, via: Via, selection: Selection, offers: &[Offer]) -> u64 {
        let at = self.send(json!({
            "cmd": "own", "via": via, "selection": selection, "offers": offers,
        }));
        let (_, v) = self.wait(at, "own", |v| v["ev"] == "owned" || v["ev"] == "own-error");
        assert_eq!(v["ev"], "owned", "{v}");
        v["source"].as_u64().expect("a source id")
    }

    /// Reads one type; the probe's report of it.
    fn read(&self, via: Via, selection: Selection, mime: &str) -> Value {
        let at =
            self.send(json!({ "cmd": "read", "via": via, "selection": selection, "mime": mime }));
        let (_, v) = self.wait(at, &format!("the read of {mime}"), |v| {
            (v["ev"] == "read" || v["ev"] == "read-error") && v["mime"] == mime
        });
        assert_eq!(v["ev"], "read", "{v}");
        v
    }

    /// How many times the lazy writer of `mime` has run.
    fn writer_runs(&self, mime: &str) -> usize {
        self.app
            .lines()
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["ev"] == "writer" && v["mime"] == mime)
            .count()
    }

    /// Waits until the lazy writer of `mime` has run at least `n` times (its log line may
    /// trail the read it served), then checks it ran exactly `n` times.
    fn assert_writer_runs(&self, mime: &str, n: usize, why: &str) {
        let deadline = std::time::Instant::now() + timeout();
        while self.writer_runs(mime) < n && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(self.writer_runs(mime), n, "{mime}: {why}");
    }

    /// For a test that needs winit's display: whether it is here. Below libwayland 1.23 the
    /// probe must report it Unsupported, which is all the test then checks.
    fn shared(&self) -> bool {
        let have = libwayland_has_get_display();
        assert_eq!(self.attached["shared"], have, "{}", self.attached);
        if !have {
            let why = self.attached["unsupported"].as_str().unwrap_or_default();
            assert!(why.contains("libwayland < 1.23"), "{}", self.attached);
            eprintln!(
                "libwayland < 1.23 here: only checked that winit's display is reported \
                 Unsupported; the shared route didn't run"
            );
        }
        have
    }

    fn no_errors(&self) {
        let bad: Vec<String> = self
            .app
            .lines()
            .into_iter()
            .filter(|l| {
                [
                    "exit-error",
                    "\"failed\"",
                    "drag-error",
                    "drop-error",
                    "command-error",
                ]
                .iter()
                .any(|e| l.contains(e))
            })
            .collect();
        assert!(bad.is_empty(), "the probe reported errors: {bad:?}");
    }
}

/// Polls the peer until the selection lists every one of `want`.
fn wait_targets(peer: &Peer, selection: Selection, via: Via, want: &[String]) -> Vec<String> {
    let deadline = std::time::Instant::now() + timeout();
    loop {
        peer.sync();
        let got = peer.targets(selection, via).unwrap_or_default();
        if want.iter().all(|m| got.contains(m)) {
            return got;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the peer's {via:?} selection offers {got:?}, not {want:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// The window offers three types; the peer lists them and reads each. Each writer runs only
/// when the peer asks for its type, and once however often it asks.
fn window_offers_three(
    probe: &Probe,
    peer: &Peer,
    own_via: Via,
    peer_via: Via,
    selection: Selection,
) {
    let offers = three();
    probe.own(own_via, selection, &offers);
    wait_targets(peer, selection, peer_via, &mimes(&offers));
    for o in &offers {
        assert_eq!(
            probe.writer_runs(&o.mime),
            0,
            "{} ran before anyone asked",
            o.mime
        );
    }
    for _ in 0..2 {
        let got = peer.read(selection, peer_via, &offers[0].mime);
        assert_eq!(got.fnv1a64, fnv(&offers[0]), "{}", offers[0].mime);
    }
    let got = peer.read(selection, peer_via, &offers[1].mime);
    assert_eq!(got.text.as_deref(), Some(TEXT));
    probe.assert_writer_runs(&offers[0].mime, 1, "read twice, written once");
    probe.assert_writer_runs(&offers[1].mime, 1, "read once");
    assert_eq!(probe.writer_runs(PNG), 0, "nobody asked for it yet");
    let got = peer.read(selection, peer_via, PNG);
    assert_eq!(
        (got.len, got.fnv1a64.as_str()),
        (4 << 20, fnv(&offers[2]).as_str())
    );
    probe.assert_writer_runs(PNG, 1, "read once");
    // Again, now that a late line from a second run would have arrived.
    probe.assert_writer_runs(&offers[0].mime, 1, "read twice, written once");
    probe.no_errors();
}

/// The peer offers three types; the window hears them and reads each.
fn peer_offers_three(probe: &Probe, at: usize, probe_via: Via, selection: Selection) {
    let offers = three();
    let want = mimes(&offers);
    let (_, heard) = probe.wait(at, "the peer's selection", |v| {
        v["ev"] == "selection"
            && v["via"] == via_name(probe_via)
            && v["mimes"]
                .as_array()
                .is_some_and(|m| want.iter().all(|w| m.iter().any(|x| x == w.as_str())))
    });
    assert!(heard["mimes"].is_array(), "{heard}");
    for o in &offers {
        let got = probe.read(probe_via, selection, &o.mime);
        assert_eq!(got["fnv1a64"], fnv(o), "{}: {got}", o.mime);
    }
    probe.no_errors();
}

// ---- Wayland: the window offers ----

fn wayland_window_offers_over_its_own_data_control(prefer: &str) {
    let s = Session::start();
    let probe = Probe::start(
        &s,
        Display::Wayland,
        &["--own-data-control", "--data-control", prefer],
    );
    assert_eq!(probe.attached["own"], true, "{}", probe.attached);
    assert_eq!(probe.attached["data_control"], prefer, "{}", probe.attached);
    let peer = s.peer(Display::Wayland);
    window_offers_three(
        &probe,
        &peer,
        Via::DataControl,
        Via::DataControl,
        Selection::Clipboard,
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_window_offers_three_types_over_its_own_ext_data_control() {
    wayland_window_offers_over_its_own_data_control("ext");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_window_offers_three_types_over_its_own_wlr_data_control() {
    wayland_window_offers_over_its_own_data_control("wlr");
}

/// Shared.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_window_offers_three_types_over_shared_data_control() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &[]);
    if !probe.shared() {
        assert_eq!(
            probe.attached["own"], true,
            "data control falls back to its own connection"
        );
        return;
    }
    assert_eq!(probe.attached["own"], false, "{}", probe.attached);
    let peer = s.peer(Display::Wayland);
    window_offers_three(
        &probe,
        &peer,
        Via::DataControl,
        Via::DataControl,
        Selection::Clipboard,
    );
    // Data control has the primary selection too.
    let one = [Offer::text(UTF8, "primary from P0")];
    probe.own(Via::DataControl, Selection::Primary, &one);
    wait_targets(&peer, Selection::Primary, Via::DataControl, &mimes(&one));
    let got = peer.read(Selection::Primary, Via::DataControl, UTF8);
    assert_eq!(got.text.as_deref(), Some("primary from P0"));
}

/// Shared: a click gives the window a serial, and the data device takes the selection with
/// it. The window hears its own selection echoed back (how a dropped write would show).
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_window_offers_three_types_through_the_data_device() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &[]);
    if !probe.shared() {
        let at = probe.send(json!({
            "cmd": "own", "via": Via::DataDevice, "selection": Selection::Clipboard, "offers": three(),
        }));
        let (_, v) = probe.wait(at, "own", |v| v["ev"] == "own-error");
        assert!(
            v["error"]
                .as_str()
                .unwrap_or_default()
                .contains("data device"),
            "{v}"
        );
        return;
    }
    let peer = s.peer(Display::Wayland);
    let at = probe.app.line_count();
    s.input().click_at(probe.window.at(0.5, 0.5));
    probe.wait(at, "the click", |v| v["ev"] == "press");
    let at = probe.app.line_count();
    window_offers_three(
        &probe,
        &peer,
        Via::DataDevice,
        Via::DataControl,
        Selection::Clipboard,
    );
    let want = mimes(&three());
    probe.wait(at, "our selection echoed on the data device", |v| {
        v["ev"] == "selection"
            && v["via"] == "data-device"
            && v["mimes"].as_array().is_some_and(|m| m.len() == want.len())
    });
}

/// Shared: wlroots drops a data-device write whose serial is older than the current
/// selection's, with no error and no event (S2 Q5). The peer's data-control write takes a
/// serial newer than the click's, so the probe's write with the click's serial is dropped:
/// P0 reports its source cancelled and no longer owned. A write after a fresh click holds.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_data_device_write_the_compositor_drops_is_reported_cancelled() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &[]);
    if !probe.shared() {
        return;
    }
    let peer = s.peer(Display::Wayland);
    let at = probe.app.line_count();
    s.input().click_at(probe.window.at(0.5, 0.5));
    probe.wait(at, "the click", |v| v["ev"] == "press");
    peer.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[Offer::text(UTF8, "the peer's")],
    );
    probe.wait(at, "the peer's selection on the data device", |v| {
        v["ev"] == "selection"
            && v["via"] == "data-device"
            && v["mimes"].as_array().is_some_and(|m| m.len() == 1)
    });
    let at = probe.app.line_count();
    let dropped = probe.own(
        Via::DataDevice,
        Selection::Clipboard,
        &[Offer::text(UTF8, TEXT)],
    );
    probe.wait(at, "the dropped write reported cancelled", |v| {
        v["ev"] == "cancelled" && v["source"] == dropped
    });
    let at = probe.send(json!({ "cmd": "owns", "source": dropped }));
    let (_, v) = probe.wait(at, "owns", |v| v["ev"] == "owns");
    assert_eq!(v["owns"], false, "{v}");
    let got = peer.read(Selection::Clipboard, Via::DataControl, UTF8);
    assert_eq!(got.text.as_deref(), Some("the peer's"));

    let at = probe.app.line_count();
    s.input().click_at(probe.window.at(0.5, 0.5));
    probe.wait(at, "the second click", |v| v["ev"] == "press");
    let at = probe.app.line_count();
    let held = probe.own(
        Via::DataDevice,
        Selection::Clipboard,
        &[Offer::text(UTF8, TEXT)],
    );
    probe.wait(at, "the write echoed", |v| {
        v["ev"] == "selection" && v["via"] == "data-device"
    });
    // The peer's read goes through the source, long after the sync came back.
    let got = peer.read(Selection::Clipboard, Via::DataControl, UTF8);
    assert_eq!(got.text.as_deref(), Some(TEXT));
    let at2 = probe.send(json!({ "cmd": "owns", "source": held }));
    let (_, v) = probe.wait(at2, "owns", |v| v["ev"] == "owns");
    assert_eq!(v["owns"], true, "{v}");
    assert!(
        !probe.app.lines()[at..]
            .iter()
            .any(|l| l.contains("\"cancelled\"")),
        "a write that took was reported cancelled"
    );
    probe.no_errors();
}

/// libwayland < 1.23 (Ubuntu 24.04, an unsupported environment), on any host: the probe's
/// test switch makes the lookup of `wl_proxy_get_display` find nothing. winit's display is
/// reported Unsupported, the data device is out of reach, and data control falls back to a
/// connection of its own, both ways.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn without_winits_display_a_wayland_window_falls_back_to_its_own_data_control() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &["--as-if-libwayland-1-22"]);
    let a = &probe.attached;
    assert_eq!(
        (a["shared"].as_bool(), a["own"].as_bool()),
        (Some(false), Some(true)),
        "{a}"
    );
    let why = a["unsupported"].as_str().unwrap_or_default();
    assert!(why.contains("libwayland < 1.23"), "{a}");
    assert!(a["data_control"].is_string(), "{a}");
    let at = probe.send(json!({
        "cmd": "own", "via": Via::DataDevice, "selection": Selection::Clipboard, "offers": three(),
    }));
    let (_, v) = probe.wait(at, "own", |v| v["ev"] == "own-error");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .contains("data device"),
        "{v}"
    );
    let peer = s.peer(Display::Wayland);
    window_offers_three(
        &probe,
        &peer,
        Via::DataControl,
        Via::DataControl,
        Selection::Clipboard,
    );
    let at = probe.app.line_count();
    peer.set_selection(Selection::Clipboard, Via::DataControl, &three());
    peer_offers_three(&probe, at, Via::DataControl, Selection::Clipboard);
}

/// libwayland < 1.23 and no data control (mutter's case, Ubuntu 24.04's GNOME, an unsupported
/// environment), simulated: no Wayland route is left, so the selections go through Xwayland.
/// The window's X11 copy reaches an X11 peer, and a Wayland peer through the compositor's
/// bridge, and the window reads the Wayland peer's copy back the same way. wlroots bridges the
/// clipboard only while an Xwayland window has keyboard focus (its xwm denies both directions
/// otherwise), so the X11 peer's window has it here. wlroots compositors have data control and
/// never take this route; mutter's bridge, the one it is for, gets no hand test, as 24.04 is
/// unsupported.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn without_any_wayland_route_a_wayland_window_takes_x11_selections_through_xwayland() {
    let s = Session::start();
    let x_peer = s.peer(Display::X11);
    let probe = Probe::start_with(
        &s,
        Display::Wayland,
        &[
            "--as-if-libwayland-1-22",
            "--as-if-no-data-control",
            "--x11-fallback",
        ],
        &[Display::X11],
    );
    let a = &probe.attached;
    assert_eq!(a["backend"], "wayland", "{a}");
    assert_eq!(
        (
            a["shared"].as_bool(),
            a["own"].as_bool(),
            a["x11"].as_bool()
        ),
        (Some(false), Some(false), Some(true)),
        "{a}"
    );
    x_peer.map("p0 x11 peer");
    let windows = s.wait_windows(&[probe.app.pid(), x_peer.pid()]);
    let x_window = windows
        .iter()
        .find(|w| w.pid == x_peer.pid())
        .expect("the X11 peer's window");
    s.focus(x_window);
    x_peer.wait_event("focus", |e| matches!(e, Event::Focus { focused: true }));
    let wl_peer = s.peer(Display::Wayland);
    let offers = three();
    probe.own(Via::X11, Selection::Clipboard, &offers);
    wait_targets(&x_peer, Selection::Clipboard, Via::X11, &mimes(&offers));
    wait_targets(
        &wl_peer,
        Selection::Clipboard,
        Via::DataControl,
        &mimes(&offers),
    );
    for o in &offers {
        // The 4 MiB image goes by INCR on X11 to the X11 peer.
        assert_eq!(
            x_peer.read(Selection::Clipboard, Via::X11, &o.mime).fnv1a64,
            fnv(o)
        );
        // Through sway's bridge only small data: wlroots writes each INCR chunk into the
        // Wayland reader's non-blocking pipe without waiting for it to drain, and drops the
        // transfer on EAGAIN (`xwayland/selection/incoming.c:112`, "write error to target fd
        // …: Resource temporarily unavailable"), so a large type arrives cut short under load.
        // That is wlroots' bug, not this route's.
        if o.data.bytes().map_or(0, |b| b.len()) >= 64 << 10 {
            continue;
        }
        assert_eq!(
            wl_peer
                .read(Selection::Clipboard, Via::DataControl, &o.mime)
                .fnv1a64,
            fnv(o),
            "{} through the bridge",
            o.mime
        );
    }
    let at = probe.app.line_count();
    wl_peer.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[Offer::text(UTF8, "the Wayland peer's")],
    );
    // The bridge names UTF-8 text by its X11 target.
    probe.wait(at, "the Wayland peer's copy, through Xwayland", |v| {
        v["ev"] == "selection"
            && v["via"] == "x11"
            && v["mimes"]
                .as_array()
                .is_some_and(|m| m.iter().any(|m| m == "UTF8_STRING"))
    });
    let got = probe.read(Via::X11, Selection::Clipboard, "UTF8_STRING");
    assert_eq!(got["text"], "the Wayland peer's");
    probe.no_errors();
}

/// No Wayland route, and the X11 fallback finds no X server where `DISPLAY` points: the
/// window has no route at all, which is Unsupported (not a connection error).
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn without_any_route_and_no_x_server_a_wayland_window_is_unsupported() {
    let s = Session::start();
    let mut cmd = s.command(example("transfer-probe"), Display::Wayland);
    // A display number no server listens on, in the session's own runtime directory.
    cmd.env("DISPLAY", ":250");
    cmd.args([
        "--title",
        "p0 probe",
        "--as-if-libwayland-1-22",
        "--as-if-no-data-control",
        "--x11-fallback",
    ]);
    let app = s.spawn("probe", cmd);
    let v = app.wait_json("attach", |v| {
        v["ev"] == "attached" || v["ev"] == "attach-error"
    });
    assert_eq!(v["ev"], "attach-error", "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .starts_with("unsupported"),
        "{v}"
    );
}

// ---- Wayland: the peer offers ----

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_offers_three_types_to_a_wayland_window_over_its_own_data_control() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &["--own-data-control"]);
    let peer = s.peer(Display::Wayland);
    let at = probe.app.line_count();
    peer.set_selection(Selection::Clipboard, Via::DataControl, &three());
    peer_offers_three(&probe, at, Via::DataControl, Selection::Clipboard);
}

/// Shared.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_offers_three_types_to_a_wayland_window_over_shared_data_control() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &[]);
    if !probe.shared() {
        return;
    }
    let peer = s.peer(Display::Wayland);
    let at = probe.app.line_count();
    peer.set_selection(Selection::Clipboard, Via::DataControl, &three());
    peer_offers_three(&probe, at, Via::DataControl, Selection::Clipboard);
}

/// Shared: the data device tells a client the selection only when it gains keyboard focus.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_offers_three_types_to_a_wayland_window_through_the_data_device() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &[]);
    if !probe.shared() {
        return;
    }
    let peer = s.peer(Display::Wayland);
    peer.map("p0 peer");
    let windows = s.wait_windows(&[probe.app.pid(), peer.pid()]);
    let peer_window = windows
        .iter()
        .find(|w| w.pid == peer.pid())
        .expect("the peer's window");
    s.focus(peer_window);
    peer.wait_event("focus", |e| matches!(e, Event::Focus { focused: true }));
    peer.set_selection(Selection::Clipboard, Via::DataDevice, &three());
    let at = probe.app.line_count();
    let probe_window = windows
        .iter()
        .find(|w| w.pid == probe.app.pid())
        .expect("the probe's window");
    s.focus(probe_window);
    peer_offers_three(&probe, at, Via::DataDevice, Selection::Clipboard);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_wayland_window_hears_it_lost_the_clipboard() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &["--own-data-control"]);
    let peer = s.peer(Display::Wayland);
    let at = probe.app.line_count();
    let source = probe.own(
        Via::DataControl,
        Selection::Clipboard,
        &[Offer::text(UTF8, TEXT)],
    );
    wait_targets(
        &peer,
        Selection::Clipboard,
        Via::DataControl,
        &[UTF8.into()],
    );
    peer.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[Offer::text(UTF8, "the peer's")],
    );
    probe.wait(at, "losing the clipboard", |v| {
        v["ev"] == "cancelled" && v["source"] == source
    });
    let got = probe.read(Via::DataControl, Selection::Clipboard, UTF8);
    assert_eq!(got["text"], "the peer's");
    probe.no_errors();
}

// ---- Wayland: drags in ----

/// The input peer drags three types from peer `a`'s window into the probe's; the probe
/// accepts Copy, reads every type on the drop and finishes it.
fn drag_into_probe(s: &Session, probe: &Probe) {
    let a = s.peer(Display::Wayland);
    a.map("p0 drag source");
    let windows = s.wait_windows(&[a.pid(), probe.app.pid()]);
    let from = windows
        .iter()
        .find(|w| w.pid == a.pid())
        .expect("a's window");
    let to = windows
        .iter()
        .find(|w| w.pid == probe.app.pid())
        .expect("the probe's window");
    a.sync();
    let input = s.input();
    s.point_at(&a, from.at(0.5, 0.5));
    input.button(true);
    a.wait_event("the press", |e| {
        matches!(e, Event::PointerButton { pressed: true })
    });
    let offers = three();
    let at = probe.app.line_count();
    a.drag_start(&offers, &[Action::Copy, Action::Move]);
    input.path(to.at(0.5, 0.5));
    let (_, entered) = probe.wait(at, "the drag entering", |v| v["ev"] == "drag-enter");
    assert_eq!(entered["mimes"], json!(mimes(&offers)), "{entered}");
    probe.wait(at, "the source's actions", |v| {
        v["ev"] == "drag-actions" && v["source"] == "[Copy, Move]"
    });
    probe.wait(at, "the drag moving", |v| v["ev"] == "drag-motion");
    a.wait_event("the window accepting Copy", |e| {
        matches!(
            e,
            Event::DragAction {
                action: Some(Action::Copy)
            }
        )
    });
    input.button(false);
    probe.wait(at, "the drop", |v| v["ev"] == "drop");
    for o in &offers {
        let (_, got) = probe.wait(at, &format!("{} read from the drop", o.mime), |v| {
            v["ev"] == "dropped" && v["mime"] == o.mime.as_str()
        });
        assert_eq!(got["fnv1a64"], fnv(o), "{got}");
    }
    probe.wait(at, "the drop finished", |v| v["ev"] == "drop-finished");
    a.wait_event("the drag finishing with Copy", |e| {
        matches!(
            e,
            Event::DragFinished {
                action: Some(Action::Copy)
            }
        )
    });
    probe.no_errors();
}

/// Shared.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_drops_three_types_on_a_wayland_window() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &[]);
    if !probe.shared() {
        return;
    }
    drag_into_probe(&s, &probe);
}

/// The review's sequence (round 2): the source offers [copy, ask], the probe prefers ask on
/// enter and answers the drop with move, which the source lacks. The compositor keeps ask, so
/// finishing would be a protocol error that ends winit's connection: the probe's finish must
/// be refused, and the app live on to quit cleanly. Shared.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_drop_left_on_ask_refuses_finish_and_the_app_lives_on() {
    let s = Session::start();
    let mut probe = Probe::start(&s, Display::Wayland, &["--answer-ask-with-move"]);
    if !probe.shared() {
        return;
    }
    let a = s.peer(Display::Wayland);
    a.map("p0 ask source");
    let windows = s.wait_windows(&[a.pid(), probe.app.pid()]);
    let from = windows
        .iter()
        .find(|w| w.pid == a.pid())
        .expect("a's window");
    let to = windows
        .iter()
        .find(|w| w.pid == probe.app.pid())
        .expect("the probe's window");
    a.sync();
    let input = s.input();
    s.point_at(&a, from.at(0.5, 0.5));
    input.button(true);
    a.wait_event("the press", |e| {
        matches!(e, Event::PointerButton { pressed: true })
    });
    let at = probe.app.line_count();
    a.drag_start(&three(), &[Action::Copy, Action::Ask]);
    input.path(to.at(0.5, 0.5));
    probe.wait(at, "the drag entering", |v| v["ev"] == "drag-enter");
    a.wait_event("the compositor choosing ask", |e| {
        matches!(
            e,
            Event::DragAction {
                action: Some(Action::Ask)
            }
        )
    });
    input.button(false);
    probe.wait(at, "the drop", |v| v["ev"] == "drop");
    // Move isn't the source's: after the drop that answer is refused, and ask stands.
    let (_, answered) = probe.wait(at, "the answer", |v| v["ev"] == "ask-answered");
    assert_eq!(answered["ok"], false, "{answered}");
    let (_, refused) = probe.wait(at, "the refused finish", |v| {
        v["ev"] == "drop-error" || v["ev"] == "drop-finished"
    });
    assert_eq!(
        refused["ev"], "drop-error",
        "finish wasn't refused: {refused}"
    );
    let why = refused["error"].as_str().unwrap_or_default();
    assert!(why.contains("still ask"), "{refused}");
    let at = probe.send(json!({ "cmd": "quit" }));
    probe.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
    let status = probe.app.wait_exit();
    assert!(
        status.success(),
        "the probe exited with {status}: winit's connection died?"
    );
    let failed = probe
        .app
        .lines()
        .iter()
        .filter(|l| l.contains("\"failed\""))
        .count();
    assert_eq!(failed, 0, "a connection failed");
}

/// Review round 3's repro: a copy drop, then set_actions(none) after it. wlroots would then
/// have no action, its answer can't reach a dropped offer (leave came with the drop), and a
/// finish would be a protocol error ending winit's connection. The request must be refused,
/// the drop finish as a copy, and the app quit cleanly. Shared.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn actions_set_after_a_drop_are_refused_and_the_drop_finishes() {
    let s = Session::start();
    let mut probe = Probe::start(&s, Display::Wayland, &["--clear-actions-after-drop"]);
    if !probe.shared() {
        return;
    }
    let a = s.peer(Display::Wayland);
    a.map("p0 copy source");
    let windows = s.wait_windows(&[a.pid(), probe.app.pid()]);
    let from = windows
        .iter()
        .find(|w| w.pid == a.pid())
        .expect("a's window");
    let to = windows
        .iter()
        .find(|w| w.pid == probe.app.pid())
        .expect("the probe's window");
    a.sync();
    let input = s.input();
    s.point_at(&a, from.at(0.5, 0.5));
    input.button(true);
    a.wait_event("the press", |e| {
        matches!(e, Event::PointerButton { pressed: true })
    });
    let at = probe.app.line_count();
    a.drag_start(&three(), &[Action::Copy, Action::Move]);
    input.path(to.at(0.5, 0.5));
    probe.wait(at, "the drag entering", |v| v["ev"] == "drag-enter");
    a.wait_event("the probe accepting copy", |e| {
        matches!(
            e,
            Event::DragAction {
                action: Some(Action::Copy)
            }
        )
    });
    input.button(false);
    let (_, cleared) = probe.wait(at, "set_actions after the drop", |v| {
        v["ev"] == "actions-cleared"
    });
    assert_eq!(cleared["ok"], false, "{cleared}");
    probe.wait(at, "the drop finished", |v| v["ev"] == "drop-finished");
    a.wait_event("the drag finishing with copy", |e| {
        matches!(
            e,
            Event::DragFinished {
                action: Some(Action::Copy)
            }
        )
    });
    let at = probe.send(json!({ "cmd": "quit" }));
    probe.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
    let status = probe.app.wait_exit();
    assert!(
        status.success(),
        "the probe exited with {status}: winit's connection died?"
    );
    probe.no_errors();
}

/// User and system CPU time of `pid`, in clock ticks.
fn cpu_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("the probe's stat");
    // The fields after the command's closing parenthesis; utime and stime are 14 and 15.
    let rest = stat.rsplit_once(')').map(|(_, r)| r).unwrap_or_default();
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let field = |n: usize| {
        fields
            .get(n - 3)
            .and_then(|f| f.parse::<u64>().ok())
            .unwrap_or(0)
    };
    field(14) + field(15)
}

/// Threads of `pid` named `name` (as `/proc` truncates names: 15 bytes).
fn threads_named(pid: u32, name: &str) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/task"))
        .map(|tasks| {
            tasks
                .flatten()
                .filter(|t| {
                    std::fs::read_to_string(t.path().join("comm"))
                        .is_ok_and(|c| c.trim_end() == name)
                })
                .count()
        })
        .unwrap_or(0)
}

/// The probe's stderr log, which a spinning reader would flood.
fn probe_stderr_len() -> u64 {
    let test = std::thread::current()
        .name()
        .unwrap_or_default()
        .replace("::", "-");
    let log = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("session")
        .join(test)
        .join("probe.stderr.log");
    std::fs::metadata(log).map(|m| m.len()).unwrap_or(0)
}

/// The review's B3 (round 2): the compositor ends the Wayland connection of our own with a
/// protocol error while events are queued on it. libwayland then dispatches nothing, and
/// wayland-client drops the error, so a reader that only re-prepared spun a core and the app
/// never heard. Exactly one `Failed` must arrive, the reader must exit, and the process go
/// quiet. Fails fast on a flood, so a regression can't fill the log.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_dead_wayland_connection_is_reported_once_and_goes_quiet() {
    let s = Session::start();
    let mut probe = Probe::start(&s, Display::Wayland, &["--own-data-control"]);
    let pid = probe.app.pid();
    assert_eq!(
        threads_named(pid, "transfer-wl-own"),
        1,
        "the own connection's reader"
    );
    // A source held on the connection of our own (data control goes there).
    let source = probe.own(
        Via::DataControl,
        Selection::Clipboard,
        &[Offer::text(UTF8, TEXT)],
    );
    let owns = |probe: &Probe| {
        let at = probe.send(json!({ "cmd": "owns", "source": source }));
        probe.wait(at, "owns", |v| v["ev"] == "owns").1["owns"].as_bool()
    };
    assert_eq!(owns(&probe), Some(true));
    let at = probe.send(json!({ "cmd": "break-own" }));
    let (_, broke) = probe.wait(at, "the protocol error asked for", |v| {
        v["ev"] == "broke-own"
    });
    assert_eq!(broke["ok"], true, "{broke}");
    let failures = || {
        probe.app.lines()[at..]
            .iter()
            .filter(|l| l.contains("\"failed\""))
            .count()
    };
    let deadline = std::time::Instant::now() + timeout();
    while failures() == 0 {
        assert!(probe_stderr_len() < 4 << 20, "the probe floods its stderr");
        assert!(std::time::Instant::now() < deadline, "no Failed event");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Settle, then the process must idle: a spinning reader takes a whole core.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let before = cpu_ticks(pid);
    std::thread::sleep(std::time::Duration::from_secs(1));
    let spent = cpu_ticks(pid) - before;
    assert!(
        spent < 50,
        "the probe used {spent} ticks in 1 s after the failure"
    );
    assert_eq!(failures(), 1, "Failed is reported once");
    assert_eq!(
        threads_named(pid, "transfer-wl-own"),
        0,
        "the reader exited"
    );
    assert!(probe_stderr_len() < 1 << 20, "the probe's stderr grew");
    // Nothing is held on a dead connection, and nothing more is taken there.
    assert_eq!(
        owns(&probe),
        Some(false),
        "a source still held on a dead connection"
    );
    let at = probe.send(json!({
        "cmd": "own", "via": Via::DataControl, "selection": Selection::Clipboard,
        "offers": [Offer::text(UTF8, TEXT)],
    }));
    let (_, refused) = probe.wait(at, "own", |v| v["ev"] == "owned" || v["ev"] == "own-error");
    assert_eq!(
        refused["ev"], "own-error",
        "taken on a dead connection: {refused}"
    );
    let at = probe.send(json!({ "cmd": "quit" }));
    probe.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
    let status = probe.app.wait_exit();
    assert!(status.success(), "the probe exited with {status}");
}

// ---- Wayland: lifecycle ----

/// S5's note on upstream #139: a covered Wayland window's `poll` and timers stop. The
/// substrate's events reach the app through `AppHandle::update_if`, which must still run with
/// the window on a workspace nobody sees. Shared where the host has it, else own.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_wayland_window_on_another_workspace_still_hears_the_clipboard() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::Wayland, &[]);
    s.sway_command(&format!(
        "[con_id={}] move container to workspace 9",
        probe.window.id
    ));
    s.wait_window_where("the probe on workspace 9", |w| {
        w.pid == probe.app.pid() && !w.focused
    });
    let peer = s.peer(Display::Wayland);
    let at = probe.app.line_count();
    peer.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[Offer::text(UTF8, TEXT)],
    );
    probe.wait(at, "the peer's selection while hidden", |v| {
        v["ev"] == "selection" && v["via"] == "data-control" && v["mimes"] == json!([UTF8])
    });
    let got = probe.read(Via::DataControl, Selection::Clipboard, UTF8);
    assert_eq!(got["text"], TEXT);
    probe.no_errors();
}

/// Shared: Wayland destroys a hidden window, and showing it makes a new surface. The
/// substrate stays bound once, and routes a drag to the new surface (it keeps the
/// `WindowHandle`, never the surface).
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_wayland_window_hidden_and_shown_keeps_one_binding() {
    let s = Session::start();
    let mut probe = Probe::start(&s, Display::Wayland, &[]);
    if !probe.shared() {
        return;
    }
    let pid = probe.app.pid();
    for round in 0..3 {
        probe.send(json!({ "cmd": "hide" }));
        let gone = || !has_pid(&s.tree(), pid);
        let deadline = std::time::Instant::now() + timeout();
        while !gone() {
            assert!(
                std::time::Instant::now() < deadline,
                "round {round}: the window never went"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let at = probe.send(json!({ "cmd": "show" }));
        let (_, again) = probe.wait(at, "attach on the new window", |v| v["ev"] == "attached");
        assert_eq!(again["fresh"], false, "round {round}: bound again: {again}");
        probe.window = s.wait_window(pid);
    }
    probe.wait_layout();
    drag_into_probe(&s, &probe);
}

/// Shared: the quit path tears the binding down before `run` returns, so the process exits
/// cleanly (a late drop crashed every S2 run with SIGSEGV).
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn quitting_tears_the_wayland_binding_down_before_run_returns() {
    let s = Session::start();
    let mut probe = Probe::start(&s, Display::Wayland, &[]);
    let shared = probe.shared();
    // Off the UI thread the shutdown is refused, and the substrate stays bound.
    let at = probe.send(json!({ "cmd": "shutdown-elsewhere" }));
    let (_, elsewhere) = probe.wait(at, "a shutdown off the UI thread", |v| {
        v["ev"] == "shutdown-elsewhere"
    });
    assert_eq!(
        (&elsewhere["refused"], &elsewhere["still_bound"]),
        (&json!(true), &json!(true)),
        "{elsewhere}"
    );
    let at = probe.send(json!({ "cmd": "quit" }));
    let (_, down) = probe.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
    assert_eq!(down["bound"], true, "{down}");
    probe.wait(at, "run returning", |v| {
        v["ev"] == "exit" && v["shut_down"] == true
    });
    let status = probe.app.wait_exit();
    assert!(
        status.success(),
        "the probe exited with {status} (shared: {shared})"
    );
}

/// Shared: closing the last window runs `on_close`, which tears the binding down.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn closing_the_last_window_tears_the_wayland_binding_down() {
    let s = Session::start();
    let mut probe = Probe::start(&s, Display::Wayland, &[]);
    let shared = probe.shared();
    let at = probe.app.line_count();
    s.sway_command(&format!("[con_id={}] kill", probe.window.id));
    let (_, down) = probe.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
    assert_eq!(down["why"], "close", "{down}");
    let status = probe.app.wait_exit();
    assert!(
        status.success(),
        "the probe exited with {status} (shared: {shared})"
    );
}

/// Shared. The only attached window closes, the reader then waits on a turn posted to its
/// app, which is gone, and a later window attaches as P1 and P2 attach a later window: no
/// dispatch, no round trip. Its attach must release the reader, or nothing on winit's
/// display is dispatched again (P4's review 2, should-fix B).
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_window_attached_after_the_only_attached_one_closed_hears_the_clipboard() {
    let s = Session::start();
    if !libwayland_has_get_display() {
        eprintln!("libwayland < 1.23 here: no winit's display, so no reader; nothing ran");
        return;
    }
    let mut cmd = s.command(example("transfer-probe"), Display::Wayland);
    cmd.args(["--title", "p0 host", "--host"]);
    let app = s.spawn("probe", cmd);
    app.wait_json("the host's first frame", |v| v["ev"] == "frame");
    let peer = s.peer(Display::Wayland);
    let open = |key: &str| {
        let at = app.line_count();
        app.send_line(&json!({ "cmd": "open-window", "key": key }).to_string());
        app.wait_json_from(at, &format!("{key}'s attach"), |v| {
            (v["ev"] == "attached" && v["key"] == key) || v["ev"] == "attach-error"
        })
        .1
    };
    let offer = |name: &str| {
        let mime = format!("text/x-p0-{name}");
        let at = app.line_count();
        peer.set_selection(
            Selection::Clipboard,
            Via::DataControl,
            &[Offer::text(&mime, name)],
        );
        (at, mime)
    };
    let heard = |v: &Value, mime: &str| {
        v["ev"] == "selection"
            && v["mimes"]
                .as_array()
                .is_some_and(|m| m.iter().any(|x| x == mime))
    };

    let first = open("w1");
    assert_eq!(
        (&first["fresh"], &first["shared"]),
        (&json!(true), &json!(true)),
        "{first}"
    );
    let (at, mime) = offer("one");
    app.wait_json_from(at, "the peer's first selection", |v| heard(v, &mime));

    let at = app.line_count();
    app.send_line(&json!({ "cmd": "close-window", "key": "w1" }).to_string());
    app.wait_json_from(at, "w1's on_close", |v| v["ev"] == "window-closed");
    let deadline = std::time::Instant::now() + timeout();
    while has_title(&s.tree(), "p0 w1") {
        assert!(std::time::Instant::now() < deadline, "w1 never went");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // The reader reads it, asks w1's app, and waits: with no live window attached, nothing
    // dispatches it. Checked, or this test would prove nothing.
    let (at, two) = offer("two");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        !app.lines()[at..]
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .any(|v| heard(&v, &two)),
        "heard with no live window attached: the reader wasn't waiting"
    );

    let second = open("w2");
    assert_eq!(second["fresh"], false, "{second}");
    app.wait_json_from(at, "the selection queued while no window lived", |v| {
        heard(v, &two)
    });
    let (at, mime) = offer("three");
    app.wait_json_from(at, "a selection after w2 attached", |v| heard(v, &mime));
    let at = app.line_count();
    app.send_line(&json!({ "cmd": "quit" }).to_string());
    app.wait_json_from(at, "the shutdown", |v| v["ev"] == "shutdown");
    let mut app = app;
    assert!(app.wait_exit().success());
}

/// Whether any node of sway's tree is named `title`.
fn has_title(node: &Value, title: &str) -> bool {
    match node {
        Value::Object(map) => {
            map.get("name").and_then(Value::as_str) == Some(title)
                || map.values().any(|v| has_title(v, title))
        }
        Value::Array(items) => items.iter().any(|v| has_title(v, title)),
        _ => false,
    }
}

/// Whether any node of sway's tree belongs to `pid`.
fn has_pid(node: &Value, pid: u32) -> bool {
    match node {
        Value::Object(map) => {
            map.get("pid").and_then(Value::as_u64) == Some(u64::from(pid))
                || map.values().any(|v| has_pid(v, pid))
        }
        Value::Array(items) => items.iter().any(|v| has_pid(v, pid)),
        _ => false,
    }
}

// ---- X11 ----

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_window_offers_three_types_on_the_clipboard() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::X11, &[]);
    assert_eq!(probe.attached["backend"], "x11", "{}", probe.attached);
    let peer = s.peer(Display::X11);
    window_offers_three(&probe, &peer, Via::X11, Via::X11, Selection::Clipboard);
    let one = [Offer::text(UTF8, "primary from P0")];
    probe.own(Via::X11, Selection::Primary, &one);
    wait_targets(&peer, Selection::Primary, Via::X11, &mimes(&one));
    let got = peer.read(Selection::Primary, Via::X11, UTF8);
    assert_eq!(got.text.as_deref(), Some("primary from P0"));
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn peer_offers_three_types_to_an_x11_window() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::X11, &[]);
    let peer = s.peer(Display::X11);
    let at = probe.app.line_count();
    peer.set_selection(Selection::Clipboard, Via::X11, &three());
    peer_offers_three(&probe, at, Via::X11, Selection::Clipboard);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn an_x11_window_hears_it_lost_the_clipboard() {
    let s = Session::start();
    let probe = Probe::start(&s, Display::X11, &[]);
    let peer = s.peer(Display::X11);
    let at = probe.app.line_count();
    let source = probe.own(Via::X11, Selection::Clipboard, &[Offer::text(UTF8, TEXT)]);
    wait_targets(&peer, Selection::Clipboard, Via::X11, &[UTF8.into()]);
    peer.set_selection(
        Selection::Clipboard,
        Via::X11,
        &[Offer::text(UTF8, "the peer's")],
    );
    probe.wait(at, "losing the clipboard", |v| {
        v["ev"] == "cancelled" && v["source"] == source
    });
    probe.wait(at, "the peer's selection", |v| {
        v["ev"] == "selection"
            && v["via"] == "x11"
            && v["mimes"]
                .as_array()
                .is_some_and(|m| m.iter().any(|x| x == UTF8))
    });
    let got = probe.read(Via::X11, Selection::Clipboard, UTF8);
    assert_eq!(got["text"], "the peer's");
    probe.no_errors();
}

/// The X server ends our X11 connection (the probe's test switch): one Failed, no source
/// held any more (`owns` false), and no more taken; the app still quits cleanly.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_failed_x11_connection_holds_and_takes_nothing() {
    let s = Session::start();
    let mut probe = Probe::start(&s, Display::X11, &[]);
    let source = probe.own(Via::X11, Selection::Clipboard, &[Offer::text(UTF8, TEXT)]);
    let owns = |probe: &Probe| {
        let at = probe.send(json!({ "cmd": "owns", "source": source }));
        probe.wait(at, "owns", |v| v["ev"] == "owns").1["owns"].as_bool()
    };
    // The take completes on the X11 thread.
    let deadline = std::time::Instant::now() + timeout();
    while owns(&probe) != Some(true) {
        assert!(
            std::time::Instant::now() < deadline,
            "the selection was never held"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let at = probe.send(json!({ "cmd": "break-x11" }));
    let (_, broke) = probe.wait(at, "the kill asked for", |v| v["ev"] == "broke-x11");
    assert_eq!(broke["ok"], true, "{broke}");
    probe.wait(at, "the failure", |v| v["ev"] == "failed");
    assert_eq!(
        owns(&probe),
        Some(false),
        "a source still held on a dead connection"
    );
    let at = probe.send(json!({
        "cmd": "own", "via": Via::X11, "selection": Selection::Clipboard,
        "offers": [Offer::text(UTF8, TEXT)],
    }));
    let (_, refused) = probe.wait(at, "own", |v| v["ev"] == "owned" || v["ev"] == "own-error");
    assert_eq!(
        refused["ev"], "own-error",
        "taken on a dead connection: {refused}"
    );
    let failed = probe
        .app
        .lines()
        .iter()
        .filter(|l| l.contains("\"failed\""))
        .count();
    assert_eq!(failed, 1, "Failed is reported once");
    let at = probe.send(json!({ "cmd": "quit" }));
    probe.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
    let status = probe.app.wait_exit();
    assert!(status.success(), "the probe exited with {status}");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn quitting_tears_the_x11_transfer_down() {
    let s = Session::start();
    let mut probe = Probe::start(&s, Display::X11, &[]);
    let at = probe.send(json!({ "cmd": "quit" }));
    let (_, down) = probe.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
    assert_eq!(down["bound"], true, "{down}");
    let status = probe.app.wait_exit();
    assert!(status.success(), "the probe exited with {status}");
}

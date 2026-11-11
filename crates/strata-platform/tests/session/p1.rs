//! P1's session tests: drags out to the peer and drops in from it, on Wayland and X11, with
//! the `dnd` example (`examples/dnd.rs`) as the app.
//!
//! Work package: P1, `docs/work/phase-2-composites/P1-drag-and-drop.md`.
//!
//! What runs here, per backend: a drag out with three MIME types that the peer reads; drops
//! in of `text/uri-list`, `text/plain`, `text/x-moz-url`, `image/png` and HTML; the action
//! with each modifier, and an ask; cancelled drags; an in-process drag between two windows
//! with a typed payload. On Wayland also: Rune's capture ends when the drag starts, not at the
//! drop. Hand tests (Nautilus, Dolphin, Hyprland, GNOME, KDE) are pending, not run here.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use strata_test_peer::protocol::{Action, Event, Offer, Payload, fnv1a64};

use crate::support::{Display, Peer, Proc, Session, Window, example, timeout};

const URI_LIST: &str = "text/uri-list";
const TEXT: &str = "text/plain;charset=utf-8";
const MOZ_URL: &str = "text/x-moz-url";
const PNG: &str = "image/png";
const HTML: &str = "text/html";

fn fnv(offer: &Offer) -> String {
    fnv1a64(&offer.data.bytes().expect("offer bytes"))
}

fn mimes(offers: &[Offer]) -> Vec<&str> {
    offers.iter().map(|o| o.mime.as_str()).collect()
}

/// The app, and its windows as sway has them.
struct Demo {
    app: Proc,
    title: String,
    keys: Vec<&'static str>,
}

impl Demo {
    fn start(s: &Session, display: Display, two: bool) -> Demo {
        let title = "p1 dnd".to_owned();
        let mut cmd = s.command(example("dnd"), display);
        cmd.args(["--title", &title]);
        if two {
            cmd.arg("--two-windows");
        }
        let app = s.spawn("dnd", cmd);
        let keys = if two {
            vec!["main", "second"]
        } else {
            vec!["main"]
        };
        let demo = Demo { app, title, keys };
        for k in &demo.keys {
            let (_, v) = demo.wait(0, &format!("{k} attaching"), |v| {
                (v["ev"] == "attached" || v["ev"] == "attach-error") && v["window"] == *k
            });
            assert_eq!(v["ev"], "attached", "{v}");
            assert_eq!(v["drag_in"], true, "{v}");
            assert_eq!(v["drag_out"], true, "{v}");
        }
        demo.wait(0, "the first frame", |v| v["ev"] == "frame");
        demo
    }

    fn title_of(&self, key: &str) -> String {
        if key == "main" {
            self.title.clone()
        } else {
            format!("{} {key}", self.title)
        }
    }

    fn window(&self, s: &Session, key: &str) -> Window {
        let (title, pid) = (self.title_of(key), self.app.pid());
        s.wait_window_where(&format!("the {key} window"), |w| {
            w.pid == pid && w.title == title && w.content.width > 0.0
        })
    }

    fn send(&self, cmd: Value) -> usize {
        let mark = self.app.line_count();
        self.app.send_line(&cmd.to_string());
        mark
    }

    /// Sends `cmd` and waits for its acknowledgement line `ack`, from each window it reaches.
    fn command(&self, cmd: Value, ack: &str) {
        let only = cmd["window"].as_str().map(str::to_owned);
        let at = self.send(cmd);
        for k in &self.keys {
            if only.as_ref().is_some_and(|o| o != k) {
                continue;
            }
            self.wait(at, &format!("{ack} from {k}"), |v| {
                v["ev"] == ack && v["window"] == *k
            });
        }
    }

    fn wait(&self, from: usize, what: &str, pred: impl Fn(&Value) -> bool) -> (usize, Value) {
        self.app.wait_json_from(from, what, pred)
    }

    fn lines(&self) -> Vec<Value> {
        self.app
            .lines()
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .collect()
    }

    /// The newest rect of each keyed element of window `key`, in window points.
    fn rects(&self, key: &str) -> HashMap<String, [f64; 4]> {
        let mut out = HashMap::new();
        for v in self.lines() {
            if v["ev"] == "rect" && v["window"] == key {
                let r: Vec<f64> = v["r"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                if let (Some(k), [x, y, w, h]) = (v["key"].as_str(), r.as_slice()) {
                    out.insert(k.to_owned(), [*x, *y, *w, *h]);
                }
            }
        }
        out
    }

    /// Waits until window `key` has laid out at the size sway gave it: its zone spans it.
    fn settle(&self, s: &Session, key: &str) -> Window {
        let deadline = Instant::now() + timeout();
        loop {
            let w = self.window(s, key);
            let r = self.rects(key);
            let laid = ["tile", "manual", "plain", "zone"]
                .iter()
                .all(|k| r.contains_key(*k));
            if laid && (r["zone"][2] - w.content.width).abs() < 1.0 {
                return w;
            }
            assert!(
                Instant::now() < deadline,
                "window {key} never laid out at {:?}: {r:?}",
                w.content
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A layout point inside element `el` of window `key`.
    fn at(&self, s: &Session, key: &str, el: &str, fx: f64, fy: f64) -> (f64, f64) {
        let w = self.settle(s, key);
        let r = self.rects(key)[el];
        (
            w.content.x + r[0] + r[2] * fx,
            w.content.y + r[1] + r[3] * fy,
        )
    }

    /// Gives window `key` keyboard focus through sway and waits until the app has heard it
    /// settle. On Xwayland, the click that focuses an unfocused window brings focus in, out
    /// and in again (W3's `harness-window` shows it too, with no shim), and Rune cancels a
    /// press that lands inside it: a drag from that press never starts. So the window is
    /// focused before the press, not by it.
    fn focus(&self, s: &Session, key: &str) {
        let w = self.window(s, key);
        if !w.focused {
            s.focus(&w);
        }
        let last_focus = || {
            self.lines()
                .iter()
                .enumerate()
                .rev()
                .find(|(_, v)| v["ev"] == "focus" && v["window"] == key)
                .map(|(i, v)| (i, v["focused"] == true))
        };
        let deadline = Instant::now() + timeout();
        loop {
            let before = last_focus();
            std::thread::sleep(Duration::from_millis(250));
            let after = last_focus();
            if before == after && after.is_some_and(|(_, f)| f) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "window {key} never settled focused: {after:?}"
            );
        }
    }

    /// Moves the pointer onto element `el` until the app has the pointer there: `plain` logs
    /// its hover, so the pointer goes there first.
    fn point(&self, s: &Session, key: &str, el: &str) -> (f64, f64) {
        let plain = self.at(s, key, "plain", 0.5, 0.5);
        let mark = self.app.line_count();
        s.point_until(plain, || {
            self.lines()
                .iter()
                .skip(mark)
                .any(|v| v["ev"] == "plain-hover" && v["over"] == true && v["window"] == key)
        });
        let to = self.at(s, key, el, 0.5, 0.5);
        s.input().pointer_move(to);
        to
    }

    fn offer(&self, offers: &[Offer], actions: &[&str], local: Option<&str>, icon: Option<&str>) {
        self.command(
            json!({ "cmd": "offer", "offers": offers, "actions": actions, "local": local, "icon": icon }),
            "offered",
        );
    }

    fn zone(&self, window: Option<&str>, zone: Value) {
        let mut cmd = zone;
        cmd["cmd"] = json!("zone");
        if let Some(w) = window {
            cmd["window"] = json!(w);
        }
        self.command(cmd, "zoned");
    }

    fn no_errors(&self) {
        let bad: Vec<String> = self
            .app
            .lines()
            .into_iter()
            .filter(|l| {
                [
                    "diagnostic",
                    "drag-error",
                    "attach-error",
                    "exit-error",
                    "bad-command",
                ]
                .iter()
                .any(|e| l.contains(&format!("\"{e}\"")))
            })
            .collect();
        assert!(bad.is_empty(), "the app reported errors: {bad:?}");
    }

    fn quit(mut self) {
        let at = self.send(json!({ "cmd": "quit", "window": "main" }));
        self.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
        let status = self.app.wait_exit();
        assert!(status.success(), "the app exited with {status}");
        self.no_errors();
    }
}

/// A peer with its window mapped, tiled beside the app's.
fn peer_beside(s: &Session, display: Display, demo: &Demo) -> (Peer, Window) {
    let peer = s.peer(display);
    peer.map("p1 peer");
    let windows = s.wait_windows(&[peer.pid(), demo.app.pid()]);
    let w = windows
        .iter()
        .find(|w| w.pid == peer.pid())
        .expect("the peer's window")
        .clone();
    peer.sync();
    (peer, w)
}

/// Presses on `el` of the app's main window, focused first, and waits until the app has
/// the press.
fn press_on(s: &Session, demo: &Demo, el: &str) -> usize {
    demo.focus(s, "main");
    demo.point(s, "main", el);
    let mark = demo.app.line_count();
    s.input().button(true);
    let phase = if el == "manual" {
        "manual-phase"
    } else {
        "tile-phase"
    };
    demo.wait(mark, "the press", |v| {
        v["ev"] == phase && v["phase"] == "Down"
    });
    mark
}

/// Presses on the peer's window, focused first through sway (see [`Demo::focus`]: a click
/// that focuses an unfocused Xwayland window can lose its press), and waits until the peer
/// has the press.
fn press_on_peer(s: &Session, peer: &Peer, window: &Window) {
    let w = s.wait_window_where("the peer's window", |w| w.id == window.id);
    if !w.focused {
        s.focus(&w);
    }
    // Settled: sway has it focused, and the peer heard no focus change for a while.
    let deadline = Instant::now() + timeout();
    loop {
        let mark = peer.line_count();
        std::thread::sleep(Duration::from_millis(250));
        peer.sync();
        let quiet = !peer.saw_since(mark, Duration::ZERO, |e| matches!(e, Event::Focus { .. }));
        let focused = s
            .wait_window_where("the peer's window", |w| w.id == window.id)
            .focused;
        if focused && quiet {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the peer's window never settled focused"
        );
    }
    s.point_at(peer, window.at(0.5, 0.5));
    s.input().button(true);
    wait_peer(peer, "the press", |e| {
        matches!(e, Event::PointerButton { pressed: true })
    });
}

fn wait_peer(peer: &Peer, what: &str, pred: impl Fn(&Event) -> bool) -> Event {
    peer.wait_event(what, pred)
}

fn three() -> Vec<Offer> {
    vec![
        Offer::text(
            URI_LIST,
            "file:///tmp/p1/alpha.txt\r\nfile:///tmp/p1/b%20c.txt\r\n",
        ),
        Offer::text(TEXT, "hello from p1"),
        Offer {
            mime: PNG.into(),
            data: Payload::Pattern {
                len: 1 << 20,
                seed: 7,
            },
        },
    ]
}

/// The five types content drops take, from the peer. `text/x-moz-url` is UTF-16, as
/// Firefox sends it, so it goes in a file.
fn five(s: &Session) -> Vec<Offer> {
    let moz: Vec<u8> = "https://example.com/\nExample"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let path = s.runtime_dir().join("moz-url");
    std::fs::write(&path, moz).expect("the x-moz-url file");
    vec![
        Offer::text(URI_LIST, "file:///tmp/p1/alpha.txt\r\n"),
        Offer::text(TEXT, "dropped text"),
        Offer {
            mime: MOZ_URL.into(),
            data: Payload::File(path),
        },
        Offer {
            mime: PNG.into(),
            data: Payload::Pattern {
                len: 64 << 10,
                seed: 3,
            },
        },
        Offer::text(HTML, "<a href=\"https://example.com/\">Example</a>"),
    ]
}

// ---- drag out ----

fn drag_out_three_types(display: Display) {
    let s = Session::start();
    let demo = Demo::start(&s, display, false);
    let (peer, to) = peer_beside(&s, display, &demo);
    let offers = three();
    demo.offer(&offers, &["copy", "move"], None, Some("snapshot"));
    peer.accept_drops_with(
        &mimes(&offers),
        &[Action::Copy, Action::Move],
        Action::Copy,
        true,
        None,
    );
    let mark = press_on(&s, &demo, "tile");
    s.input().path(to.at(0.5, 0.5));
    demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
    let entered = wait_peer(&peer, "the drag entering the peer", |e| {
        matches!(e, Event::DropEnter { .. })
    });
    // The in-process token: on X11 only with a payload (other clients see it); on Wayland on
    // every drag, where it marks the drag as ours when it enters our windows.
    let Event::DropEnter { mimes: listed, .. } = entered else {
        unreachable!()
    };
    let token = |m: &String| m.starts_with("application/x-local-drag;id=");
    let others: Vec<&String> = listed.iter().filter(|m| !token(m)).collect();
    assert_eq!(others, mimes(&offers), "{listed:?}");
    let tokens = listed.iter().filter(|m| token(m)).count();
    match display {
        Display::X11 => assert_eq!(tokens, 0, "no payload, no token on X11: {listed:?}"),
        _ => assert_eq!(tokens, 1, "{listed:?}"),
    }
    let peer_mark = peer.line_count();
    s.input().button(false);
    // The peer reads them all at once, and on X11 they finish in any order.
    for o in &offers {
        let got = peer.event_since(
            peer_mark,
            &format!("{} dropped", o.mime),
            |e| matches!(e, Event::Dropped { mime, .. } if *mime == o.mime),
        );
        let Event::Dropped { data, action, .. } = got else {
            unreachable!()
        };
        assert_eq!(data.fnv1a64, fnv(o), "{}", o.mime);
        assert_eq!(action, Some(Action::Copy));
    }
    let (_, end) = demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
    assert_eq!(end["outcome"], "copied", "{end}");
    // Each lazy writer ran once, though the peer read each type once only.
    for o in &offers {
        let runs = demo
            .lines()
            .iter()
            .filter(|v| v["ev"] == "writer" && v["mime"] == o.mime.as_str())
            .count();
        assert_eq!(runs, 1, "{}'s writer", o.mime);
    }
    // Nothing of the drag reaches the tile's own handler after it started.
    let after_start: Vec<Value> = demo
        .lines()
        .into_iter()
        .skip_while(|v| v["ev"] != "drag-start")
        .filter(|v| v["ev"] == "tile-phase")
        .collect();
    assert!(after_start.is_empty(), "{after_start:?}");
    demo.quit();
}

/// operations/drag-and-drop: "Dragged items are offered as … `text/uri-list`, so other
/// applications can receive them" (the platform half: any MIME types, each read intact).
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_drag_out_offers_three_types_the_peer_reads() {
    drag_out_three_types(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_drag_out_offers_three_types_the_peer_reads() {
    drag_out_three_types(Display::X11);
}

/// Rune's capture (U10, draft): on Wayland the drag's start ends it. The cancelled Up that
/// P1's Escape brings arrives while the button is still held, so it isn't the compositor's
/// focus bounce at the drop; the Escape reaches no key handler; and after the drop the
/// pointer hovers again with no move reaching the element.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_runes_capture_ends_when_the_drag_starts() {
    let s = Session::start();
    let demo = Demo::start(&s, Display::Wayland, false);
    let (peer, to) = peer_beside(&s, Display::Wayland, &demo);
    demo.offer(&[Offer::text(TEXT, "capture")], &["copy"], None, None);
    peer.accept_drops(&[TEXT], &[Action::Copy], Action::Copy);
    let mark = press_on(&s, &demo, "manual");
    s.input().path(to.at(0.5, 0.5));
    demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
    let (up_at, up) = demo.wait(mark, "the capture ending, button held", |v| {
        v["ev"] == "manual-phase" && v["phase"] == "Up"
    });
    assert_eq!(up["cancelled"], true, "{up}");
    assert_eq!(up["owned"], true, "{up}");
    s.input().button(false);
    wait_peer(&peer, "the drop", |e| matches!(e, Event::Dropped { .. }));
    demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
    // Back over the window, no button held.
    let hover_mark = demo.app.line_count();
    let plain = demo.at(&s, "main", "plain", 0.5, 0.5);
    s.point_until(plain, || {
        demo.lines()
            .iter()
            .skip(hover_mark)
            .any(|v| v["ev"] == "plain-hover" && v["over"] == true)
    });
    let manual = demo.at(&s, "main", "manual", 0.5, 0.5);
    s.input().path(manual);
    s.input().path(plain);
    let lines = demo.lines();
    let after: Vec<&Value> = lines
        .iter()
        .skip(up_at + 1)
        .filter(|v| v["ev"] == "manual-phase")
        .collect();
    assert!(
        after.is_empty(),
        "moves reached the element after the drag: {after:?}"
    );
    let escapes: Vec<&Value> = lines
        .iter()
        .filter(|v| v["ev"] == "key" && v["key"] == "Escape")
        .collect();
    assert!(
        escapes.is_empty(),
        "the synthetic Escape leaked: {escapes:?}"
    );
    demo.quit();
}

// ---- drop in ----

fn drop_in_five_types(display: Display) {
    let s = Session::start();
    let demo = Demo::start(&s, display, false);
    let (peer, from) = peer_beside(&s, display, &demo);
    let offers = five(&s);
    demo.zone(None, json!({ "mimes": mimes(&offers), "read_all": true }));
    press_on_peer(&s, &peer, &from);
    let mark = demo.app.line_count();
    peer.drag_start(&offers, &[Action::Copy, Action::Move]);
    let zone_at = demo.at(&s, "main", "zone", 0.5, 0.5);
    s.input().path(zone_at);
    let (_, hover) = demo.wait(mark, "the zone accepting", |v| {
        v["ev"] == "hover"
            && v["state"]
                .as_str()
                .is_some_and(|s| s.starts_with("accepting"))
    });
    assert_eq!(hover["mimes"], json!(mimes(&offers)), "{hover}");
    // Every motion, in the zone's own points.
    let (_, mv) = demo.wait(mark, "a motion", |v| v["ev"] == "move");
    let zone = demo.rects("main")["zone"];
    let (x, wx) = (
        mv["x"].as_f64().unwrap_or(-1.0),
        mv["wx"].as_f64().unwrap_or(-2.0),
    );
    assert!(
        (x - (wx - zone[0])).abs() < 0.01,
        "{mv} with the zone at {zone:?}"
    );
    s.input().button(false);
    let (_, dropped) = demo.wait(mark, "the drop", |v| v["ev"] == "drop");
    for o in &offers {
        let item = dropped["items"]
            .as_array()
            .and_then(|a| a.iter().find(|i| i["mime"] == o.mime.as_str()))
            .unwrap_or_else(|| panic!("{} not read: {dropped}", o.mime));
        assert_eq!(item["fnv1a64"], fnv(o), "{}", o.mime);
    }
    assert_eq!(dropped["failed"], json!([]), "{dropped}");
    assert_eq!(dropped["action"], "copy", "{dropped}");
    demo.wait(mark, "the hover ending", |v| {
        v["ev"] == "hover" && v["state"] == "left"
    });
    let fin = wait_peer(&peer, "the drag finishing", |e| {
        matches!(e, Event::DragFinished { .. })
    });
    assert_eq!(
        fin,
        Event::DragFinished {
            action: Some(Action::Copy)
        }
    );
    demo.quit();
}

/// operations/drag-and-drop/content-drops: the platform half, every type read intact.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_drop_in_reads_five_types() {
    drop_in_five_types(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_drop_in_reads_five_types() {
    drop_in_five_types(Display::X11);
}

// ---- actions ----

/// A drag out with `held` modifier held from before the press; the outcome.
fn drag_out_with(
    s: &Session,
    demo: &Demo,
    to: &Window,
    held: Option<&str>,
    mid: Option<&str>,
) -> String {
    if let Some(m) = held {
        s.input().key_down(m);
    }
    let mark = press_on(s, demo, "tile");
    let half = (
        (to.at(0.5, 0.5).0 + demo.at(s, "main", "tile", 0.5, 0.5).0) / 2.0,
        to.at(0.5, 0.5).1,
    );
    s.input().path(half);
    demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
    if let Some(m) = mid {
        s.input().key_down(m);
    }
    s.input().path(to.at(0.5, 0.5));
    // Let the peer hear a position with the modifiers as they are now.
    std::thread::sleep(Duration::from_millis(150));
    s.input().button(false);
    for m in held.into_iter().chain(mid) {
        s.input().key_up(m);
    }
    let (_, end) = demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
    end["outcome"].as_str().unwrap_or_default().to_owned()
}

/// operations/drag-and-drop/copy-or-move: "Holding Ctrl at the drop copies and holding
/// Shift moves" (the source's half). On Wayland the modifiers at the start narrow what the
/// source offers; on X11 the proposed action follows them, through the drag.
fn actions_follow_modifiers_on_drag_out(display: Display) {
    let s = Session::start();
    let demo = Demo::start(&s, display, false);
    let (peer, to) = peer_beside(&s, display, &demo);
    demo.offer(
        &[Offer::text(TEXT, "act")],
        &["copy", "move"],
        None,
        Some("image"),
    );
    // The peer prefers move: a plain drag shows what the source lets it take.
    peer.accept_drops(&[TEXT], &[Action::Copy, Action::Move], Action::Move);
    let plain = drag_out_with(&s, &demo, &to, None, None);
    let ctrl = drag_out_with(&s, &demo, &to, Some("ctrl"), None);
    let shift = drag_out_with(&s, &demo, &to, Some("shift"), None);
    match display {
        Display::Wayland => {
            assert_eq!(
                plain, "moved",
                "copy and move offered: the target's preference"
            );
            assert_eq!(ctrl, "copied", "Ctrl offers copy only");
            assert_eq!(shift, "moved", "Shift offers move only");
        }
        _ => {
            assert_eq!(
                plain, "copied",
                "a plain drag proposes copy, which the peer takes"
            );
            // Under GTK's rule Ctrl alone proposes copy, as a plain drag does when copy is
            // allowed, so no outcome tells them apart on X11: Ctrl+Shift below can fail.
            assert_eq!(ctrl, "copied");
            assert_eq!(shift, "moved", "Shift proposes move");
            let late = drag_out_with(&s, &demo, &to, None, Some("shift"));
            assert_eq!(late, "moved", "Shift pressed during the drag proposes move");
            // Link, X11's only: Ctrl+Shift.
            demo.offer(
                &[Offer::text(TEXT, "link")],
                &["copy", "move", "link"],
                None,
                None,
            );
            peer.accept_drops(
                &[TEXT],
                &[Action::Copy, Action::Move, Action::Link],
                Action::Move,
            );
            let both = drag_out_with(&s, &demo, &to, Some("ctrl+shift"), None);
            assert_eq!(both, "linked", "Ctrl+Shift proposes link");
        }
    }
    demo.quit();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_drag_out_actions_follow_the_modifiers_at_the_start() {
    actions_follow_modifiers_on_drag_out(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_drag_out_actions_follow_the_modifiers() {
    actions_follow_modifiers_on_drag_out(Display::X11);
}

/// A peer drag over the zone, dropped, with `held` held from before it reaches the zone.
/// The app's drop line and the peer's finish.
fn drop_in_with(
    s: &Session,
    demo: &Demo,
    peer: &Peer,
    from: &Window,
    actions: &[Action],
    held: Option<&str>,
) -> (Value, Event) {
    press_on_peer(s, peer, from);
    let mark = demo.app.line_count();
    peer.drag_start(&[Offer::text(URI_LIST, "file:///tmp/p1/x\r\n")], actions);
    if let Some(m) = held {
        s.input().key_down(m);
    }
    s.input().path(demo.at(s, "main", "zone", 0.5, 0.5));
    demo.wait(mark, "the zone hovered", |v| {
        v["ev"] == "hover" && v["state"] != "left"
    });
    std::thread::sleep(Duration::from_millis(150));
    s.input().button(false);
    if let Some(m) = held {
        s.input().key_up(m);
    }
    let (_, dropped) = demo.wait(mark, "the drop", |v| v["ev"] == "drop");
    let fin = wait_peer(peer, "the drag finishing", |e| {
        matches!(e, Event::DragFinished { .. })
    });
    (dropped, fin)
}

/// The target's half of the action: Shift on X11 (known there); on Wayland a drag from
/// another client brings no modifiers, so the compositor's action stands (the owner's
/// ruling: a known limit); and an ask, answered after the drop.
fn actions_on_drop_in(display: Display) {
    let s = Session::start();
    let demo = Demo::start(&s, display, false);
    let (peer, from) = peer_beside(&s, display, &demo);
    demo.zone(None, json!({ "mimes": [URI_LIST] }));
    let (dropped, fin) = drop_in_with(
        &s,
        &demo,
        &peer,
        &from,
        &[Action::Copy, Action::Move],
        Some("shift"),
    );
    match display {
        Display::X11 => {
            assert_eq!(dropped["action"], "move", "{dropped}");
            assert_eq!(dropped["mods"]["shift"], true, "{dropped}");
            assert_eq!(
                fin,
                Event::DragFinished {
                    action: Some(Action::Move)
                }
            );
            // Ctrl: copy, which a plain drop takes too, so the modifiers it reports are what
            // can fail here.
            let (dropped, fin) = drop_in_with(
                &s,
                &demo,
                &peer,
                &from,
                &[Action::Move, Action::Copy],
                Some("ctrl"),
            );
            assert_eq!(dropped["mods"]["ctrl"], true, "{dropped}");
            assert_eq!(dropped["mods"]["shift"], false, "{dropped}");
            assert_eq!(dropped["action"], "copy", "{dropped}");
            assert_eq!(
                fin,
                Event::DragFinished {
                    action: Some(Action::Copy)
                }
            );
            // Ctrl+Shift: link, X11's only.
            let (dropped, fin) = drop_in_with(
                &s,
                &demo,
                &peer,
                &from,
                &[Action::Copy, Action::Move, Action::Link],
                Some("ctrl+shift"),
            );
            assert_eq!(dropped["action"], "link", "{dropped}");
            assert_eq!(
                fin,
                Event::DragFinished {
                    action: Some(Action::Link)
                }
            );
        }
        _ => {
            assert_eq!(
                dropped["mods"],
                Value::Null,
                "no modifiers from another client: {dropped}"
            );
            assert_eq!(dropped["action"], "copy", "{dropped}");
            assert_eq!(
                fin,
                Event::DragFinished {
                    action: Some(Action::Copy)
                }
            );
        }
    }
    // An ask, answered with move after the drop (on Wayland, S1's confirmation first).
    demo.zone(
        None,
        json!({ "mimes": [URI_LIST], "action": "ask", "ask_answer": "move" }),
    );
    let mark = demo.app.line_count();
    let (dropped, fin) = drop_in_with(
        &s,
        &demo,
        &peer,
        &from,
        &[Action::Copy, Action::Move, Action::Ask],
        None,
    );
    demo.wait(mark, "the ask", |v| v["ev"] == "ask");
    assert_eq!(dropped["action"], "move", "{dropped}");
    assert_eq!(
        fin,
        Event::DragFinished {
            action: Some(Action::Move)
        }
    );
    demo.quit();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_drop_in_takes_the_compositors_action_and_answers_an_ask() {
    actions_on_drop_in(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_drop_in_action_follows_shift_and_answers_an_ask() {
    actions_on_drop_in(Display::X11);
}

// ---- cancel ----

/// operations/drag-and-drop: "Releasing outside every drop target ends the drag without
/// transferring"; a drag cancelled by the app; on X11 by Escape; and a drag in that leaves.
fn cancelled_drags(display: Display) {
    let s = Session::start();
    let demo = Demo::start(&s, display, false);
    let (peer, to) = peer_beside(&s, display, &demo);
    // Over nothing that takes it: the zone reads uri-lists only.
    demo.offer(&[Offer::text(TEXT, "nowhere")], &["copy"], None, None);
    peer.accept_drops(&[TEXT], &[Action::Copy], Action::Copy);
    let mark = press_on(&s, &demo, "tile");
    s.input().path(demo.at(&s, "main", "zone", 0.5, 0.5));
    demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
    s.input().button(false);
    let (_, end) = demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
    assert_eq!(end["outcome"], "cancelled", "{end}");
    // Cancelled by the app, over a peer that would take it.
    let mark = press_on(&s, &demo, "tile");
    s.input().path(to.at(0.5, 0.5));
    demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
    wait_peer(&peer, "the drag over the peer", |e| {
        matches!(e, Event::DropEnter { .. })
    });
    let peer_mark = peer.line_count();
    demo.command(json!({ "cmd": "cancel", "window": "main" }), "cancel-sent");
    let (_, end) = demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
    assert_eq!(end["outcome"], "cancelled", "{end}");
    s.input().button(false);
    assert!(
        !peer.saw_since(peer_mark, Duration::from_millis(500), |e| {
            matches!(e, Event::Dropped { .. })
        }),
        "a cancelled drag dropped"
    );
    if display == Display::X11 {
        // Escape reaches Rune's window (it has focus; the X11 source grabs nothing).
        let mark = press_on(&s, &demo, "tile");
        s.input().path(to.at(0.5, 0.5));
        demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
        s.input().key("Escape");
        let (_, end) = demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
        assert_eq!(end["outcome"], "cancelled", "{end}");
        s.input().button(false);
    }
    // A drag in that leaves the zone and drops elsewhere: the zone hears it leave, no drop.
    let from = to;
    demo.zone(None, json!({ "mimes": [URI_LIST] }));
    press_on_peer(&s, &peer, &from);
    let mark = demo.app.line_count();
    peer.drag_start(
        &[Offer::text(URI_LIST, "file:///tmp/p1/x\r\n")],
        &[Action::Copy],
    );
    s.input().path(demo.at(&s, "main", "zone", 0.5, 0.5));
    demo.wait(mark, "the zone hovered", |v| {
        v["ev"] == "hover" && v["state"] != "left"
    });
    s.input().path(from.at(0.5, 0.5));
    demo.wait(mark, "the hover ending", |v| {
        v["ev"] == "hover" && v["state"] == "left"
    });
    s.input().button(false);
    std::thread::sleep(Duration::from_millis(300));
    let drops = demo
        .lines()
        .iter()
        .skip(mark)
        .filter(|v| v["ev"] == "drop")
        .count();
    assert_eq!(drops, 0, "a drag that left dropped");
    demo.quit();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_cancelled_drags_transfer_nothing() {
    cancelled_drags(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_cancelled_drags_transfer_nothing() {
    cancelled_drags(Display::X11);
}

/// A target with `on_reject` hears the moves of a drag it refuses (for autoscroll that follows
/// any drag, as Strata's does), and the release drops nothing: the peer, refused, leaves.
fn a_refusing_target_hears_moves(display: Display) {
    let s = Session::start();
    let demo = Demo::start(&s, display, false);
    let (peer, from) = peer_beside(&s, display, &demo);
    demo.zone(None, json!({ "mimes": [URI_LIST], "action": "refuse" }));
    press_on_peer(&s, &peer, &from);
    let mark = demo.app.line_count();
    peer.drag_start(
        &[Offer::text(URI_LIST, "file:///tmp/p1/x\r\n")],
        &[Action::Copy],
    );
    s.input().path(demo.at(&s, "main", "zone", 0.3, 0.5));
    let (hovered, _) = demo.wait(mark, "the zone refusing", |v| {
        v["ev"] == "hover" && v["state"] == "rejecting"
    });
    s.input().path(demo.at(&s, "main", "zone", 0.7, 0.5));
    let (_, mv) = demo.wait(hovered, "a move while refusing", |v| v["ev"] == "move");
    let zone = demo.rects("main")["zone"];
    let x = mv["x"].as_f64().unwrap_or(-1.0);
    assert!(
        (0.0..=zone[2]).contains(&x),
        "{mv}: in the zone's own points, {zone:?}"
    );
    s.input().button(false);
    demo.wait(hovered, "the drag leaving", |v| {
        v["ev"] == "hover" && v["state"] == "left"
    });
    wait_peer(&peer, "the drag ending", |e| {
        matches!(e, Event::DragFinished { .. } | Event::DragCancelled)
    });
    assert!(
        !demo.lines().iter().skip(mark).any(|v| v["ev"] == "drop"),
        "a refused drag dropped"
    );
    demo.quit();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_a_refusing_target_hears_moves() {
    a_refusing_target_hears_moves(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_a_refusing_target_hears_moves() {
    a_refusing_target_hears_moves(Display::X11);
}

// ---- in-process ----

/// A drag between two windows of the app carries the typed payload: nothing is read.
fn in_process_typed_payload(display: Display) {
    let s = Session::start();
    let demo = Demo::start(&s, display, true);
    // Tiled side by side: both windows are the app's.
    let deadline = Instant::now() + timeout();
    while demo
        .window(&s, "main")
        .content
        .overlaps(&demo.window(&s, "second").content)
    {
        assert!(
            Instant::now() < deadline,
            "the two windows never tiled apart"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    demo.offer(
        &[Offer::text(URI_LIST, "file:///tmp/p1/local\r\n")],
        &["copy", "move"],
        Some("payload-42"),
        None,
    );
    demo.zone(
        Some("second"),
        json!({ "mimes": [URI_LIST], "local": true }),
    );
    let mark = press_on(&s, &demo, "tile");
    s.input().path(demo.at(&s, "second", "zone", 0.5, 0.5));
    demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
    let (_, hover) = demo.wait(mark, "the second window's zone hovered", |v| {
        v["ev"] == "hover" && v["window"] == "second" && v["state"] != "left"
    });
    assert_eq!(hover["local"], true, "{hover}");
    s.input().button(false);
    let (_, dropped) = demo.wait(mark, "the drop", |v| {
        v["ev"] == "drop" && v["window"] == "second"
    });
    assert_eq!(dropped["local"], "payload-42", "{dropped}");
    assert_eq!(dropped["items"], json!([]), "nothing read: {dropped}");
    let (_, end) = demo.wait(mark, "the outcome", |v| {
        v["ev"] == "drag-end" && v["window"] == "main"
    });
    assert_eq!(end["outcome"], "copied", "{end}");
    let writers = demo
        .lines()
        .iter()
        .skip(mark)
        .filter(|v| v["ev"] == "writer")
        .count();
    assert_eq!(writers, 0, "a type was written for an in-process drop");
    demo.quit();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_in_process_drag_carries_the_typed_payload() {
    in_process_typed_payload(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_in_process_drag_carries_the_typed_payload() {
    in_process_typed_payload(Display::X11);
}

// ---- fast flicks (review round 1, B1 and B2) ----

/// Presses the tile, crosses the threshold and releases at once, `tries` times. Each drag
/// must end by itself (B1: on X11 a drag left running dropped on the next click anywhere),
/// no Escape may reach the app's keys (B2: on Wayland P1's Escape leaked when the release
/// had already ended Rune's capture), and a later plain click on the peer must drop nothing.
fn fast_flicks(display: Display) {
    let s = Session::start();
    let demo = Demo::start(&s, display, false);
    let (peer, to) = peer_beside(&s, display, &demo);
    demo.offer(&[Offer::text(TEXT, "flick")], &["copy", "move"], None, None);
    peer.accept_drops(&[TEXT], &[Action::Copy, Action::Move], Action::Move);
    for i in 0..5 {
        let mark = press_on(&s, &demo, "tile");
        let (tx, ty) = demo.at(&s, "main", "tile", 0.5, 0.5);
        s.input().pointer_move((tx + 30.0, ty));
        s.input().button(false);
        demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
        let (_, end) = demo.wait(mark, &format!("flick {i} ending by itself"), |v| {
            v["ev"] == "drag-end"
        });
        assert!(end["outcome"].is_string(), "{end}");
        // A later plain click on the peer drops nothing.
        let peer_mark = peer.line_count();
        s.point_at(&peer, to.at(0.5, 0.5));
        s.input().button(true);
        s.input().button(false);
        assert!(
            !peer.saw_since(peer_mark, Duration::from_millis(500), |e| {
                matches!(e, Event::Dropped { .. } | Event::DropEnter { .. })
            }),
            "flick {i}: a later click on the peer took a drop"
        );
    }
    let escapes: Vec<Value> = demo
        .lines()
        .into_iter()
        .filter(|v| v["ev"] == "key" && v["key"] == "Escape")
        .collect();
    assert!(
        escapes.is_empty(),
        "P1's Escape reached the app: {escapes:?}"
    );
    demo.quit();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_fast_flicks_end_and_leak_no_escape() {
    fast_flicks(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_fast_flicks_end_by_themselves() {
    fast_flicks(Display::X11);
}

/// X11: the drag's icon is an override-redirect window typed `_NET_WM_WINDOW_TYPE_DND`
/// (review round 1), mapped while the drag runs and gone once it ends.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_drag_icon_is_a_dnd_window_for_the_drags_life() {
    const DND: &str = "_NET_WM_WINDOW_TYPE_DND";
    let s = Session::start();
    let demo = Demo::start(&s, Display::X11, false);
    let (peer, to) = peer_beside(&s, Display::X11, &demo);
    demo.offer(&[Offer::text(TEXT, "icon")], &["copy"], None, Some("image"));
    peer.accept_drops(&[TEXT], &[Action::Copy], Action::Copy);
    let mark = press_on(&s, &demo, "tile");
    let half = (
        (to.at(0.5, 0.5).0 + demo.at(&s, "main", "tile", 0.5, 0.5).0) / 2.0,
        to.at(0.5, 0.5).1,
    );
    s.input().path(half);
    demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
    let deadline = Instant::now() + timeout();
    let icon = loop {
        let found = peer.windows_of_type(DND);
        if let Some(w) = found.iter().find(|w| w.mapped) {
            break w.clone();
        }
        assert!(
            Instant::now() < deadline,
            "no mapped {DND} window: {found:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        (icon.width, icon.height),
        (32, 24),
        "the example's 32×24 image"
    );
    s.input().path(to.at(0.5, 0.5));
    s.input().button(false);
    demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
    let deadline = Instant::now() + timeout();
    while peer.windows_of_type(DND).iter().any(|w| w.xid == icon.xid) {
        assert!(
            Instant::now() < deadline,
            "the icon window outlived its drag"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    demo.quit();
}

// ---- review round 2 ----

/// R2-B2: the threshold is crossed with the pointer already over the peer (one jump), so the
/// drag never enters the app's windows. Rune's capture must still end while the button is
/// held (the peer's answer is the compositor's proof), not wait for sway's focus bounce at
/// the drop; and afterwards no Escape reached the app.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_capture_ends_for_a_drag_that_never_enters_our_windows() {
    let s = Session::start();
    let demo = Demo::start(&s, Display::Wayland, false);
    let (peer, to) = peer_beside(&s, Display::Wayland, &demo);
    demo.offer(&[Offer::text(TEXT, "off")], &["copy"], None, None);
    peer.accept_drops(&[TEXT], &[Action::Copy], Action::Copy);
    let mark = press_on(&s, &demo, "manual");
    s.input().pointer_move(to.at(0.5, 0.5));
    demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
    wait_peer(&peer, "the drag entering the peer", |e| {
        matches!(e, Event::DropEnter { .. })
    });
    let (_, up) = demo.wait(mark, "the capture ending, button held", |v| {
        v["ev"] == "manual-phase" && v["phase"] == "Up"
    });
    assert_eq!(up["cancelled"], true, "{up}");
    assert_eq!(up["owned"], true, "{up}");
    let entered = demo.lines().iter().skip(mark).any(|v| v["ev"] == "hover");
    assert!(
        !entered,
        "the drag was meant never to enter the app's window"
    );
    s.input().button(false);
    wait_peer(&peer, "the drop", |e| matches!(e, Event::Dropped { .. }));
    demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
    let escapes = demo
        .lines()
        .iter()
        .filter(|v| v["ev"] == "key" && v["key"] == "Escape")
        .count();
    assert_eq!(escapes, 0, "P1's Escape reached the app");
    demo.quit();
}

/// R2-B2's last case: a drag that starts over no surface at all (one jump into sway's outer
/// gap around the window) gets no proof: nothing enters the app's windows and no target
/// answers, so P1 sends no Escape (path 3 in the module docs). It is released there: on
/// sway the capture ends by sway's focus change at the drop, no Escape reaches the app, and
/// hover comes back. Elsewhere it would wait for the window's next press or focus change.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_a_drag_over_nothing_ends_its_capture_after_it_ends() {
    let s = Session::start();
    let demo = Demo::start(&s, Display::Wayland, false);
    // Outer gaps leave bare desktop around the tiled window: nothing there to enter.
    let before = demo.settle(&s, "main").content.width;
    s.sway_command("gaps outer current set 150");
    let deadline = Instant::now() + timeout();
    while demo.settle(&s, "main").content.width >= before - 1.0 {
        assert!(
            Instant::now() < deadline,
            "the gaps never shrank the window"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    demo.offer(&[Offer::text(TEXT, "nowhere")], &["copy"], None, None);
    let mark = press_on(&s, &demo, "manual");
    s.input().pointer_move((40.0, 40.0));
    demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
    s.input().button(false);
    let (_, end) = demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
    assert_eq!(end["outcome"], "cancelled", "{end}");
    let (_, up) = demo.wait(mark, "the capture ending after the drag", |v| {
        v["ev"] == "manual-phase" && v["phase"] == "Up"
    });
    assert_eq!(up["owned"], true, "{up}");
    let hover_mark = demo.app.line_count();
    let plain = demo.at(&s, "main", "plain", 0.5, 0.5);
    s.point_until(plain, || {
        demo.lines()
            .iter()
            .skip(hover_mark)
            .any(|v| v["ev"] == "plain-hover" && v["over"] == true)
    });
    std::thread::sleep(Duration::from_millis(300));
    let escapes = demo
        .lines()
        .iter()
        .filter(|v| v["ev"] == "key" && v["key"] == "Escape")
        .count();
    assert_eq!(escapes, 0, "P1's Escape reached the app");
    demo.quit();
}

/// Drag and drop alone, where Wayland has no route at all (libwayland < 1.23 and a compositor
/// without data control, both simulated, with Xwayland at hand): attaching is unsupported,
/// and opens no X11 connection to Xwayland, which drag and drop has no use for.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_without_any_route_dnd_alone_stays_unsupported() {
    let s = Session::start();
    let mut cmd = s.command(example("dnd"), Display::Wayland);
    cmd.envs(s.env(Display::X11));
    cmd.args([
        "--title",
        "p1 dnd",
        "--as-if-libwayland-1-22",
        "--as-if-no-data-control",
    ]);
    let app = s.spawn("dnd", cmd);
    let v = app.wait_json("attach", |v| {
        v["ev"] == "attached" || v["ev"] == "attach-error"
    });
    assert_eq!(v["ev"], "attach-error", "{v}");
    let error = v["error"].as_str().unwrap_or_default();
    assert!(error.contains("unsupported"), "{v}");
}

//! P3's session tests: the `csd` example window under headless sway, borderless on X11
//! (Xwayland) and on Wayland, moving from its header, resizing from its edges where it can,
//! and its buttons reaching the window manager.
//!
//! The window floats (a `for_window` rule), because sway resizes only floating windows on a
//! client's request. sway is wlroots: its X11 window manager doesn't grab the pointer, so the
//! X11 resize works here whatever mutter and KWin do (a hand test, still pending).
//!
//! Feature map: `app/window`, "dragging … header moves the window and dragging a window edge
//! resizes it", and "Minimize minimizes the window and Close closes it": Close on both backends,
//! Minimise as far as the request reaching the window manager (sway minimises nothing).
//! Maximise is a hand test: floating sway ignores it, so only the request is checked.
//!
//! Work package: P3, `docs/work/phase-1-leaves/P3-client-side-decorations.md`.

use std::time::{Duration, Instant};

use serde_json::Value;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt, EventMask};

use crate::support::{Display, Options, Proc, Session, Window, example, timeout};

const APP_ID: &str = "strata-csd-example";

/// A session whose sway floats the example, and gives windows its default borders back (W3's
/// config turns them off): a title bar for a window that asks for server-side decorations,
/// and `csd` for one that draws its own.
fn session() -> Session {
    Session::with(Options {
        sway_config: format!(
            "for_window [app_id=\"{APP_ID}\"] floating enable\n\
             for_window [class=\"{APP_ID}\"] floating enable\n\
             default_border normal\n\
             default_floating_border normal\n"
        ),
        ..Options::default()
    })
}

struct App {
    proc: Proc,
    /// The `csd` line the window last logged.
    csd: Value,
    xid: Option<u32>,
}

/// Opens the example with `args` and waits until it is linked to its window (on X11 that
/// includes the first read of its state and the window manager's capabilities), floating, and
/// laid out at sway's size.
fn open(s: &Session, display: Display, args: &[&str]) -> (App, Window) {
    let mut cmd = s.command(example("csd"), display);
    cmd.args(args);
    let proc = s.spawn("csd", cmd);
    let (_, ready) = proc.wait_json_from(0, "ready", |v| v["ev"] == "ready");
    proc.wait_json("the first frame", |v| v["ev"] == "frame");
    let (_, csd) = proc.wait_json_from(0, "the window linked", |v| {
        v["ev"] == "csd" && v["notice"] == "attached"
    });
    let window = s.wait_window_where("the floating example", |w| {
        w.pid == proc.pid() && w.floating && w.content.width > 0.0
    });
    wait_layout(&proc, &window);
    let xid = ready["xid"].as_u64().and_then(|x| u32::try_from(x).ok());
    (App { proc, csd, xid }, window)
}

/// Waits for the example to lay out at the window's size, so its bands sit on its edges.
fn wait_layout(app: &Proc, window: &Window) {
    let (w, h) = (window.content.width, window.content.height);
    app.wait_json(&format!("a layout at {w}x{h}"), |v| {
        v["ev"] == "layout"
            && (v["w"].as_f64().unwrap_or(0.0) - w).abs() < 1.0
            && (v["h"].as_f64().unwrap_or(0.0) - h).abs() < 1.0
    });
}

/// Waits for sway to report the window's rect passing `pred`.
fn wait_rect(s: &Session, pid: u32, what: &str, pred: impl Fn(&Window) -> bool) -> Window {
    s.wait_window_where(what, |w| w.pid == pid && pred(w))
}

/// Presses at `from`, moves to `to` with the button held, and releases.
///
/// The pointer moves twice first and settles: a press right after a floating window maps or
/// resizes can go nowhere (docs/testing.md, "Point, then press, after a relayout").
fn drag(s: &Session, from: (f64, f64), to: (f64, f64)) {
    let input = s.input();
    input.pointer_move((from.0 + 1.0, from.1));
    std::thread::sleep(Duration::from_millis(200));
    input.pointer_move(from);
    std::thread::sleep(Duration::from_millis(200));
    input.button(true);
    input.path(to);
    input.button(false);
}

/// A click at `at`, after the pointer has moved there twice and settled, as [`drag`] does.
fn click(s: &Session, at: (f64, f64)) {
    let input = s.input();
    input.pointer_move((at.0 + 1.0, at.1));
    std::thread::sleep(Duration::from_millis(200));
    input.click_at(at);
}

/// The centre of a button the example logged, in layout points.
fn button(app: &Proc, window: &Window, key: &str) -> (f64, f64) {
    let (_, r) = app.wait_json_from(0, key, |v| v["ev"] == "rect" && v["key"] == key);
    let at = |k: &str| r[k].as_f64().unwrap_or(0.0);
    (
        window.content.x + at("x") + at("w") / 2.0,
        window.content.y + at("y") + at("h") / 2.0,
    )
}

/// sway's own tree node for the window: its `border` says who draws the frame. Right after a
/// window maps and floats, sway's tree can still say `none` for a window it frames: wait
/// until it says something else.
fn border(s: &Session, pid: u32) -> String {
    let deadline = Instant::now() + timeout();
    loop {
        let border = border_now(s, pid);
        if border != "none" || Instant::now() >= deadline {
            return border;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn border_now(s: &Session, pid: u32) -> String {
    node(s, pid)
        .and_then(|n| n["border"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// sway's tree node for the window of `pid`.
fn node(s: &Session, pid: u32) -> Option<Value> {
    fn find(node: &Value, pid: u32) -> Option<&Value> {
        if node["pid"].as_u64() == Some(u64::from(pid)) {
            return Some(node);
        }
        ["nodes", "floating_nodes"]
            .iter()
            .flat_map(|k| node[*k].as_array().into_iter().flatten())
            .find_map(|n| find(n, pid))
    }
    find(&s.tree(), pid).cloned()
}

/// The geometry sway gives the window's container (`rect`): what the window manager did,
/// whether or not the X client has caught up with it yet (see [`followed`]).
fn wm_rect(s: &Session, pid: u32) -> [f64; 4] {
    let r = node(s, pid).map(|n| n["rect"].clone()).unwrap_or_default();
    let at = |k: &str| r[k].as_f64().unwrap_or(0.0);
    [at("x"), at("y"), at("width"), at("height")]
}

/// Waits for sway's container geometry to pass `pred`.
fn wait_wm_rect(s: &Session, pid: u32, what: &str, pred: impl Fn([f64; 4]) -> bool) {
    let deadline = Instant::now() + timeout();
    loop {
        let r = wm_rect(s, pid);
        if pred(r) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: sway's rect is {r:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A resize from an edge or a corner: its name, where to press (fractions of the window), how
/// far the pointer goes, and how x, y, width and height change.
type Resize = (&'static str, (f64, f64), (f64, f64), [f64; 4]);

/// Every resize from an edge or a corner, 40 points outward.
const EDGES: [Resize; 8] = [
    ("n", (0.5, 0.0), (0.0, -40.0), [0.0, -40.0, 0.0, 40.0]),
    ("ne", (1.0, 0.0), (40.0, -40.0), [0.0, -40.0, 40.0, 40.0]),
    ("e", (1.0, 0.5), (40.0, 0.0), [0.0, 0.0, 40.0, 0.0]),
    ("se", (1.0, 1.0), (40.0, 40.0), [0.0, 0.0, 40.0, 40.0]),
    ("s", (0.5, 1.0), (0.0, 40.0), [0.0, 0.0, 0.0, 40.0]),
    ("sw", (0.0, 1.0), (-40.0, 40.0), [-40.0, 0.0, 40.0, 40.0]),
    ("w", (0.0, 0.5), (-40.0, 0.0), [-40.0, 0.0, 40.0, 0.0]),
    ("nw", (0.0, 0.0), (-40.0, -40.0), [-40.0, -40.0, 40.0, 40.0]),
];

/// A point 3 points inside the window at fractions (fx, fy) of it: on a side's 6-point band,
/// or a corner's 12-point one.
fn inside(window: &Window, (fx, fy): (f64, f64)) -> (f64, f64) {
    let c = window.content;
    let x = c.x
        + c.width * fx
        + if fx == 0.0 {
            3.0
        } else if fx == 1.0 {
            -3.0
        } else {
            0.0
        };
    let y = c.y
        + c.height * fy
        + if fy == 0.0 {
            3.0
        } else if fy == 1.0 {
            -3.0
        } else {
            0.0
        };
    (x, y)
}

/// Whether the window followed a drag of `by` points with a change of `moved`: the same way, at
/// least half as far, and no further than the pointer went; no change where `by` is 0.
///
/// sway starts a move or resize a step or two into the drag, so the window follows a little
/// less far than the pointer went ((76, 57) for (80, 60) here).
///
/// The resize test reads sway's container geometry, not the client's: under Xwayland the
/// client can stay short of the size sway gave it (seen: a left edge moved by 38 while the
/// client grew by only 6 or 26; W3's "floating windows" trap, where sway's configure doesn't
/// always reach an X11 client). That is between wlroots and Xwayland; this module's part ends
/// once the window manager has taken the resize.
fn followed(moved: f64, by: f64) -> bool {
    if by == 0.0 {
        moved.abs() <= 3.0
    } else {
        moved.signum() == by.signum()
            && moved.abs() >= by.abs() / 2.0
            && moved.abs() <= by.abs() + 3.0
    }
}

/// Each edge and corner on a fresh window, so one resize's aftermath can't move the next
/// one's bands: after a resize under Xwayland the app can stay a step short of sway's size
/// (see [`followed`]), which would leave a band a few points off the edge sway reports.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_window_resizes_from_each_edge() {
    let s = session();
    for (name, at, by, change) in EDGES {
        let (app, window) = open(&s, Display::X11, &["--title", name]);
        assert_eq!(app.csd["backend"], "x11");
        assert_eq!(app.csd["frame"], "borderless", "Auto is borderless on X11");
        assert_eq!(app.csd["bands"], true);
        assert_eq!(
            app.csd["capabilities"]["resize"], true,
            "wlroots lists _NET_WM_MOVERESIZE"
        );
        let start = wm_rect(&s, app.proc.pid());
        let x = Observer::new(&s);
        let from = inside(&window, at);
        drag(&s, from, (from.0 + by.0, from.1 + by.1));
        // EWMH: the pointer on the root window, the direction, the left button, and 1 for a
        // request from an application. sway's layout points are root pixels at scale 1.
        let xid = app.xid.expect("the window's X id");
        let sent = x
            .message("_NET_WM_MOVERESIZE", xid, timeout())
            .expect("_NET_WM_MOVERESIZE");
        let root = |v: u32| f64::from(v.cast_signed());
        assert!(
            (root(sent[0]) - from.0).abs() <= 1.0 && (root(sent[1]) - from.1).abs() <= 1.0,
            "{name}: sent at {sent:?}, pressed at {from:?}"
        );
        assert_eq!(sent[2..], [direction(name), 1, 1], "{name}: {sent:?}");
        let pid = app.proc.pid();
        wait_wm_rect(&s, pid, &format!("the window resized from {name}"), |r| {
            (0..4).all(|i| followed(r[i] - start[i], change[i]))
        });
        let request = format!("Resize({})", edge_name(name));
        app.proc
            .wait_json(&format!("the {name} resize request"), |v| {
                v["ev"] == "outcome" && v["route"] == "X11" && v["request"] == request.as_str()
            });
    }
}

/// `_NET_WM_MOVERESIZE`'s direction for an edge: 0 is top-left, then clockwise.
fn direction(short: &str) -> u32 {
    match short {
        "nw" => 0,
        "n" => 1,
        "ne" => 2,
        "e" => 3,
        "se" => 4,
        "s" => 5,
        "sw" => 6,
        _ => 7,
    }
}

fn edge_name(short: &str) -> &'static str {
    match short {
        "n" => "North",
        "ne" => "NorthEast",
        "e" => "East",
        "se" => "SouthEast",
        "s" => "South",
        "sw" => "SouthWest",
        "w" => "West",
        _ => "NorthWest",
    }
}

/// Moving from the header is Rune's `.window_drag()`, which `csd::header` keeps.
fn header_drag_moves(display: Display) {
    let s = session();
    let (app, window) = open(&s, display, &[]);
    let before = window.content;
    let from = (before.x + 200.0, before.y + 20.0);
    drag(&s, from, (from.0 + 80.0, from.1 + 60.0));
    wait_rect(
        &s,
        app.proc.pid(),
        "the window moved by about (80, 60)",
        |w| {
            let c = w.content;
            followed(c.x - before.x, 80.0)
                && followed(c.y - before.y, 60.0)
                && c.width == before.width
                && c.height == before.height
        },
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_header_drag_moves_the_window() {
    header_drag_moves(Display::X11);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_header_drag_moves_the_window() {
    header_drag_moves(Display::Wayland);
}

/// Watches the X server's root window for the client messages sent to the window manager.
struct Observer {
    conn: x11rb::rust_connection::RustConnection,
}

impl Observer {
    fn new(s: &Session) -> Observer {
        let (_, display) = s.xwayland().expect("Xwayland runs");
        let (conn, screen) =
            x11rb::connect(Some(&format!(":{display}"))).expect("Xwayland connection");
        let root = conn.setup().roots[screen].root;
        let mask = ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY);
        conn.change_window_attributes(root, &mask)
            .expect("select")
            .check()
            .expect("selected");
        Observer { conn }
    }

    fn atom(&self, name: &str) -> u32 {
        self.conn
            .intern_atom(false, name.as_bytes())
            .expect("intern")
            .reply()
            .expect("atom")
            .atom
    }

    /// The data of the first `kind` client message about `window` within `wait`.
    fn message(&self, kind: &str, window: u32, wait: Duration) -> Option<[u32; 5]> {
        let kind = self.atom(kind);
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            match self.conn.poll_for_event().expect("events") {
                Some(Event::ClientMessage(e)) if e.type_ == kind && e.window == window => {
                    return Some(e.data.as_data32());
                }
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        None
    }
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_buttons_reach_the_window_manager() {
    let s = session();
    let (app, window) = open(&s, Display::X11, &[]);
    let xid = app.xid.expect("the window's X id");
    let x = Observer::new(&s);
    let input = s.input();

    click(&s, button(&app.proc, &window, "minimize"));
    let iconify = x
        .message("WM_CHANGE_STATE", xid, timeout())
        .expect("WM_CHANGE_STATE");
    assert_eq!(iconify[0], 3, "IconicState");

    click(&s, button(&app.proc, &window, "maximize"));
    let state = x
        .message("_NET_WM_STATE", xid, timeout())
        .expect("_NET_WM_STATE");
    let (horz, vert) = (
        x.atom("_NET_WM_STATE_MAXIMIZED_HORZ"),
        x.atom("_NET_WM_STATE_MAXIMIZED_VERT"),
    );
    assert_eq!(state[0], 1, "_NET_WM_STATE_ADD");
    assert!(
        state[1..3].contains(&horz) && state[1..3].contains(&vert),
        "{state:?}"
    );

    // wlroots has no window menu, so the header keeps no secondary click for one.
    assert_eq!(app.csd["capabilities"]["window_menu"], false);
    assert_eq!(app.csd["window_menu"], false);
    input.right_click_at((window.content.x + 200.0, window.content.y + 20.0));
    let menu = x.message("_GTK_SHOW_WINDOW_MENU", xid, Duration::from_secs(1));
    assert_eq!(menu, None);
    let outcomes: Vec<String> = app
        .proc
        .lines()
        .into_iter()
        .filter(|l| l.contains("\"outcome\""))
        .collect();
    assert_eq!(
        outcomes.len(),
        2,
        "minimise and maximise only: {outcomes:?}"
    );
}

/// Close goes through Rune on every backend: the window leaves sway's tree and the app, whose
/// only window it was, exits.
fn close_closes(display: Display) {
    let s = session();
    let (mut app, window) = open(&s, display, &[]);
    let pid = app.proc.pid();
    click(&s, button(&app.proc, &window, "close"));
    let outcome = app
        .proc
        .wait_json("Close's outcome", |v| v["ev"] == "outcome");
    assert_eq!(
        (&outcome["request"], &outcome["route"]),
        (&"Close".into(), &"Rune".into())
    );
    let status = app.proc.wait_exit();
    assert!(status.success(), "the app exits cleanly: {status}");
    let deadline = Instant::now() + timeout();
    // Compact JSON, so `"pid":N` is followed by `,` or `}`: no other pid's prefix matches.
    let gone = |t: &str| {
        !t.contains(&format!("\"pid\":{pid},")) && !t.contains(&format!("\"pid\":{pid}}}"))
    };
    while !gone(&s.tree().to_string()) {
        assert!(
            Instant::now() < deadline,
            "the window is still in sway's tree"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_close_closes_the_window() {
    close_closes(Display::X11);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_close_closes_the_window() {
    close_closes(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_full_screen_puts_the_bands_away() {
    let s = session();
    let (app, window) = open(&s, Display::X11, &[]);
    let mark = app.proc.line_count();
    s.sway_command(&format!("[con_id={}] fullscreen enable", window.id));
    let (mark, _) = app
        .proc
        .wait_json_from(mark, "full screen, without bands", |v| {
            v["ev"] == "csd" && v["state"]["fullscreen"] == true && v["bands"] == false
        });
    s.sway_command(&format!("[con_id={}] fullscreen disable", window.id));
    app.proc
        .wait_json_from(mark + 1, "windowed, with bands", |v| {
            v["ev"] == "csd" && v["state"]["fullscreen"] == false && v["bands"] == true
        });
}

/// Wayland under `Auto`: borderless, with no resize, minimise or window menu until Rune ships
/// drafts U6–U8 (not filed). The edges take no press, and Minimise answers its gap.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_window_is_borderless_without_resize_minimise_or_menu() {
    let s = session();
    let (app, window) = open(&s, Display::Wayland, &[]);
    assert_eq!(
        border(&s, app.proc.pid()),
        "csd",
        "the client draws its frame"
    );
    let csd = &app.csd;
    assert_eq!(
        (&csd["backend"], &csd["frame"]),
        (&"wayland".into(), &"borderless".into())
    );
    assert_eq!(csd["bands"], false, "U6: no bands");
    assert_eq!(csd["window_menu"], false, "U8: no window menu");
    let caps = &csd["capabilities"];
    assert_eq!(
        (&caps["resize"], &caps["minimize"]),
        (&false.into(), &false.into())
    );

    let before = window.content;
    let from = inside(&window, (1.0, 0.5));
    drag(&s, from, (from.0 + 40.0, from.1));
    click(&s, button(&app.proc, &window, "minimize"));
    let outcome = app
        .proc
        .wait_json("Minimise's outcome", |v| v["ev"] == "outcome");
    assert_eq!(outcome["request"], "Minimize");
    assert_eq!(outcome["result"], "unsupported");
    assert_eq!(outcome["draft"], "U7");
    let after = s.wait_window(app.proc.pid()).content;
    assert_eq!(
        (after.width, after.height),
        (before.width, before.height),
        "a drag on the edge resized nothing"
    );
    let outcomes = app
        .proc
        .lines()
        .into_iter()
        .filter(|l| l.contains("\"outcome\""))
        .count();
    assert_eq!(outcomes, 1, "the edge asked for nothing");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_native_policy_keeps_the_server_frame() {
    let s = session();
    let (app, _) = open(&s, Display::Wayland, &["--frame", "native"]);
    assert_eq!(app.csd["frame"], "native");
    assert_eq!(app.csd["bands"], false);
    assert_eq!(border(&s, app.proc.pid()), "normal", "sway's title bar");
}

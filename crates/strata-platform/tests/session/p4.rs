//! P4's session tests: parenting to the peer's exported window.
//!
//! Work package: P4, `docs/work/phase-2-composites/P4-foreign-parent.md`. The probe is
//! `examples/foreign-parent.rs`: a host window, and a chooser per `open` command, each opened
//! hidden and parented from `App::on_window`, as a portal backend opens its dialog.
//!
//! sway's IPC shows no xdg-foreign parent (S3), so a Wayland parent is seen as sway treats a
//! transient: the window floats where a parentless one tiles, and shows over a full-screen
//! parent where a parentless one is hidden. On Xwayland the tree has `transient_for`, and the
//! hints are read from the X server.

use serde_json::{Value, json};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
use x11rb::rust_connection::RustConnection;

use crate::support::{Display, Proc, Session, Window, example, timeout};

fn probe(s: &Session, display: Display, args: &[&str]) -> Proc {
    let mut cmd = s.command(example("foreign-parent"), display);
    cmd.args(args);
    let p = s.spawn("foreign-parent", cmd);
    p.wait_json("the host window", |v| v["ev"] == "host-ready");
    p
}

/// Opens chooser `key` with `parent`, and returns what `apply` reported.
fn open(p: &Proc, key: &str, parent: &str, modal: bool) -> Value {
    let at = p.line_count();
    p.send_line(
        &json!({ "cmd": "open", "key": key, "parent": parent, "modal": modal }).to_string(),
    );
    p.wait_json_from(at, &format!("chooser {key} applied"), |v| {
        v["ev"] == "applied" && v["key"] == key
    })
    .1
}

/// Chooser `key`'s toplevel in sway's tree, once it has a size.
fn chooser(s: &Session, p: &Proc, key: &str) -> Window {
    let title = format!("chooser {key}");
    s.wait_window_where(&title, |w| {
        w.pid == p.pid() && w.title == title && w.content.width > 0.0
    })
}

/// sway's node `id`, from the raw tree.
fn node(tree: &Value, id: i64) -> Option<Value> {
    if tree["id"].as_i64() == Some(id) {
        return Some(tree.clone());
    }
    ["nodes", "floating_nodes"]
        .iter()
        .flat_map(|k| tree[*k].as_array().into_iter().flatten())
        .find_map(|n| node(n, id))
}

/// Waits until sway's node `id` passes `pred`, and returns it.
fn wait_node(s: &Session, id: i64, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = std::time::Instant::now() + timeout();
    loop {
        let n = node(&s.tree(), id);
        if let Some(n) = n.as_ref().filter(|n| pred(n)) {
            return n.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "waiting for {what}: {n:#?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn assert_parented(v: &Value) {
    assert_eq!(v["parented"], true, "{v}");
    assert_eq!(v["late"], false, "applied before the map: {v}");
}

fn assert_parentless(v: &Value, reason: &str, fallback: &str) {
    assert_eq!(v["parented"], false, "{v}");
    assert_eq!(v["reason"], reason, "{v}");
    assert_eq!(v["fallback"], fallback, "{v}");
}

fn quit(mut p: Proc, shutdown: bool) {
    let at = p.line_count();
    p.send_line(&json!({ "cmd": "quit", "shutdown": shutdown }).to_string());
    let status = p.wait_exit();
    assert!(status.success(), "the probe exits cleanly: {status}");
    p.wait_json_from(at, "the event loop's end", |v| v["ev"] == "exit");
}

// ---- Wayland ----

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_parented_window_floats_and_a_parentless_one_tiles() {
    let s = Session::start();
    let peer = s.peer(Display::Wayland);
    peer.map("p4 parent");
    let handle = peer.export();
    let p = probe(&s, Display::Wayland, &[]);

    assert_parentless(&open(&p, "c1", "", true), "NoParent", "Compositor");
    assert!(!chooser(&s, &p, "c1").floating, "a parentless window tiles");

    let parented = open(&p, "c2", &handle, true);
    assert_parented(&parented);
    assert_eq!(parented["system"], "Wayland");
    let c2 = s.wait_window_where("chooser c2 floating", |w| {
        w.pid == p.pid() && w.title == "chooser c2" && w.floating && w.content.width > 0.0
    });

    // Closing releases the import; a new request's window parents again.
    let at = p.line_count();
    p.send_line(r#"{"cmd":"close","key":"c2"}"#);
    let (_, closed) = p.wait_json_from(at, "chooser c2 closed", |v| {
        v["ev"] == "closed" && v["key"] == "c2"
    });
    assert_eq!(closed["released"], true, "{closed}");
    wait_gone(&s, c2.id);
    assert_parented(&open(&p, "c3", &handle, true));
    s.wait_window_where("chooser c3 floating", |w| {
        w.pid == p.pid() && w.title == "chooser c3" && w.floating && w.content.width > 0.0
    });
    quit(p, true);
}

fn wait_gone(s: &Session, id: i64) {
    let deadline = std::time::Instant::now() + timeout();
    while node(&s.tree(), id).is_some() {
        assert!(std::time::Instant::now() < deadline, "window {id} stays");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_parented_window_shows_over_a_fullscreen_parent() {
    let s = Session::start();
    let peer = s.peer(Display::Wayland);
    peer.map("p4 parent");
    let handle = peer.export();
    let p = probe(&s, Display::Wayland, &[]);
    let parent = s.wait_window(peer.pid());
    s.sway_command(&format!("[con_id={}] fullscreen enable", parent.id));
    wait_node(&s, parent.id, "the parent full screen", |n| {
        n["fullscreen_mode"] == 1
    });

    assert_parentless(&open(&p, "c1", "", true), "NoParent", "Compositor");
    let alone = s.wait_window_where("chooser c1", |w| {
        w.pid == p.pid() && w.title == "chooser c1"
    });
    assert_parented(&open(&p, "c2", &handle, true));
    let over = s.wait_window_where("chooser c2", |w| {
        w.pid == p.pid() && w.title == "chooser c2"
    });
    wait_node(&s, over.id, "the parented window shown", |n| {
        n["visible"] == true
    });
    let alone = node(&s.tree(), alone.id).expect("chooser c1 in the tree");
    assert_eq!(
        alone["visible"], false,
        "a parentless window hides: {alone:#}"
    );
    quit(p, true);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_refused_and_mismatched_parents_leave_it_parentless() {
    let s = Session::start();
    let wayland = s.peer(Display::Wayland);
    wayland.map("p4 parent");
    let handle = wayland.export();
    let x11 = s.peer(Display::X11);
    x11.map("p4 x11 parent");
    let x11_handle = x11.export();
    let p = probe(&s, Display::Wayland, &[]);

    // A handle the compositor never exported is destroyed at import, and winit's connection
    // lives on: the next window still parents.
    assert_parentless(
        &open(&p, "c1", "wayland:not-an-exported-handle", true),
        "Destroyed",
        "Compositor",
    );
    assert!(!chooser(&s, &p, "c1").floating, "c1 tiles");
    // S3 decision 2 (ruled: parentless): an X11 parent for a Wayland window.
    assert_parentless(&open(&p, "c2", &x11_handle, true), "Mismatch", "Compositor");
    assert!(!chooser(&s, &p, "c2").floating, "c2 tiles");
    // A handle Parent::parse refuses is no parent.
    let at = p.line_count();
    assert_parentless(&open(&p, "c3", "x11:0x0x1", true), "NoParent", "Compositor");
    p.wait_json_from(at, "the refused handle", |v| v["ev"] == "bad-parent");

    assert_parented(&open(&p, "c4", &handle, true));
    s.wait_window_where("chooser c4 floating", |w| {
        w.pid == p.pid() && w.title == "chooser c4" && w.floating
    });
    quit(p, true);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_parent_exit_reports_lost_and_keeps_the_window() {
    let s = Session::start();
    let peer = s.peer(Display::Wayland);
    peer.map("p4 parent");
    let handle = peer.export();
    let p = probe(&s, Display::Wayland, &[]);
    assert_parented(&open(&p, "c1", &handle, true));
    let c1 = s.wait_window_where("chooser c1 floating", |w| {
        w.pid == p.pid() && w.title == "chooser c1" && w.floating
    });
    let at = p.line_count();
    drop(peer);
    p.wait_json_from(at, "the parent lost", |v| {
        v["ev"] == "lost" && v["key"] == "c1"
    });
    assert!(node(&s.tree(), c1.id).is_some(), "the window stays open");
    // Released already: the lost import was dropped.
    let at = p.line_count();
    p.send_line(r#"{"cmd":"close","key":"c1"}"#);
    let (_, closed) = p.wait_json_from(at, "chooser c1 closed", |v| v["ev"] == "closed");
    assert_eq!(closed["released"], false, "{closed}");
    quit(p, true);
}

/// Closes chooser `key` (releasing it from `on_close` when `release`), and returns its
/// `closed` report.
fn close(s: &Session, p: &Proc, key: &str, release: bool) -> Value {
    let id = chooser(s, p, key).id;
    let at = p.line_count();
    p.send_line(&json!({ "cmd": "close", "key": key, "release": release }).to_string());
    let (_, closed) = p.wait_json_from(at, &format!("chooser {key} closed"), |v| {
        v["ev"] == "closed" && v["key"] == key
    });
    wait_gone(s, id);
    closed
}

/// Review 1, B1: the first window to attach P0's substrate gave it the only turn its reader
/// asked for. Once that window closed the turn never ran again, so no event on winit's
/// display reached anyone: a later window never heard its parent go. With or without the
/// first window's `release`.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_a_later_window_hears_its_parent_go_after_the_first_closes() {
    let s = Session::start();
    for release in [true, false] {
        let peer = s.peer(Display::Wayland);
        peer.map("p4 parent");
        let handle = peer.export();
        let p = probe(&s, Display::Wayland, &[]);
        assert_parented(&open(&p, "c1", &handle, true));
        let closed = close(&s, &p, "c1", release);
        assert_eq!(closed["released"], release, "{closed}");
        assert_parented(&open(&p, "c2", &handle, true));
        s.wait_window_where("chooser c2 floating", |w| {
            w.pid == p.pid() && w.title == "chooser c2" && w.floating
        });
        let at = p.line_count();
        drop(peer);
        p.wait_json_from(at, &format!("c2's parent lost (release {release})"), |v| {
            v["ev"] == "lost" && v["key"] == "c2"
        });
        quit(p, true);
    }
}

/// Review 1, should-fix 5: P0 kept every window that ever attached, one more per request.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_requests_one_after_another_leave_one_window_tracked() {
    let s = Session::start();
    let peer = s.peer(Display::Wayland);
    peer.map("p4 parent");
    let handle = peer.export();
    let p = probe(&s, Display::Wayland, &[]);
    for i in 0..8 {
        let key = format!("c{i}");
        let applied = open(&p, &key, &handle, true);
        assert_parented(&applied);
        assert_eq!(applied["p0_windows"], 1, "request {i}: {applied}");
        let closed = close(&s, &p, &key, true);
        assert_eq!(closed["p0_windows"], 0, "request {i}: {closed}");
    }
    quit(p, true);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_without_winit_display_is_parentless() {
    // S3 decision 1 (ruled: unsupported, still parentless): libwayland 1.22, simulated.
    let s = Session::start();
    let peer = s.peer(Display::Wayland);
    peer.map("p4 parent");
    let handle = peer.export();
    let p = probe(&s, Display::Wayland, &["--as-if-libwayland-1-22"]);
    assert_parentless(&open(&p, "c1", &handle, true), "NoDisplay", "Compositor");
    assert!(!chooser(&s, &p, "c1").floating, "it tiles");
    quit(p, true);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn quitting_with_a_live_import_exits_cleanly() {
    // With the shutdown, the import is destroyed while winit's display lives; without it,
    // nothing is destroyed after the display is freed (S3's crash was a late destroy).
    let s = Session::start();
    let peer = s.peer(Display::Wayland);
    peer.map("p4 parent");
    let handle = peer.export();
    for shutdown in [true, false] {
        let p = probe(&s, Display::Wayland, &[]);
        assert_parented(&open(&p, "c1", &handle, true));
        p.wait_json("chooser c1 drawn", |v| {
            v["ev"] == "ready" && v["key"] == "c1"
        });
        quit(p, shutdown);
    }
}

// ---- X11 (Xwayland) ----

struct X {
    conn: RustConnection,
}

impl X {
    fn new(s: &Session) -> X {
        let (_, display) = s.xwayland().expect("Xwayland runs");
        let (conn, _) = x11rb::connect(Some(&format!(":{display}"))).expect("Xwayland connection");
        X { conn }
    }

    fn atom(&self, name: &str) -> u32 {
        self.conn
            .intern_atom(false, name.as_bytes())
            .expect("intern")
            .reply()
            .expect("atom")
            .atom
    }

    fn property(&self, window: u32, name: u32, ty: u32) -> Vec<u32> {
        self.conn
            .get_property(false, window, name, ty, 0, 64)
            .expect("get_property")
            .reply()
            .expect("property")
            .value32()
            .into_iter()
            .flatten()
            .collect()
    }

    fn transient_for(&self, window: u32) -> Vec<u32> {
        self.property(
            window,
            AtomEnum::WM_TRANSIENT_FOR.into(),
            AtomEnum::WINDOW.into(),
        )
    }

    fn atoms(&self, window: u32, name: &str) -> Vec<u32> {
        self.property(window, self.atom(name), AtomEnum::ATOM.into())
    }
}

/// The X window of sway's Xwayland node `id`.
fn xid_of(s: &Session, id: i64) -> u32 {
    let n = node(&s.tree(), id).expect("the node");
    n["window"]
        .as_u64()
        .and_then(|x| u32::try_from(x).ok())
        .unwrap_or_else(|| panic!("an X window: {n:#}"))
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_window_is_a_transient_modal_dialog_of_its_parent() {
    let s = Session::start();
    let peer = s.peer(Display::X11);
    let parent = peer.map("p4 x11 parent").xid.expect("an X window");
    let handle = peer.export();
    let p = probe(&s, Display::X11, &[]);
    let x = X::new(&s);
    let (dialog, modal) = (
        x.atom("_NET_WM_WINDOW_TYPE_DIALOG"),
        x.atom("_NET_WM_STATE_MODAL"),
    );

    for (key, is_modal) in [("c1", true), ("c2", false)] {
        let v = open(&p, key, &handle, is_modal);
        assert_parented(&v);
        assert_eq!(v["system"], "X11");
        let w = s.wait_window_where(&format!("chooser {key} floating"), |w| {
            w.pid == p.pid() && w.title == format!("chooser {key}") && w.floating
        });
        let n = wait_node(&s, w.id, "transient_for in the tree", |n| {
            !n["window_properties"]["transient_for"].is_null()
        });
        assert_eq!(
            n["window_properties"]["transient_for"].as_u64(),
            Some(u64::from(parent)),
            "{n:#}"
        );
        let xid = xid_of(&s, w.id);
        assert_eq!(x.transient_for(xid), vec![parent]);
        assert_eq!(x.atoms(xid, "_NET_WM_WINDOW_TYPE"), vec![dialog]);
        assert_eq!(
            x.atoms(xid, "_NET_WM_STATE").contains(&modal),
            is_modal,
            "{key}: modal {is_modal}"
        );
    }
    quit(p, true);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_stale_xid_and_wayland_handle_leave_it_a_parentless_dialog() {
    let s = Session::start();
    let gone = s.peer(Display::X11);
    gone.map("p4 x11 gone");
    let stale = gone.export();
    drop(gone);
    let wayland = s.peer(Display::Wayland);
    wayland.map("p4 wayland parent");
    let wayland_handle = wayland.export();
    let p = probe(&s, Display::X11, &[]);
    let x = X::new(&s);
    let dialog = x.atom("_NET_WM_WINDOW_TYPE_DIALOG");

    for (key, parent, reason, fallback) in [
        ("c1", stale.as_str(), "BadXid", "Centred"),
        ("c2", wayland_handle.as_str(), "Mismatch", "Compositor"),
        ("c3", "", "NoParent", "Compositor"),
    ] {
        assert_parentless(&open(&p, key, parent, true), reason, fallback);
        // A dialog all the same, as GTK's portal dialog is: sway floats it.
        let w = s.wait_window_where(&format!("chooser {key} floating"), |w| {
            w.pid == p.pid() && w.title == format!("chooser {key}") && w.floating
        });
        let xid = xid_of(&s, w.id);
        assert!(x.transient_for(xid).is_empty(), "{key}: no transient-for");
        assert_eq!(x.atoms(xid, "_NET_WM_WINDOW_TYPE"), vec![dialog], "{key}");
        assert!(
            !x.atoms(xid, "_NET_WM_STATE")
                .contains(&x.atom("_NET_WM_STATE_MODAL")),
            "{key}: not modal without a parent"
        );
    }
    quit(p, true);
}

/// Review 1, should-fix 3: `GetGeometry` answers for any drawable, so a pixmap passed as a
/// parent, and the root (`WM_TRANSIENT_FOR` = root is ICCCM's group transient), were taken
/// for parents.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_a_pixmap_or_the_root_is_no_parent() {
    let s = Session::start();
    let p = probe(&s, Display::X11, &[]);
    let x = X::new(&s);
    let screen = &x.conn.setup().roots[0];
    let (root, depth) = (screen.root, screen.root_depth);
    let pixmap = x.conn.generate_id().expect("an id");
    x.conn
        .create_pixmap(depth, pixmap, root, 1, 1)
        .expect("create_pixmap")
        .check()
        .expect("a pixmap");
    let dialog = x.atom("_NET_WM_WINDOW_TYPE_DIALOG");

    for (key, xid) in [("c1", pixmap), ("c2", root)] {
        let handle = format!("x11:{xid:x}");
        assert_parentless(&open(&p, key, &handle, true), "BadXid", "Centred");
        let w = s.wait_window_where(&format!("chooser {key} floating"), |w| {
            w.pid == p.pid() && w.title == format!("chooser {key}") && w.floating
        });
        let window = xid_of(&s, w.id);
        assert!(
            x.transient_for(window).is_empty(),
            "{key}: no transient-for"
        );
        assert_eq!(
            x.atoms(window, "_NET_WM_WINDOW_TYPE"),
            vec![dialog],
            "{key}"
        );
    }
    quit(p, true);
}

/// Waits until `window`'s `_NET_WM_STATE` has `_NET_WM_STATE_MODAL` (`on`) or hasn't.
fn wait_modal(x: &X, window: u32, on: bool) {
    let modal = x.atom("_NET_WM_STATE_MODAL");
    let deadline = std::time::Instant::now() + timeout();
    loop {
        let states = x.atoms(window, "_NET_WM_STATE");
        if states.contains(&modal) == on {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "waiting for modal {on}: {states:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Review 1, should-fix 4: once the window is mapped, modality is asked of the window manager
/// with a `_NET_WM_STATE` client message (EWMH), and the parent is set as a property.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_a_parent_applied_after_the_map_reaches_the_window_manager() {
    let s = Session::start();
    let peer = s.peer(Display::X11);
    let parent = peer.map("p4 x11 parent").xid.expect("an X window");
    let handle = peer.export();
    let p = probe(&s, Display::X11, &[]);
    let x = X::new(&s);
    assert_parentless(&open(&p, "c1", "", true), "NoParent", "Compositor");
    let w = s.wait_window_where("chooser c1 floating", |w| {
        w.pid == p.pid() && w.title == "chooser c1" && w.floating
    });
    let window = xid_of(&s, w.id);
    wait_modal(&x, window, false);

    let reapply = |parent: &str, modal: bool| {
        let at = p.line_count();
        p.send_line(
            &json!({ "cmd": "reapply", "key": "c1", "parent": parent, "modal": modal }).to_string(),
        );
        p.wait_json_from(at, "c1 applied again", |v| {
            v["ev"] == "applied" && v["key"] == "c1"
        })
        .1
    };
    let v = reapply(&handle, true);
    assert_eq!(v["parented"], true, "{v}");
    assert_eq!(v["late"], true, "applied after the map: {v}");
    assert_eq!(x.transient_for(window), vec![parent]);
    wait_node(&s, w.id, "transient_for in the tree", |n| {
        n["window_properties"]["transient_for"].as_u64() == Some(u64::from(parent))
    });
    wait_modal(&x, window, true);

    let v = reapply(&handle, false);
    assert_eq!(v["late"], true, "{v}");
    wait_modal(&x, window, false);

    let v = reapply("", true);
    assert_eq!(v["parented"], false, "{v}");
    assert!(x.transient_for(window).is_empty(), "the parent is gone");
    wait_modal(&x, window, false);
    quit(p, true);
}

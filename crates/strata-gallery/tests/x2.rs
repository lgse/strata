//! X2's session tests: the drag-and-drop wiring demo (`examples/dnd_wiring/`) against W3's
//! peer in a private headless sway, on Wayland and X11 (`mise run test-platform x2::`).
//!
//! Each test starts the demo on a fixture folder made in the session's private runtime
//! directory, finds its targets with the demo's `locate` command (window points from
//! `Cx::window_layout`), and drags with the session's virtual pointer. What the demo did
//! comes back as its JSON lines (`transfer`, `drag-end`, `spring`, …), and what the peer got
//! as its events.
//!
//! Work package: X2, `docs/work/phase-4-integration/X2-dnd-wiring.md`.

#[path = "../../strata-platform/tests/support/mod.rs"]
#[expect(
    dead_code,
    reason = "W3's harness, shared with strata-platform's session tests: X2 uses part of it"
)]
#[expect(
    unexpected_cfgs,
    unused_imports,
    reason = "the harness gates helpers on strata-platform's features, which this crate lacks \
              (so X2 uses none of those helpers: their expectations hold)"
)]
mod support;

mod x2 {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};
    use strata_test_peer::protocol::{Action, Event, Offer};

    use crate::support::{Display, Peer, Proc, Session, Window, example, timeout};

    const URI_LIST: &str = "text/uri-list";
    const GNOME: &str = "x-special/gnome-copied-files";
    const KDE: &str = "application/x-kde-cutselection";

    /// The fixture: folders with a file each, a long folder for autoscroll, `usb` (another
    /// volume to the demo), `pinned`.
    fn fixture(s: &Session) -> PathBuf {
        let root = s.runtime_dir().join("root");
        for d in [
            "Documents/Reports",
            "Music",
            "Pictures",
            "pinned",
            "usb",
            "Deep/a/b/c/d/e",
        ] {
            std::fs::create_dir_all(root.join(d)).expect("fixture folder");
        }
        for (f, body) in [
            ("notes.txt", "notes"),
            ("todo.txt", "todo"),
            ("Documents/plan.txt", "plan"),
            ("Documents/Reports/q3.txt", "q3"),
            ("usb/stick.txt", "stick"),
        ] {
            std::fs::write(root.join(f), body).expect("fixture file");
        }
        for i in 0..60 {
            std::fs::write(root.join(format!("Music/track{i:02}.ogg")), "x").expect("track");
        }
        // Folders then files, for List's file-type groups and their stuck header.
        for i in 0..20 {
            std::fs::create_dir_all(root.join(format!("Mixed/folder{i:02}"))).expect("folder");
            std::fs::write(root.join(format!("Mixed/file{i:02}.txt")), "x").expect("file");
        }
        root.canonicalize().expect("the root")
    }

    fn uri(p: &Path) -> String {
        // The fixture's paths need no escaping.
        format!("file://{}", p.display())
    }

    pub(super) struct Demo {
        pub app: Proc,
        pub title: String,
        pub keys: Vec<&'static str>,
        pub root: PathBuf,
    }

    impl Demo {
        pub fn start(s: &Session, display: Display, two: bool, apply: bool) -> Demo {
            let root = fixture(s);
            let title = "x2 dnd".to_owned();
            let mut cmd = s.command(example("dnd_wiring"), display);
            cmd.arg("--root").arg(&root).args(["--title", &title]);
            if two {
                cmd.arg("--two-windows");
            }
            if apply {
                cmd.arg("--apply");
            }
            let app = s.spawn("dnd_wiring", cmd);
            let keys = if two {
                vec!["main", "second"]
            } else {
                vec!["main"]
            };
            let demo = Demo {
                app,
                title,
                keys,
                root,
            };
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

        pub fn window(&self, s: &Session, key: &str) -> Window {
            let (title, pid) = (self.title_of(key), self.app.pid());
            s.wait_window_where(&format!("the {key} window"), |w| {
                w.pid == pid && w.title == title && w.content.width > 0.0
            })
        }

        pub fn send(&self, cmd: Value) -> usize {
            let mark = self.app.line_count();
            self.app.send_line(&cmd.to_string());
            mark
        }

        pub fn wait(
            &self,
            from: usize,
            what: &str,
            pred: impl Fn(&Value) -> bool,
        ) -> (usize, Value) {
            self.app.wait_json_from(from, what, pred)
        }

        /// Whether a line passing `pred` came from line `from` on, within `wait`.
        pub fn saw(&self, from: usize, wait: Duration, pred: impl Fn(&Value) -> bool) -> bool {
            let deadline = Instant::now() + wait;
            loop {
                if self.lines_from(from).iter().any(&pred) {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        pub fn lines_from(&self, from: usize) -> Vec<Value> {
            self.app
                .lines()
                .iter()
                .skip(from)
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .collect()
        }

        /// Waits until window `key` has laid out at the size sway gave it.
        pub fn settle(&self, s: &Session, key: &str) -> Window {
            let deadline = Instant::now() + timeout();
            loop {
                let w = self.window(s, key);
                let laid = self
                    .lines_from(0)
                    .iter()
                    .rev()
                    .find(|v| v["ev"] == "layout" && v["window"] == key)
                    .and_then(|v| v["w"].as_f64());
                if laid.is_some_and(|lw| (lw - w.content.width).abs() < 1.0) {
                    return w;
                }
                assert!(
                    Instant::now() < deadline,
                    "window {key} never laid out at {:?}: {laid:?}",
                    w.content
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        /// Sends `cmd` to window `key` and waits for the line `ack`.
        pub fn command(&self, key: &str, mut cmd: Value, ack: &str) -> Value {
            cmd["window"] = json!(key);
            let mark = self.send(cmd);
            self.wait(mark, ack, |v| v["ev"] == ack && v["window"] == key)
                .1
        }

        /// Where each of `what` is in window `key`, in the screen's points (sway's layout).
        pub fn locate(&self, s: &Session, key: &str, what: &[Value]) -> HashMap<String, [f64; 4]> {
            let w = self.settle(s, key);
            let deadline = Instant::now() + timeout();
            let mut last: Option<HashMap<String, [f64; 4]>> = None;
            loop {
                let mark = self.send(json!({ "cmd": "locate", "window": key, "what": what }));
                let mut out = HashMap::new();
                for item in what {
                    let name = item["name"].as_str().expect("named").to_owned();
                    let (_, v) = self.wait(mark, &format!("where {name} is ({item})"), |v| {
                        v["ev"] == "rect" && v["window"] == key && v["name"] == name.as_str()
                    });
                    if let Some(r) = v["r"].as_array() {
                        let r: Vec<f64> = r.iter().filter_map(Value::as_f64).collect();
                        if r.len() == 4 && r[2] > 0.0 && r[3] > 0.0 {
                            out.insert(name, [w.content.x + r[0], w.content.y + r[1], r[2], r[3]]);
                        }
                    }
                }
                // Settled: the same rects a moment later (a pane just shown lays out again, and
                // a grid reflows to its width).
                if out.len() == what.len() {
                    if last.as_ref() == Some(&out) {
                        return out;
                    }
                    last = Some(out);
                    std::thread::sleep(Duration::from_millis(150));
                    continue;
                }
                assert!(
                    Instant::now() < deadline,
                    "never located {what:?} in {key}: {out:?}"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        /// The point `(fx, fy)` across what `item` names in window `key`.
        pub fn at(&self, s: &Session, key: &str, item: Value, fx: f64, fy: f64) -> (f64, f64) {
            let mut item = item;
            item["name"] = json!("it");
            let r = self.locate(s, key, &[item])["it"];
            (r[0] + r[2] * fx, r[1] + r[3] * fy)
        }

        /// Gives window `key` keyboard focus through sway and waits until the app has heard it
        /// settle (a click that focuses an unfocused Xwayland window can lose its press).
        pub fn focus(&self, s: &Session, key: &str) {
            let w = self.window(s, key);
            if !w.focused {
                s.focus(&w);
            }
            let last = || {
                self.lines_from(0)
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, v)| v["ev"] == "focus" && v["window"] == key)
                    .map(|(i, v)| (i, v["focused"] == true))
            };
            let deadline = Instant::now() + timeout();
            loop {
                let before = last();
                std::thread::sleep(Duration::from_millis(250));
                let after = last();
                if before == after && after.is_some_and(|(_, f)| f) {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "window {key} never settled focused: {after:?}"
                );
            }
        }

        pub fn quit(mut self) {
            let mark = self.send(json!({ "cmd": "quit", "window": "main" }));
            self.wait(mark, "the shutdown", |v| v["ev"] == "shutdown");
            let status = self.app.wait_exit();
            assert!(status.success(), "the demo exited with {status}");
            self.no_errors();
        }

        pub fn no_errors(&self) {
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
            assert!(bad.is_empty(), "the demo reported errors: {bad:?}");
        }
    }

    /// A peer with its window mapped, tiled beside the demo's.
    pub(super) fn peer_beside(s: &Session, display: Display, demo: &Demo) -> (Peer, Window) {
        let peer = s.peer(display);
        peer.map("x2 peer");
        let windows = s.wait_windows(&[peer.pid(), demo.app.pid()]);
        let w = windows
            .iter()
            .find(|w| w.pid == peer.pid())
            .expect("the peer's window")
            .clone();
        peer.sync();
        (peer, w)
    }

    /// Presses on `at` in the demo's window `key`, focused first, and waits for the press to
    /// reach a collection. After a relayout a first motion can reach no surface (W3's traps),
    /// so a press that doesn't arrive is released and tried again.
    fn press(s: &Session, demo: &Demo, key: &str, at: (f64, f64)) -> usize {
        demo.focus(s, key);
        for _ in 0..5 {
            s.input().pointer_move((at.0 + 1.0, at.1));
            std::thread::sleep(Duration::from_millis(100));
            s.input().pointer_move(at);
            std::thread::sleep(Duration::from_millis(150));
            let mark = demo.app.line_count();
            s.input().button(true);
            if demo.saw(mark, Duration::from_secs(2), |v| {
                v["ev"] == "press" && v["window"] == key
            }) {
                return mark;
            }
            s.input().button(false);
            std::thread::sleep(Duration::from_millis(300));
        }
        panic!("no press reached {key} at {at:?}");
    }

    /// Drags `item` in window `key` out to `to` (the peer's window) and drops it: the outcome.
    fn drag_out(
        s: &Session,
        demo: &Demo,
        key: &str,
        item: Value,
        to: (f64, f64),
    ) -> (usize, Value) {
        drag_out_then(s, demo, key, item, to, || {})
    }

    /// [`drag_out`], running `before_release` while the drag rests on the target.
    fn drag_out_then(
        s: &Session,
        demo: &Demo,
        key: &str,
        item: Value,
        to: (f64, f64),
        before_release: impl FnOnce(),
    ) -> (usize, Value) {
        // A row's icon is near its start; a tile's `/icon` is the icon itself.
        let fx = if item.get("depth").is_none() && item["tab"] == 3 {
            0.5
        } else {
            0.1
        };
        let from = demo.at(s, key, item, fx, 0.5);
        let mark = press(s, demo, key, from);
        s.input()
            .path(((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0));
        demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
        s.input().path(to);
        // A motion after arriving, then time for the target to answer the compositor (a
        // window of this app picks its target in its next update).
        std::thread::sleep(Duration::from_millis(200));
        s.input().path((to.0 + 1.0, to.1));
        std::thread::sleep(Duration::from_millis(250));
        before_release();
        s.input().button(false);
        (
            mark,
            demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end").1,
        )
    }

    fn row(tab: u64, name: &str) -> Value {
        json!({ "tab": tab, "row": name })
    }

    // ---- Drag out ----

    /// operations/drag-and-drop: "Dragged items are offered as a GDK file list and a
    /// `text/uri-list`, so other applications can receive them" (the three formats Strata's
    /// port offers instead), from Columns, List and Icons in turn; the outcome reaches the
    /// demo; the dragged rows are drawn faded while the drag runs, and not after it.
    fn drag_out_from_each_view(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let (peer, to) = peer_beside(&s, display, &demo);
        let notes = uri(&demo.root.join("notes.txt"));
        peer.accept_drops_with(
            &[URI_LIST, GNOME, KDE],
            &[Action::Copy, Action::Move],
            Action::Copy,
            true,
            None,
        );
        let views = [
            (1u64, json!({ "tab": 1, "row": "notes.txt", "depth": 0 })),
            (2, row(2, "notes.txt")),
            (3, row(3, "notes.txt")),
        ];
        for (tab, item) in views {
            select_tab(&demo, "main", tab);
            let faded =
                || demo.command("main", json!({ "cmd": "state" }), "state")["faded"].clone();
            let (mark, end) = drag_out_then(&s, &demo, "main", item, to.at(0.5, 0.5), || {
                assert_eq!(faded(), json!(["notes.txt"]), "tab {tab}: during the drag");
            });
            assert_eq!(end["outcome"], "copied", "tab {tab}: {end}");
            let got = dropped_on_peer(&peer, 3);
            for (mime, want) in [
                (URI_LIST, format!("{notes}\r\n")),
                (GNOME, format!("copy\n{notes}")),
                (KDE, "0".to_owned()),
            ] {
                let (text, action) = got.get(mime).unwrap_or_else(|| panic!("{mime}: {got:?}"));
                assert_eq!(text.as_deref(), Some(want.as_str()), "tab {tab}, {mime}");
                assert_eq!(*action, Some(Action::Copy));
            }
            assert!(demo.saw(mark, Duration::ZERO, |v| v["ev"] == "drag-start"
                && v["items"] == json!(["notes.txt"])));
            assert_eq!(faded(), json!([]), "tab {tab}: after the drag");
        }
        demo.quit();
    }

    /// The next `n` drops the peer reads, by type: their text and action (they finish in any
    /// order on X11).
    fn dropped_on_peer(peer: &Peer, n: usize) -> HashMap<String, (Option<String>, Option<Action>)> {
        let mut out = HashMap::new();
        for _ in 0..n {
            if let Event::Dropped { mime, action, data } =
                peer.wait_event("a drop read", |e| matches!(e, Event::Dropped { .. }))
            {
                out.insert(mime, (data.text, action));
            }
        }
        out
    }

    fn select_tab(demo: &Demo, key: &str, tab: u64) {
        demo.send(json!({ "cmd": "tab", "id": tab, "window": key }));
        let st = demo.command(key, json!({ "cmd": "state" }), "state");
        assert_eq!(st["tab"], tab, "{st}");
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_drag_out_from_each_view_offers_the_three_formats() {
        drag_out_from_each_view(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_drag_out_from_each_view_offers_the_three_formats() {
        drag_out_from_each_view(Display::X11);
    }

    // ---- Drop in ----

    /// Presses on the peer's window, focused first through sway, and waits until the peer has
    /// the press.
    fn press_on_peer(s: &Session, peer: &Peer, window: &Window) {
        let w = s.wait_window_where("the peer's window", |w| w.id == window.id);
        if !w.focused {
            s.focus(&w);
        }
        // Settled: sway has it focused, and the peer said nothing for a while.
        let deadline = Instant::now() + timeout();
        loop {
            let before = peer.line_count();
            std::thread::sleep(Duration::from_millis(250));
            peer.sync();
            let quiet = peer.line_count() <= before + 1;
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
        peer.wait_event("the press", |e| {
            matches!(e, Event::PointerButton { pressed: true })
        });
    }

    /// A drag from the peer offering `offers`, moved to `path` (each point in turn, the last
    /// held `hold`), then dropped there. The demo's line count before it.
    fn drag_from_peer(
        s: &Session,
        peer: &Peer,
        from: &Window,
        offers: &[Offer],
        actions: &[Action],
        path: &[(f64, f64)],
        hold: Duration,
    ) {
        press_on_peer(s, peer, from);
        peer.drag_start(offers, actions);
        for at in path {
            s.input().path(*at);
            std::thread::sleep(Duration::from_millis(120));
        }
        // A motion after the last, so the target hears where the drag is.
        if let Some(last) = path.last() {
            s.input().path((last.0 + 1.0, last.1));
        }
        std::thread::sleep(hold);
        s.input().button(false);
    }

    fn files(root: &Path, names: &[&str]) -> Offer {
        let list: String = names
            .iter()
            .map(|n| format!("{}\r\n", uri(&root.join(n))))
            .collect();
        Offer::text(URI_LIST, &list)
    }

    const BOTH: [Action; 2] = [Action::Copy, Action::Move];

    /// Says a test ran nothing, on stderr past libtest's capture (which takes only `print!`
    /// and `eprint!`), so it shows in every run. libtest has no "skipped" result: the test
    /// still reports ok, and this line is what says it didn't run (docs/testing.md).
    fn skip(test: &str, why: &str) {
        use std::io::Write;
        // One write on the locked handle, so a parallel test's output can't split the line.
        let line = format!("SKIP x2::{test}: {why}\n");
        let _ = std::io::stderr().lock().write_all(line.as_bytes());
    }

    /// One drop from the peer onto `target` in the demo's main window: the demo's `transfer`
    /// line for it, or None when nothing was transferred within 3 seconds of the release.
    fn drop_on(
        s: &Session,
        demo: &Demo,
        peer: &Peer,
        from: &Window,
        names: &[&str],
        target: Value,
    ) -> Option<Value> {
        let at = demo.at(s, "main", target.clone(), 0.5, 0.5);
        let mark = demo.app.line_count();
        drag_from_peer(
            s,
            peer,
            from,
            &[files(&demo.root, names)],
            &BOTH,
            &[at],
            Duration::from_millis(250),
        );
        let got = demo.saw(mark, Duration::from_secs(3), |v| v["ev"] == "transfer");
        // Every drag ends at the peer, whatever the target did with it.
        peer.wait_event("the drag ending", |e| {
            matches!(e, Event::DragFinished { .. } | Event::DragCancelled)
        });
        got.then(|| {
            demo.lines_from(mark)
                .into_iter()
                .find(|v| v["ev"] == "transfer")
                .expect("seen")
        })
    }

    /// operations/drag-and-drop: "Dropping onto a folder row in Columns, List, or Icons
    /// transfers the dragged items into that folder"; "Dropping onto the background of a pane
    /// or column showing another folder transfers into that folder"; "A file list dragged from
    /// another application transfers like an internal drag"; tabs: a tab takes files into its
    /// folder; sidebar: "Sidebar Home,
    /// standard places, pinned folders, and mounted native devices accept file drops", the
    /// Trash takes files (trash: "Dropping files onto the sidebar Trash row moves them to
    /// Trash"; the demo logs them as trashed); copy-or-move: a plain drop on
    /// the same volume moves, onto another device asks ("Always ask", the default). New tab
    /// refuses folders from another app (it opens only this process's). A folder row that
    /// refuses a drop (its items' own folder, Strata's guard) stops it: the background around
    /// it doesn't take it instead.
    fn drop_in_on_each_target(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let (peer, from) = peer_beside(&s, display, &demo);
        let root = demo.root.clone();
        let into = |v: &Value| v["into"].as_str().map(PathBuf::from);
        let check = |what: &str, got: Option<Value>, dest: &Path, action: &str, via: &str| {
            let got = got.unwrap_or_else(|| panic!("{what}: no transfer"));
            assert_eq!(into(&got).as_deref(), Some(dest), "{what}: {got}");
            assert_eq!(got["action"], action, "{what}: {got}");
            assert_eq!(got["via"], via, "{what}: {got}");
            assert_eq!(got["local"], false, "{what}: {got}");
        };
        // Columns: a folder row; the background of a column showing another folder.
        select_tab(&demo, "main", 1);
        // A drag's hit test (P1's) reaches the column's row and background, not the bar layers
        // over them (C24's bars hang from an anchor).
        let w = demo.window(&s, "main");
        let pick = |item: Value| {
            let at = demo.at(&s, "main", item, 0.5, 0.5);
            let v = demo.command(
                "main",
                json!({ "cmd": "pick", "x": at.0 - w.content.x, "y": at.1 - w.content.y }),
                "picked",
            );
            v["path"].as_str().unwrap_or_default().to_owned()
        };
        let docs = pick(json!({ "tab": 1, "depth": 0, "row": "Documents" }));
        assert!(
            docs.contains("/t1-cols-col-1/") && docs.contains("/row-"),
            "{docs}"
        );
        let bg = pick(json!({ "tab": 1, "column": 0 }));
        assert!(bg.contains("/t1-cols-col-1/"), "{bg}");
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["notes.txt"],
            json!({ "tab": 1, "depth": 0, "row": "Documents" }),
        );
        check(
            "a Columns folder row",
            got,
            &root.join("Documents"),
            "move",
            "row",
        );
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["usb/stick.txt"],
            json!({ "tab": 1, "column": 0 }),
        );
        check("a column's background", got, &root, "ask", "background");
        let refused = |what: &str, names: &[&str], target: Value| {
            let got = drop_on(&s, &demo, &peer, &from, names, target);
            assert!(got.is_none(), "{what}: {got:?}");
        };
        // A folder row that refuses a drop stops it: passed on, the background around it
        // would take it into the folder shown.
        refused(
            "a Columns row, its item's own folder",
            &["Music/track00.ogg"],
            json!({ "tab": 1, "depth": 0, "row": "Music" }),
        );
        // A column's background refusing (its own folder) stops it too: the strip's target
        // around it doesn't take it instead.
        demo.command(
            "main",
            json!({ "cmd": "open", "tab": 1, "path": "Music" }),
            "navigate",
        );
        refused(
            "a column's background, its item's own folder",
            &["Music/track00.ogg"],
            json!({ "tab": 1, "column": 1 }),
        );
        // List and Icons: folder rows, and List's background.
        select_tab(&demo, "main", 2);
        let got = drop_on(&s, &demo, &peer, &from, &["notes.txt"], row(2, "Music"));
        check("a List folder row", got, &root.join("Music"), "move", "row");
        refused(
            "a List row, its item's own folder",
            &["Music/track00.ogg"],
            row(2, "Music"),
        );
        refused(
            "a List row, its folder's own parent",
            &["Documents/Reports"],
            row(2, "Documents"),
        );
        demo.command(
            "main",
            json!({ "cmd": "open", "tab": 2, "path": "Documents" }),
            "navigate",
        );
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["todo.txt"],
            json!({ "tab": 2, "pane": true }),
        );
        check(
            "List's background",
            got,
            &root.join("Documents"),
            "move",
            "background",
        );
        select_tab(&demo, "main", 3);
        let got = drop_on(&s, &demo, &peer, &from, &["notes.txt"], row(3, "Pictures"));
        check(
            "an Icons folder tile",
            got,
            &root.join("Pictures"),
            "move",
            "row",
        );
        refused(
            "an Icons tile, its item's own folder",
            &["Music/track00.ogg"],
            row(3, "Music"),
        );
        // Tabs: tab 2 shows Documents.
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["usb/stick.txt"],
            json!({ "tab": 2 }),
        );
        check("a tab", got, &root.join("Documents"), "ask", "tab");
        // The tab is selected: by its spring (450 ms) while the drag rested, or by the drop.
        assert!(
            demo.saw(0, Duration::ZERO, |v| v["ev"] == "tab-select"
                && v["tab"] == 2),
            "a drop on a tab selects it"
        );
        // The sidebar: a place, the device (another volume), the Trash.
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["notes.txt"],
            json!({ "place": "Music" }),
        );
        check(
            "a sidebar place",
            got,
            &root.join("Music"),
            "move",
            "sidebar",
        );
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["notes.txt"],
            json!({ "device": true }),
        );
        check("the USB device", got, &root.join("usb"), "ask", "sidebar");
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["notes.txt"],
            json!({ "trash": true }),
        );
        let got = got.expect("the Trash takes files");
        assert_eq!(
            (&got["into"], &got["action"], &got["sources"]),
            (&json!("trash"), &json!("trash"), &json!(["notes.txt"])),
            "{got}"
        );
        // New tab opens only this process's folder drags.
        let mark = demo.app.line_count();
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["Music"],
            json!({ "new-tab": true }),
        );
        assert!(got.is_none(), "{got:?}");
        assert!(!demo.saw(mark, Duration::ZERO, |v| v["ev"] == "tab-open"));
        // Onto the items' own folder: nothing (Strata's guard), shown before the drop. (The
        // drop on tab 2 selected it.)
        select_tab(&demo, "main", 3);
        let got = drop_on(&s, &demo, &peer, &from, &["Pictures"], row(3, "Pictures"));
        assert!(got.is_none(), "a folder into itself: {got:?}");
        demo.quit();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_drops_in_on_each_target() {
        drop_in_on_each_target(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_drops_in_on_each_target() {
        drop_in_on_each_target(Display::X11);
    }

    /// location-bar: "Parent breadcrumbs accept file drops" (under P1 since C24's bars hang
    /// from an anchor: before, the crumbs' bar frame covered them from a drag's hit test).
    fn drop_on_a_crumb(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let (peer, from) = peer_beside(&s, display, &demo);
        select_tab(&demo, "main", 2);
        demo.command(
            "main",
            json!({ "cmd": "open", "tab": 2, "path": "Documents" }),
            "navigate",
        );
        let w = demo.window(&s, "main");
        let at = demo.at(&s, "main", json!({ "crumb": 0 }), 0.5, 0.5);
        let v = demo.command(
            "main",
            json!({ "cmd": "pick", "x": at.0 - w.content.x, "y": at.1 - w.content.y }),
            "picked",
        );
        assert!(
            v["path"]
                .as_str()
                .unwrap_or_default()
                .contains("loc-crumb-0"),
            "a drag's hit test reaches the crumb: {v}"
        );
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["Documents/plan.txt"],
            json!({ "crumb": 0 }),
        );
        let got = got.expect("a transfer into the root");
        assert_eq!(got["via"], "crumb", "{got}");
        assert_eq!(
            got["into"].as_str().map(PathBuf::from),
            Some(demo.root.clone())
        );
        demo.quit();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_drop_on_a_crumb() {
        drop_on_a_crumb(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_drop_on_a_crumb() {
        drop_on_a_crumb(Display::X11);
    }

    // ---- Springs and autoscroll ----

    /// A drag from the peer that rests on `target` for `rest`, then goes back to the peer and
    /// is dropped there (where nothing takes it). The demo's line count before it.
    fn rest_on(
        s: &Session,
        demo: &Demo,
        peer: &Peer,
        from: &Window,
        offers: &[Offer],
        target: (f64, f64),
        rest: Duration,
    ) -> usize {
        let mark = demo.app.line_count();
        press_on_peer(s, peer, from);
        peer.drag_start(offers, &BOTH);
        s.input().path(target);
        std::thread::sleep(Duration::from_millis(100));
        s.input().path((target.0 + 1.0, target.1));
        std::thread::sleep(rest);
        s.input().path(from.at(0.5, 0.5));
        std::thread::sleep(Duration::from_millis(150));
        s.input().button(false);
        peer.wait_event("the drag ending", |e| {
            matches!(e, Event::DragFinished { .. } | Event::DragCancelled)
        });
        mark
    }

    /// operations/drag-and-drop: "Holding a file drag over a folder row or sidebar row for
    /// 750 ms navigates there, and the drag stays held"; tabs: a file drag resting on a tab
    /// selects it (450 ms); column-autoscroll: "Holding a file drag within 44 px of the column
    /// strip's left or right edge scrolls the columns toward that edge", and "Only drags
    /// offering a file list or `text/uri-list` autoscroll".
    fn springs_and_autoscroll(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let (peer, from) = peer_beside(&s, display, &demo);
        let root = demo.root.clone();
        let offer = [files(&root, &["notes.txt"])];
        // A folder row in List.
        select_tab(&demo, "main", 2);
        let at = demo.at(&s, "main", row(2, "Documents"), 0.5, 0.5);
        let mark = rest_on(
            &s,
            &demo,
            &peer,
            &from,
            &offer,
            at,
            Duration::from_millis(1200),
        );
        let seen = demo.lines_from(mark);
        assert!(
            seen.iter()
                .any(|v| v["ev"] == "spring" && v["site"] == "row"),
            "{seen:?}"
        );
        assert!(
            seen.iter().any(|v| v["ev"] == "navigate"
                && v["folder"].as_str() == root.join("Documents").to_str())
        );
        assert!(
            !seen.iter().any(|v| v["ev"] == "transfer"),
            "the drag stayed held: {seen:?}"
        );
        // Too short a rest opens nothing (Strata's "leaving before 750 ms cancels").
        demo.command(
            "main",
            json!({ "cmd": "open", "tab": 2, "path": "" }),
            "navigate",
        );
        let at = demo.at(&s, "main", row(2, "Music"), 0.5, 0.5);
        let mark = rest_on(
            &s,
            &demo,
            &peer,
            &from,
            &offer,
            at,
            Duration::from_millis(300),
        );
        assert!(!demo.saw(mark, Duration::ZERO, |v| v["ev"] == "spring"));
        // A crumb (location-bar: "Holding a file drag over a parent breadcrumb for 750 ms
        // navigates there, and the drag stays held"), from Documents.
        demo.command(
            "main",
            json!({ "cmd": "open", "tab": 2, "path": "Documents" }),
            "navigate",
        );
        let at = demo.at(&s, "main", json!({ "crumb": 0 }), 0.5, 0.5);
        let mark = rest_on(
            &s,
            &demo,
            &peer,
            &from,
            &[files(&root, &["usb/stick.txt"])],
            at,
            Duration::from_millis(1200),
        );
        assert!(demo.saw(mark, Duration::ZERO, |v| v["ev"] == "crumb-open"
            && v["via"] == "Spring"
            && v["folder"].as_str() == root.to_str()));
        assert!(!demo.saw(mark, Duration::ZERO, |v| v["ev"] == "transfer"));
        // A sidebar place.
        let at = demo.at(&s, "main", json!({ "place": "Pictures" }), 0.5, 0.5);
        let mark = rest_on(
            &s,
            &demo,
            &peer,
            &from,
            &offer,
            at,
            Duration::from_millis(1200),
        );
        assert!(demo.saw(mark, Duration::ZERO, |v| v["ev"] == "spring"
            && v["site"] == "sidebar"));
        assert!(demo.saw(mark, Duration::ZERO, |v| v["ev"] == "navigate"
            && v["folder"].as_str() == root.join("Pictures").to_str()));
        // A tab (on a file from elsewhere: tab 3 shows the root, where notes.txt is).
        let at = demo.at(&s, "main", json!({ "tab": 3 }), 0.5, 0.5);
        let mark = rest_on(
            &s,
            &demo,
            &peer,
            &from,
            &[files(&root, &["usb/stick.txt"])],
            at,
            Duration::from_millis(900),
        );
        assert!(demo.saw(mark, Duration::ZERO, |v| v["ev"] == "tab-select"
            && v["tab"] == 3
            && v["via"] == "Spring"));
        // Columns: six columns deep overflow the strip, which shows the last; resting at its
        // left edge scrolls it back toward the first.
        select_tab(&demo, "main", 1);
        demo.command(
            "main",
            json!({ "cmd": "open", "tab": 1, "path": "Deep" }),
            "navigate",
        );
        for p in ["Deep/a", "Deep/a/b", "Deep/a/b/c", "Deep/a/b/c/d"] {
            demo.command(
                "main",
                json!({ "cmd": "open", "tab": 1, "path": p }),
                "navigate",
            );
        }
        let strip = demo.at(&s, "main", json!({ "tab": 1, "pane": true }), 0.0, 0.5);
        let first = || {
            demo.locate(
                &s,
                "main",
                &[json!({ "name": "c0", "tab": 1, "column": 0 })],
            )["c0"][0]
        };
        let before = first();
        // A drag offering only text doesn't scroll the strip.
        let text = [Offer::text("text/plain;charset=utf-8", "words")];
        rest_on(
            &s,
            &demo,
            &peer,
            &from,
            &text,
            (strip.0 + 10.0, strip.1),
            Duration::from_millis(1200),
        );
        assert!((first() - before).abs() < 1.0, "a text drag doesn't scroll");
        rest_on(
            &s,
            &demo,
            &peer,
            &from,
            &offer,
            (strip.0 + 10.0, strip.1),
            Duration::from_millis(1200),
        );
        let after = first();
        assert!(
            after > before + 20.0,
            "the strip scrolled toward the first column: {before} → {after}"
        );
        demo.quit();
    }

    /// column-autoscroll and List's edge scroll follow a file drag every target refuses, as
    /// Strata's motion controller does: `Deep/a` held at the strip's left edge, over columns
    /// inside it (each refuses it), scrolls the strip; `Music/track00.ogg` held at the bottom
    /// edge of List showing Music (its own folder) scrolls the list.
    fn refused_drags_autoscroll(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let (peer, from) = peer_beside(&s, display, &demo);
        let root = demo.root.clone();
        select_tab(&demo, "main", 1);
        for p in ["Deep", "Deep/a", "Deep/a/b", "Deep/a/b/c", "Deep/a/b/c/d"] {
            demo.command(
                "main",
                json!({ "cmd": "open", "tab": 1, "path": p }),
                "navigate",
            );
        }
        let strip = demo.at(&s, "main", json!({ "tab": 1, "pane": true }), 0.0, 0.5);
        let first = || {
            demo.locate(
                &s,
                "main",
                &[json!({ "name": "c0", "tab": 1, "column": 0 })],
            )["c0"][0]
        };
        let before = first();
        let mark = rest_on(
            &s,
            &demo,
            &peer,
            &from,
            &[files(&root, &["Deep/a"])],
            (strip.0 + 10.0, strip.1),
            Duration::from_millis(1500),
        );
        let after = first();
        assert!(
            after > before + 20.0,
            "a refused drag scrolls the strip: {before} → {after}"
        );
        assert!(!demo.saw(mark, Duration::ZERO, |v| v["ev"] == "transfer"));
        // List, in Music: the top row moves on under a refused drag at the bottom edge.
        select_tab(&demo, "main", 2);
        demo.command(
            "main",
            json!({ "cmd": "open", "tab": 2, "path": "Music" }),
            "navigate",
        );
        let w = demo.window(&s, "main");
        let pane = demo.locate(
            &s,
            "main",
            &[json!({ "name": "p", "tab": 2, "pane": true })],
        )["p"];
        let (px, py, ph) = (pane[0], pane[1], pane[3]);
        let top_row = || {
            let v = demo.command(
                "main",
                json!({
                    "cmd": "pick",
                    "x": px + 60.0 - w.content.x,
                    "y": py + 40.0 - w.content.y,
                }),
                "picked",
            );
            let path = v["path"].as_str().unwrap_or_default().to_owned();
            path.split('/')
                .find_map(|seg| seg.strip_prefix("row-")?.parse::<u64>().ok())
                .unwrap_or_else(|| panic!("no row at the top: {path}"))
        };
        let before = top_row();
        let mark = rest_on(
            &s,
            &demo,
            &peer,
            &from,
            &[files(&root, &["Music/track00.ogg"])],
            (px + 120.0, py + ph - 8.0),
            Duration::from_millis(1500),
        );
        std::thread::sleep(Duration::from_millis(300));
        let after = top_row();
        assert!(
            after > before,
            "a refused drag scrolls List: row {before} → {after}"
        );
        assert!(!demo.saw(mark, Duration::ZERO, |v| v["ev"] == "transfer"));
        demo.quit();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_refused_drags_autoscroll() {
        refused_drags_autoscroll(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_refused_drags_autoscroll() {
        refused_drags_autoscroll(Display::X11);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_springs_and_column_autoscroll() {
        springs_and_autoscroll(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_springs_and_column_autoscroll() {
        springs_and_autoscroll(Display::X11);
    }

    // ---- In process ----

    /// operations/drag-and-drop: a drag between Strata's own windows carries typed entries, not
    /// serialized URIs (plan §4, P1's fast path): a drop in the second window takes the
    /// payload with nothing read; tabs: "A folder-only drag" from Strata's own views dropped
    /// on New tab opens it in a new tab.
    fn between_two_windows(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, true, false);
        let _ = s.wait_windows(&[demo.app.pid()]);
        for key in ["main", "second"] {
            select_tab(&demo, key, 2);
        }
        let to = demo.at(&s, "second", row(2, "Music"), 0.5, 0.5);
        let (mark, end) = drag_out(&s, &demo, "main", row(2, "notes.txt"), to);
        assert_eq!(end["outcome"], "moved", "{end}");
        let (_, got) = demo.wait(mark, "the second window's transfer", |v| {
            v["ev"] == "transfer" && v["window"] == "second"
        });
        assert_eq!(got["local"], true, "{got}");
        assert_eq!(got["types"], json!([]), "nothing read: {got}");
        assert_eq!(got["sources"], json!(["notes.txt"]));
        assert_eq!(got["action"], "move");
        // A folder from main's List onto second's New tab: a new tab there.
        let to = demo.at(&s, "second", json!({ "new-tab": true }), 0.5, 0.5);
        let (mark, end) = drag_out(&s, &demo, "main", row(2, "Pictures"), to);
        assert_ne!(end["outcome"], "cancelled", "{end}");
        let (_, opened) = demo.wait(mark, "a tab opening in second", |v| {
            v["ev"] == "tab-open" && v["window"] == "second"
        });
        assert_eq!(
            opened["folder"].as_str(),
            demo.root.join("Pictures").to_str()
        );
        demo.quit();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_between_two_windows_takes_the_typed_payload() {
        between_two_windows(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_between_two_windows_takes_the_typed_payload() {
        between_two_windows(Display::X11);
    }

    // ---- Actions, Escape, peeks, pins ----

    /// copy-or-move: "Holding Ctrl at the drop copies and holding Shift moves, on the same or
    /// another device", where the platform says (X11 in both directions; on Wayland a drag
    /// from another client brings none, P1's limit, and the plain rule applies); the source's
    /// half: Ctrl or Shift at a drag's start narrows what it offers (P1).
    fn modifiers(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let (peer, from) = peer_beside(&s, display, &demo);
        let root = demo.root.clone();
        select_tab(&demo, "main", 2);
        let drop_with = |held: &str, names: &[&str], target: Value| {
            let at = demo.at(&s, "main", target, 0.5, 0.5);
            let mark = demo.app.line_count();
            press_on_peer(&s, &peer, &from);
            peer.drag_start(&[files(&root, names)], &BOTH);
            s.input().key_down(held);
            s.input().path(at);
            std::thread::sleep(Duration::from_millis(150));
            s.input().path((at.0 + 1.0, at.1));
            std::thread::sleep(Duration::from_millis(200));
            s.input().button(false);
            s.input().key_up(held);
            peer.wait_event("the drag ending", |e| {
                matches!(e, Event::DragFinished { .. } | Event::DragCancelled)
            });
            demo.wait(mark, "the transfer", |v| v["ev"] == "transfer").1
        };
        let ctrl = drop_with("ctrl", &["notes.txt"], row(2, "Documents"));
        let shift = drop_with("shift", &["usb/stick.txt"], row(2, "Music"));
        match display {
            Display::X11 => {
                assert_eq!(ctrl["action"], "copy", "Ctrl copies on one volume: {ctrl}");
                assert_eq!(
                    shift["action"], "move",
                    "Shift moves across devices: {shift}"
                );
            }
            _ => {
                assert_eq!(ctrl["action"], "move", "no modifiers on Wayland: {ctrl}");
                assert_eq!(shift["action"], "ask", "no modifiers on Wayland: {shift}");
            }
        }
        // The source's half, both platforms: the modifiers at the start.
        peer.accept_drops_with(&[URI_LIST], &BOTH, Action::Move, false, None);
        let to = from.at(0.5, 0.5);
        for (held, want) in [("ctrl", "copied"), ("shift", "moved")] {
            s.input().key_down(held);
            let (_, end) = drag_out(&s, &demo, "main", row(2, "todo.txt"), to);
            s.input().key_up(held);
            assert_eq!(end["outcome"], want, "{held}: {end}");
            let _ = dropped_on_peer(&peer, 1);
        }
        demo.quit();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_actions_by_modifier() {
        modifiers(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_actions_by_modifier() {
        modifiers(Display::X11);
    }

    /// tabs: "Escape during a file drag in Columns, List, or Icons… cancels it" (X11: Rune hears
    /// the key; on Wayland the compositor's grab keeps it, and sway doesn't cancel, P1's
    /// limit).
    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_escape_cancels_a_drag_out() {
        let s = Session::start();
        let demo = Demo::start(&s, Display::X11, false, false);
        let (peer, to) = peer_beside(&s, Display::X11, &demo);
        peer.accept_drops_with(&[URI_LIST], &BOTH, Action::Copy, false, None);
        select_tab(&demo, "main", 2);
        let from = demo.at(&s, "main", row(2, "notes.txt"), 0.1, 0.5);
        let mark = press(&s, &demo, "main", from);
        s.input().path(((from.0 + to.at(0.5, 0.5).0) / 2.0, from.1));
        demo.wait(mark, "the drag starting", |v| v["ev"] == "drag-start");
        s.input().key("Escape");
        let (_, end) = demo.wait(mark, "the outcome", |v| v["ev"] == "drag-end");
        assert_eq!(end["outcome"], "cancelled", "{end}");
        s.input().path(to.at(0.5, 0.5));
        s.input().button(false);
        assert!(!demo.saw(mark, Duration::from_millis(500), |v| v["ev"] == "transfer"));
        demo.quit();
    }

    /// operations/drag-and-drop: "Starting a drag closes any open folder peek" (C22's host
    /// duty), and a drag dropped on a folder never peeks it after (`hold_rest`).
    fn peeks(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let (peer, from) = peer_beside(&s, display, &demo);
        select_tab(&demo, "main", 2);
        demo.focus(&s, "main");
        let docs = demo.at(&s, "main", row(2, "Documents"), 0.3, 0.5);
        let mark = demo.app.line_count();
        s.input().pointer_move((docs.0 + 4.0, docs.1));
        s.input().pointer_move(docs);
        demo.wait(mark, "a peek opening", |v| {
            v["ev"] == "peek" && v["open"] == true
        });
        // A drag from the folder it peeks closes it (moving to another row would close it
        // anyway, after the leave delay).
        peer.accept_drops_with(&[URI_LIST], &BOTH, Action::Copy, false, None);
        let (mark, _) = drag_out(&s, &demo, "main", row(2, "Documents"), from.at(0.5, 0.5));
        assert!(demo.saw(mark, Duration::ZERO, |v| v["ev"] == "peek"
            && v["open"] == false
            && v["why"] == "drag"));
        let _ = dropped_on_peer(&peer, 1);
        // A drop from the peer on Music, the pointer left resting there: Music doesn't peek.
        // (Rows the pointer rests on on its way to the peer before the drag may; on X11 the
        // pointer waits over the list while the peer takes focus.)
        let mark = demo.app.line_count();
        let got = drop_on(&s, &demo, &peer, &from, &["todo.txt"], row(2, "Music"));
        assert!(got.is_some());
        let music = demo.root.join("Music");
        std::thread::sleep(Duration::from_millis(1200));
        let lines = demo.lines_from(mark);
        let dropped = lines
            .iter()
            .position(|v| v["ev"] == "transfer")
            .expect("the drop");
        let peeked = lines[dropped..].iter().any(|v| {
            v["ev"] == "peek" && v["open"] == true && v["folder"].as_str() == music.to_str()
        });
        assert!(
            !peeked,
            "a folder a drag was dropped on doesn't peek: {lines:?}"
        );
        demo.quit();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_a_drag_closes_the_peek() {
        peeks(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_a_drag_closes_the_peek() {
        peeks(Display::X11);
    }

    /// pins/drag-to-pin: folders dropped in a PINNED gap are pinned there; a gap shows nothing
    /// for a drag that isn't all folders, checked while it hovers (its `text/uri-list` read
    /// before the drop), so a file is refused.
    fn pin_gaps(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let (peer, from) = peer_beside(&s, display, &demo);
        let root = demo.root.clone();
        for (names, pins) in [(vec!["Music"], true), (vec!["notes.txt"], false)] {
            // The sidebar lays its gaps out again while a drag is over it: find the gap then.
            let near = demo.at(&s, "main", json!({ "place": "Music" }), 0.5, 0.5);
            let mark = demo.app.line_count();
            press_on_peer(&s, &peer, &from);
            peer.drag_start(&[files(&root, &names)], &BOTH);
            s.input().path(near);
            std::thread::sleep(Duration::from_millis(300));
            let at = demo.at(&s, "main", json!({ "gap": 0 }), 0.5, 0.5);
            s.input().path(at);
            // A checked folder drag opens the gap, which moves it: find it again.
            std::thread::sleep(Duration::from_millis(400));
            let at = demo.at(&s, "main", json!({ "gap": 0 }), 0.5, 0.5);
            s.input().path(at);
            std::thread::sleep(Duration::from_millis(250));
            s.input().path((at.0 + 1.0, at.1));
            std::thread::sleep(Duration::from_millis(250));
            s.input().button(false);
            peer.wait_event("the drag ending", |e| {
                matches!(e, Event::DragFinished { .. } | Event::DragCancelled)
            });
            let pinned = demo.saw(mark, Duration::from_millis(800), |v| v["ev"] == "pin");
            assert_eq!(pinned, pins, "{names:?}");
        }
        let st = demo.command("main", json!({ "cmd": "state" }), "state");
        assert_eq!(st["pins"], json!(["Music", "pinned"]), "{st}");
        demo.quit();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_pin_gaps_take_folders_only() {
        pin_gaps(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_pin_gaps_take_folders_only() {
        pin_gaps(Display::X11);
    }

    /// C1f's stuck group header takes no input, so a drop over it goes to the folder the list
    /// shows, not to the row it covers (List's "Dropping onto the background of a pane…"); just
    /// under it, the folder row there takes the drop. Under P1 the drag's hit test finds the
    /// stuck header itself, inside the collection but in no row, so the background zone
    /// takes it by path: C1e's drop-by-point routing in `row.rs` (for binders that route
    /// through the header to the row under it, Rune's) isn't reached.
    fn stuck_header(display: Display) {
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let (peer, from) = peer_beside(&s, display, &demo);
        let root = demo.root.clone();
        select_tab(&demo, "main", 2);
        demo.command(
            "main",
            json!({ "cmd": "open", "tab": 2, "path": "Mixed" }),
            "navigate",
        );
        demo.command("main", json!({ "cmd": "group", "tab": 2 }), "grouped");
        demo.command(
            "main",
            json!({ "cmd": "scroll", "tab": 2, "y": 130.0 }),
            "scrolled",
        );
        let got = drop_on(
            &s,
            &demo,
            &peer,
            &from,
            &["notes.txt"],
            json!({ "tab": 2, "stuck": true }),
        );
        let got = got.expect("a drop over the stuck header");
        assert_eq!(got["via"], "background", "{got}");
        assert_eq!(
            got["into"].as_str().map(PathBuf::from),
            Some(root.join("Mixed"))
        );
        // A visible folder row under the header takes its own drops.
        let got = drop_on(&s, &demo, &peer, &from, &["notes.txt"], row(2, "folder10"));
        let got = got.expect("a drop on a folder row");
        assert_eq!(got["via"], "row", "{got}");
        assert_eq!(
            got["into"].as_str().map(PathBuf::from),
            Some(root.join("Mixed/folder10"))
        );
        demo.quit();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_a_drop_over_the_stuck_header_goes_to_the_folder_shown() {
        stuck_header(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_a_drop_over_the_stuck_header_goes_to_the_folder_shown() {
        stuck_header(Display::X11);
    }

    // ---- Nautilus (opt-in, best effort) ----

    /// Nautilus in the nested sway, both ways: a file dragged from Nautilus onto a List
    /// folder row transfers into it (Nautilus's GTK drag reaches the demo through P1), and a
    /// file dragged from the demo onto Nautilus's folder lands there. Best effort: Nautilus's
    /// layout isn't known, so its file is looked for at a few likely points. Opt-in, with
    /// Nautilus installed: `STRATA_TEST_NAUTILUS=1 mise run test-platform x2::nautilus`.
    fn nautilus_both_ways(display: Display) {
        if std::env::var_os("STRATA_TEST_NAUTILUS").is_none() {
            let test = match display {
                Display::Wayland => "nautilus_wayland_drags_both_ways",
                _ => "nautilus_x11_drags_both_ways",
            };
            skip(test, "set STRATA_TEST_NAUTILUS=1 to run the Nautilus drags");
            return;
        }
        let s = Session::start();
        let demo = Demo::start(&s, display, false, false);
        let root = demo.root.clone();
        let mut cmd = s.command("nautilus", display);
        cmd.env("GSK_RENDERER", "cairo")
            .env("GTK_A11Y", "none")
            .env(
                "GDK_BACKEND",
                if display == Display::X11 {
                    "x11"
                } else {
                    "wayland"
                },
            )
            .arg("--new-window")
            .arg(root.join("usb"));
        let nautilus = s.spawn("nautilus", cmd);
        let window = s.wait_window(nautilus.pid());
        let _ = s.wait_windows(&[nautilus.pid(), demo.app.pid()]);
        let window = s.wait_window_where("Nautilus", |w| w.id == window.id);
        std::thread::sleep(Duration::from_millis(2500));
        select_tab(&demo, "main", 2);
        // Nautilus → the demo: stick.txt, the folder's only file, onto Music.
        let to = demo.at(&s, "main", row(2, "Music"), 0.5, 0.5);
        let mut landed = None;
        'points: for fy in [0.18, 0.25, 0.32] {
            for fx in [0.08, 0.15, 0.3, 0.45] {
                let at = window.at(fx, fy);
                s.focus(&window);
                std::thread::sleep(Duration::from_millis(300));
                s.input().pointer_move(at);
                std::thread::sleep(Duration::from_millis(200));
                let mark = demo.app.line_count();
                s.input().button(true);
                // Past GTK's threshold, and time for Nautilus to start its drag.
                s.input().path((at.0 + 20.0, at.1 + 10.0));
                std::thread::sleep(Duration::from_millis(300));
                s.input().path(to);
                std::thread::sleep(Duration::from_millis(500));
                s.input().path((to.0 + 1.0, to.1));
                std::thread::sleep(Duration::from_millis(400));
                s.input().button(false);
                if demo.saw(mark, Duration::from_secs(2), |v| v["ev"] == "transfer") {
                    landed = demo
                        .lines_from(mark)
                        .into_iter()
                        .find(|v| v["ev"] == "transfer");
                    break 'points;
                }
            }
        }
        // Both ways run before either is judged, so a failure says which.
        let inward = landed.filter(|got| {
            got["into"].as_str().map(PathBuf::from) == Some(root.join("Music"))
                && got["sources"] == json!(["stick.txt"])
                && got["local"] == false
        });
        eprintln!("Nautilus → demo: {inward:?}");
        // The demo → Nautilus: notes.txt into the folder Nautilus shows (usb, another volume
        // to the demo; Nautilus decides its own action).
        // (The drag above may have sprung List into Music.)
        demo.command(
            "main",
            json!({ "cmd": "open", "tab": 2, "path": "" }),
            "navigate",
        );
        let to = window.at(0.6, 0.7);
        let (_, end) = drag_out(&s, &demo, "main", row(2, "notes.txt"), to);
        let deadline = Instant::now() + timeout();
        while !root.join("usb/notes.txt").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        let outward = root.join("usb/notes.txt").exists();
        eprintln!("demo → Nautilus: {end}, written: {outward}");
        drop(nautilus);
        demo.quit();
        assert!(
            inward.is_some(),
            "no drag from Nautilus reached the demo's Music row"
        );
        assert!(outward, "Nautilus didn't write notes.txt into usb: {end}");
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn nautilus_wayland_drags_both_ways() {
        nautilus_both_ways(Display::Wayland);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn nautilus_x11_drags_both_ways() {
        nautilus_both_ways(Display::X11);
    }
}

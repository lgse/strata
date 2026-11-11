//! X3's session tests: the clipboard wiring demo (`examples/clipboard_wiring/`) against W3's
//! peer in a private headless sway, on Wayland (data control, and the data device as on
//! mutter) and X11 (`mise run test-platform x3::`).
//!
//! Each test starts the demo with `--apply` on a fixture folder in the session's private
//! runtime directory, sets it up with its commands (`open`, `select`), and presses the
//! clipboard's keys with the session's virtual keyboard. What the demo did comes back as its
//! JSON lines (`copied`, `badge`, `paste`, `state`), what it offered as the peer's reads, and
//! what a paste wrote on disk.
//!
//! Work package: X3, `docs/work/phase-3-views/X3-clipboard-wiring.md`.

#[path = "../../strata-platform/tests/support/mod.rs"]
#[expect(
    dead_code,
    reason = "W3's harness, shared with strata-platform's session tests: X3 uses part of it"
)]
#[expect(
    unexpected_cfgs,
    unused_imports,
    reason = "the harness gates helpers on strata-platform's features, which this crate lacks \
              (so X3 uses none of those helpers: their expectations hold)"
)]
mod support;

mod x3 {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};
    use strata_platform::clipboard::files::file_uri;
    use strata_test_peer::protocol::{Offer, Payload, Selection, Via, fnv1a64};

    use crate::support::{Display, Peer, Proc, Session, Window, example, timeout};

    const GNOME: &str = "x-special/gnome-copied-files";
    const URIS: &str = "text/uri-list";
    const KDE: &str = "application/x-kde-cutselection";
    const TEXT: [&str; 5] = [
        "text/plain;charset=utf-8",
        "text/plain",
        "UTF8_STRING",
        "TEXT",
        "STRING",
    ];

    /// Where the demo runs: Wayland with data control (sway's), Wayland with the data device
    /// (as on mutter), or X11.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum B {
        W,
        D,
        X,
    }

    impl B {
        fn display(self) -> Display {
            match self {
                B::W | B::D => Display::Wayland,
                B::X => Display::X11,
            }
        }

        /// How the peer reads and writes: data control needs no focus, so it serves D too.
        fn via(self) -> Via {
            match self {
                B::W | B::D => Via::DataControl,
                B::X => Via::X11,
            }
        }

        fn route(self) -> &'static str {
            match self {
                B::W => "DataControl",
                B::D => "DataDevice",
                B::X => "X11",
            }
        }
    }

    /// The fixture: `src` with files to paste from, `dest` (with `inner`) to paste into, and
    /// `Music` holding a `notes.txt` of its own.
    fn fixture(s: &Session) -> PathBuf {
        let root = s.runtime_dir().join("root");
        for d in ["src", "dest/inner", "Music"] {
            std::fs::create_dir_all(root.join(d)).expect("fixture folder");
        }
        for (f, body) in [
            ("a b.txt", "a b"),
            ("notes.txt", "notes"),
            ("todo.txt", "todo"),
            ("Music/notes.txt", "music's"),
        ] {
            std::fs::write(root.join(f), body).expect("fixture file");
        }
        root.canonicalize().expect("the root")
    }

    struct Demo {
        app: Proc,
        window: Window,
        root: PathBuf,
    }

    impl Demo {
        fn start(s: &Session, b: B) -> Demo {
            Demo::start_on(s, b, fixture(s))
        }

        fn start_on(s: &Session, b: B, root: PathBuf) -> Demo {
            let mut cmd = s.command(example("clipboard_wiring"), b.display());
            cmd.arg("--root")
                .arg(&root)
                .args(["--apply", "--title", "x3 clipboard"]);
            if b == B::D {
                cmd.arg("--as-if-no-data-control");
            }
            let app = s.spawn("clipboard_wiring", cmd);
            let attached = app.wait_json("attach", |v| {
                v["ev"] == "attached" || v["ev"] == "attach-error"
            });
            assert_eq!(attached["ev"], "attached", "{attached}");
            assert_eq!(attached["route"], b.route(), "{attached}");
            app.wait_json("the first frame", |v| v["ev"] == "frame");
            let window = s.wait_window(app.pid());
            Demo { app, window, root }
        }

        fn send(&self, cmd: Value) -> usize {
            let mark = self.app.line_count();
            self.app.send_line(&cmd.to_string());
            mark
        }

        fn wait(&self, from: usize, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
            self.app.wait_json_from(from, what, pred).1
        }

        fn lines_from(&self, from: usize) -> Vec<Value> {
            self.app
                .lines()
                .iter()
                .skip(from)
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .collect()
        }

        fn open(&self, rel: &str) {
            let at = self.send(json!({ "cmd": "open", "path": rel }));
            self.wait(at, &format!("opening {rel}"), |v| v["ev"] == "opened");
        }

        fn select(&self, names: &[&str]) {
            let at = self.send(json!({ "cmd": "select", "names": names }));
            self.wait(at, "the selection", |v| v["ev"] == "selected");
        }

        fn state(&self) -> Value {
            let at = self.send(json!({ "cmd": "state" }));
            self.wait(at, "the state", |v| v["ev"] == "state")
        }

        /// Focuses the window (once it has said so, keys reach it), then presses `chord`.
        fn key(&self, s: &Session, chord: &str) -> usize {
            let focused = |v: &Value| v["ev"] == "focus";
            let last = self.lines_from(0).into_iter().rfind(focused);
            if last.is_none_or(|v| v["focused"] != true) {
                let at = self.app.line_count();
                s.focus(&self.window);
                self.wait(at, "keyboard focus", |v| {
                    v["ev"] == "focus" && v["focused"] == true
                });
            }
            let at = self.app.line_count();
            s.input().key(chord);
            at
        }

        /// Ctrl+C or Ctrl+X on `names`; the copy's line.
        fn copy(&self, s: &Session, names: &[&str], cut: bool) -> Value {
            self.select(names);
            let at = self.key(s, if cut { "ctrl+x" } else { "ctrl+c" });
            let v = self.wait(at, "the copy", |v| {
                v["ev"] == "copied" || v["ev"] == "copy-error"
            });
            assert_eq!(v["ev"], "copied", "{v}");
            v
        }

        /// Ctrl+V; the paste's outcome line.
        fn paste(&self, s: &Session) -> Value {
            let at = self.key(s, "ctrl+v");
            self.wait(at, "the paste", |v| {
                ["paste", "paste-refused", "paste-nothing", "paste-error"]
                    .contains(&v["ev"].as_str().unwrap_or_default())
            })
        }

        /// Waits for the badge to turn `on` from line `from` on.
        fn badge(&self, from: usize, on: bool) {
            self.wait(from, &format!("the badge {on}"), |v| {
                v["ev"] == "badge" && v["on"] == on
            });
        }

        fn badge_is(&self) -> bool {
            self.lines_from(0)
                .into_iter()
                .rfind(|v| v["ev"] == "badge")
                .is_some_and(|v| v["on"] == true)
        }

        /// The peer copies `offers`, and the demo has heard it: the badge shows for a file
        /// list, else is off. A badge already showing is first turned off by a text copy, so
        /// that its turning on again means this offer was heard.
        fn peer_copies(&self, peer: &Peer, via: Via, offers: &[Offer], files: bool) {
            if files && self.badge_is() {
                let at = self.app.line_count();
                peer.set_selection(Selection::Clipboard, via, &[Offer::text(TEXT[0], "x")]);
                self.badge(at, false);
            }
            let at = self.app.line_count();
            peer.set_selection(Selection::Clipboard, via, offers);
            if files || self.badge_is() {
                self.badge(at, files);
            }
        }

        fn no_errors(&self) {
            let bad: Vec<Value> = self
                .lines_from(0)
                .into_iter()
                .filter(|v| {
                    [
                        "copy-error",
                        "paste-error",
                        "cut-consumed-error",
                        "bad-command",
                        "exit-error",
                    ]
                    .contains(&v["ev"].as_str().unwrap_or_default())
                })
                .collect();
            assert!(bad.is_empty(), "the demo reported errors: {bad:?}");
        }
    }

    fn uri(p: &Path) -> String {
        file_uri(p)
    }

    /// Polls the peer until its `via` clipboard offers `want` (None: nothing).
    fn wait_targets(peer: &Peer, via: Via, want: Option<&[&str]>) -> Vec<String> {
        let deadline = Instant::now() + timeout();
        loop {
            peer.sync();
            let got = peer.targets(Selection::Clipboard, via).unwrap_or_default();
            let done = match want {
                Some(want) => want.iter().all(|m| got.iter().any(|g| g == m)),
                None => got.is_empty(),
            };
            if done {
                return got;
            }
            assert!(
                Instant::now() < deadline,
                "the peer's clipboard offers {got:?}, not {want:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Polls the peer's read of `mime` until it reads `want`.
    fn wait_read(peer: &Peer, via: Via, mime: &str, want: &str) {
        let deadline = Instant::now() + timeout();
        loop {
            let got = peer.read(Selection::Clipboard, via, mime).text;
            if got.as_deref() == Some(want) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{mime} reads {got:?}, not {want:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    // ---- Copy and cut: what the peer reads ----

    fn copy_and_cut_offer_the_three_formats(b: B) {
        let s = Session::start();
        let demo = Demo::start(&s, b);
        let peer = s.peer(b.display());
        let (ab, notes) = (demo.root.join("a b.txt"), demo.root.join("notes.txt"));
        // operations/clipboard/file-manager-formats: Ctrl+C or Ctrl+X on a selection offers a GDK file list, `text/uri-list`, `x-special/gnome-copied-files`, and `application/x-kde-cutselection` together.
        let at = demo.app.line_count();
        demo.copy(&s, &["a b.txt", "notes.txt"], false);
        let offered = wait_targets(&peer, b.via(), Some(&[GNOME, URIS, KDE]));
        assert!(
            !offered.iter().any(|m| TEXT.contains(&m.as_str())),
            "no text, as Strata: {offered:?}"
        );
        // operations/clipboard/file-manager-formats: After Ctrl+C, the GNOME payload is `copy` followed by one URI per line, with no trailing newline, and the KDE flag is `0`.
        wait_read(
            &peer,
            b.via(),
            GNOME,
            &format!("copy\n{}\n{}", uri(&ab), uri(&notes)),
        );
        // operations/clipboard/file-manager-formats: The `text/uri-list` payload holds one URI per line, each ending in CRLF.
        wait_read(
            &peer,
            b.via(),
            URIS,
            &format!("{}\r\n{}\r\n", uri(&ab), uri(&notes)),
        );
        wait_read(&peer, b.via(), KDE, "0");
        demo.badge(at, true);
        let st = demo.state();
        assert_eq!(
            st["marks"],
            json!({ "a b.txt": "copied", "notes.txt": "copied" }),
            "{st}"
        );
        // operations/clipboard/file-manager-formats: After Ctrl+X, the GNOME payload starts with `cut` and the KDE flag is `1`.
        demo.copy(&s, &["notes.txt"], true);
        wait_read(&peer, b.via(), GNOME, &format!("cut\n{}", uri(&notes)));
        wait_read(&peer, b.via(), KDE, "1");
        wait_read(&peer, b.via(), URIS, &format!("{}\r\n", uri(&notes)));
        let st = demo.state();
        assert_eq!(st["marks"], json!({ "notes.txt": "cut" }), "{st}");
        assert_eq!(st["badge"], true, "{st}");
        // operations/clipboard: Ctrl+X or the item menu's Cut leaves the selection in place until a paste moves it.
        assert!(notes.exists(), "a cut touches nothing before the paste");
        demo.no_errors();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_copy_and_cut_offer_the_three_formats() {
        copy_and_cut_offer_the_three_formats(B::W);
    }

    /// That the data-device route carries the copy and its formats (`--as-if-no-data-control`
    /// attaches as on mutter). It doesn't show that the copy runs inside the key's handler:
    /// wlroots takes any serial it has sent the client, so a later write would pass too.
    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_data_device_copy_and_cut_offer_the_three_formats() {
        copy_and_cut_offer_the_three_formats(B::D);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_copy_and_cut_offer_the_three_formats() {
        copy_and_cut_offer_the_three_formats(B::X);
    }

    // ---- Another app's copy: marks and the badge ----

    fn another_apps_copy(b: B) {
        let s = Session::start();
        let demo = Demo::start(&s, b);
        let peer = s.peer(b.display());
        let at = demo.app.line_count();
        demo.copy(&s, &["notes.txt"], true);
        demo.badge(at, true);
        // operations/clipboard: When another application takes the clipboard, copy and cut marks clear in every window.
        let at = demo.app.line_count();
        peer.set_selection(
            Selection::Clipboard,
            b.via(),
            &[Offer::text(TEXT[0], "the peer's")],
        );
        demo.badge(at, false);
        let st = demo.state();
        assert_eq!(st["marks"], json!({}), "{st}");
        // Strata's badge rule: another app's file list shows it too; an empty one doesn't.
        let todo = demo.root.join("todo.txt");
        let at = demo.app.line_count();
        peer.set_selection(
            Selection::Clipboard,
            b.via(),
            &[Offer::text(URIS, &format!("{}\r\n", uri(&todo)))],
        );
        demo.badge(at, true);
        assert_eq!(
            demo.state()["marks"],
            json!({}),
            "another app's files mark nothing"
        );
        let at = demo.app.line_count();
        peer.set_selection(Selection::Clipboard, b.via(), &[Offer::text(URIS, "\r\n")]);
        demo.badge(at, false);
        demo.no_errors();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_another_apps_copy_clears_the_marks_and_its_files_show_the_badge() {
        another_apps_copy(B::W);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_another_apps_copy_clears_the_marks_and_its_files_show_the_badge() {
        another_apps_copy(B::X);
    }

    // ---- Pasting another app's files ----

    fn gnome(op: &str, p: &Path) -> Offer {
        Offer::text(GNOME, &format!("{op}\n{}", uri(p)))
    }

    fn uri_list(p: &Path) -> Offer {
        Offer::text(URIS, &format!("{}\r\n", uri(p)))
    }

    fn paste_reads_each_foreign_format(b: B) {
        let s = Session::start();
        let demo = Demo::start(&s, b);
        let peer = s.peer(b.display());
        let (src, dest) = (demo.root.join("src"), demo.root.join("dest"));
        demo.open("dest");
        // (case, offers, moves): each with a file of its own in `src`.
        let kde = |flag: &str| Offer::text(KDE, flag);
        type Offers = fn(&Path, &dyn Fn(&str) -> Offer) -> Vec<Offer>;
        let cases: [(&str, Offers, bool); 7] = [
            // operations/clipboard/file-manager-formats: Ctrl+V on a clipboard whose `x-special/gnome-copied-files` starts with `cut` moves the files; one starting with `copy` copies them.
            (
                "gnome-copy",
                |p, _| vec![gnome("copy", p), uri_list(p)],
                false,
            ),
            ("gnome-cut", |p, _| vec![gnome("cut", p), uri_list(p)], true),
            // operations/clipboard/file-manager-formats: Ctrl+V on a `text/uri-list` clipboard with `application/x-kde-cutselection` set to `1` moves the files. With `0`, or with no KDE flag, it copies them.
            ("kde-cut", |p, k| vec![uri_list(p), k("1")], true),
            ("kde-copy", |p, k| vec![uri_list(p), k("0")], false),
            ("uri-list", |p, _| vec![uri_list(p)], false),
            // operations/clipboard/file-manager-formats: A clipboard offering only `x-special/gnome-copied-files`, with no file list or `text/uri-list`, still pastes on Ctrl+V.
            ("gnome-only", |p, _| vec![gnome("copy", p)], false),
            // operations/clipboard/file-manager-formats: When both formats are offered, a GNOME payload whose first line is `cut` or `copy` decides, whatever the KDE flag says.
            (
                "gnome-decides",
                |p, k| vec![gnome("copy", p), uri_list(p), k("1")],
                false,
            ),
        ];
        for (case, offers, moves) in cases {
            let file = src.join(format!("{case}.txt"));
            std::fs::write(&file, case).expect("a source");
            demo.peer_copies(&peer, b.via(), &offers(&file, &kde), true);
            let v = demo.paste(&s);
            assert_eq!(v["ev"], "paste", "{case}: {v}");
            assert_eq!(v["moving"], moves, "{case}: {v}");
            let landed = dest.join(format!("{case}.txt"));
            assert_eq!(
                std::fs::read_to_string(&landed).ok().as_deref(),
                Some(case),
                "{case}: {v}"
            );
            // operations/clipboard: Ctrl+C or the item menu's Copy puts the selection on the clipboard in the formats `operations/clipboard/file-manager-formats` lists; pasting copies it and leaves the sources in place.
            assert_eq!(file.exists(), !moves, "{case}: the source");
            let st = demo.state();
            assert_eq!(
                st["selected"],
                json!([format!("dest/{case}.txt")]),
                "{case}: {st}"
            );
        }
        // Into the one folder selected: the demo then shows it.
        let file = src.join("into-inner.txt");
        std::fs::write(&file, "inner").expect("a source");
        demo.peer_copies(&peer, b.via(), &[uri_list(&file)], true);
        demo.select(&["inner"]);
        let v = demo.paste(&s);
        assert_eq!(v["dest"], "dest/inner", "{v}");
        assert!(dest.join("inner/into-inner.txt").exists());
        assert_eq!(demo.state()["folder"], "dest/inner");
        // A source outside the root refuses the whole paste.
        let outside = s.runtime_dir().join("outside.txt");
        std::fs::write(&outside, "outside").expect("a file outside");
        let inside = src.join("beside.txt");
        std::fs::write(&inside, "beside").expect("a source");
        let both = format!("{}\r\n{}\r\n", uri(&inside), uri(&outside));
        demo.peer_copies(&peer, b.via(), &[Offer::text(URIS, &both)], true);
        let v = demo.paste(&s);
        assert_eq!(v["ev"], "paste-refused", "{v}");
        assert!(
            !dest.join("inner/beside.txt").exists() && !dest.join("inner/outside.txt").exists()
        );
        // Text alone pastes nothing.
        demo.peer_copies(&peer, b.via(), &[Offer::text(TEXT[0], "words")], false);
        assert_eq!(demo.paste(&s)["ev"], "paste-nothing");
        demo.no_errors();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_paste_reads_each_foreign_format() {
        paste_reads_each_foreign_format(B::W);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_paste_reads_each_foreign_format() {
        paste_reads_each_foreign_format(B::X);
    }

    // ---- Our own cut and copy, pasted ----

    fn cut_then_paste(b: B) {
        let s = Session::start();
        let demo = Demo::start(&s, b);
        let peer = s.peer(b.display());
        let root = demo.root.clone();
        // A cut's paste empties the clipboard of what it moved. (That a newer copy stays, the
        // other half of the operations/clipboard bullet, is the Board's: adapter/tests.rs.)
        demo.copy(&s, &["todo.txt"], true);
        wait_targets(&peer, b.via(), Some(&[GNOME]));
        demo.open("Music");
        let at = demo.app.line_count();
        let v = demo.paste(&s);
        assert_eq!(
            (&v["moving"], &v["written"]),
            (&json!(true), &json!(["todo.txt"])),
            "{v}"
        );
        assert!(root.join("Music/todo.txt").exists() && !root.join("todo.txt").exists());
        let consumed = demo.wait(at, "the cut consumed", |v| v["ev"] == "cut-consumed");
        assert_eq!(consumed["clipboard_changed"], true, "{consumed}");
        wait_targets(&peer, b.via(), None);
        demo.badge(at, false);
        let st = demo.state();
        assert_eq!(
            (&st["marks"], &st["selected"]),
            (&json!({}), &json!(["Music/todo.txt"])),
            "{st}"
        );
        // operations/clipboard/conflicts: Keep Both creates the first free `name (N).ext` beside the existing item and leaves it untouched.
        demo.open("");
        demo.copy(&s, &["notes.txt"], false);
        demo.open("Music");
        let at = demo.app.line_count();
        let v = demo.paste(&s);
        assert_eq!(v["written"], json!(["notes (1).txt"]), "{v}");
        let conflict = demo.wait(at, "the conflict", |v| v["ev"] == "conflict");
        assert_eq!(conflict["kept_as"], "notes (1).txt", "{conflict}");
        assert_eq!(
            std::fs::read_to_string(root.join("Music/notes.txt")).expect("Music/notes.txt"),
            "music's"
        );
        // operations/clipboard: Pasting a copied file into its own folder creates `name (1).ext`, then `name (2).ext`, with no dialog.
        demo.open("");
        demo.select(&["notes.txt"]);
        let v = demo.paste(&s);
        assert_eq!(v["written"], json!(["notes (1).txt"]), "{v}");
        assert!(root.join("notes (1).txt").exists() && root.join("notes.txt").exists());
        // operations/clipboard: Cut and paste into the same folder does nothing.
        demo.copy(&s, &["notes.txt"], true);
        let v = demo.paste(&s);
        assert_eq!(v["written"], json!([]), "{v}");
        assert_eq!(
            v["skipped"],
            json!([["notes.txt", "its own folder"]]),
            "{v}"
        );
        assert!(root.join("notes.txt").exists() && !root.join("notes (2).txt").exists());
        assert_eq!(
            demo.state()["marks"],
            json!({ "notes.txt": "cut" }),
            "the cut stays"
        );
        demo.no_errors();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_cut_then_paste_moves_and_empties_the_clipboard() {
        cut_then_paste(B::W);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_cut_then_paste_moves_and_empties_the_clipboard() {
        cut_then_paste(B::X);
    }

    // ---- A paste that takes part of our cut ----

    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }

    /// The rest of a cut stays cut, after a paste that skipped part of it and after one that
    /// failed partway. Not on the data device: the rest is copied again from the paste's read,
    /// outside the key's handler.
    fn a_partial_cut_paste_keeps_the_rest_cut(b: B) {
        let s = Session::start();
        let root = fixture(&s);
        std::fs::write(root.join("dest/x.txt"), "x").expect("a file");
        // `a-moves` moves; `b-locked` can't (a folder moves only with write access to it, for
        // its `..`).
        for d in ["src/a-moves", "src/b-locked"] {
            std::fs::create_dir(root.join(d)).expect("a folder");
        }
        let locked = root.join("src/b-locked");
        set_mode(&locked, 0o555);
        let demo = Demo::start_on(&s, b, root.clone());
        let peer = s.peer(b.display());
        let (inner, x) = (root.join("dest/inner"), root.join("dest/x.txt"));
        demo.open("dest");
        demo.copy(&s, &["inner", "x.txt"], true);
        wait_read(
            &peer,
            b.via(),
            GNOME,
            &format!("cut\n{}\n{}", uri(&inner), uri(&x)),
        );
        // Into `inner`: `inner` is skipped, `x.txt` moves.
        demo.select(&["inner"]);
        let at = demo.app.line_count();
        let v = demo.paste(&s);
        assert_eq!(
            (&v["dest"], &v["written"], &v["skipped"]),
            (
                &json!("dest/inner"),
                &json!(["x.txt"]),
                &json!([["inner", "into itself"]])
            ),
            "{v}"
        );
        assert!(inner.join("x.txt").exists() && !x.exists());
        // operations/clipboard/file-manager-formats: After a paste consumes part of a cut, the clipboard keeps the remaining items with the `cut` header and the KDE flag `1`.
        wait_read(&peer, b.via(), GNOME, &format!("cut\n{}", uri(&inner)));
        wait_read(&peer, b.via(), KDE, "1");
        wait_read(&peer, b.via(), URIS, &format!("{}\r\n", uri(&inner)));
        let consumed = demo.wait(at, "the cut consumed", |v| v["ev"] == "cut-consumed");
        assert_eq!(consumed["clipboard_changed"], true, "{consumed}");
        demo.open("dest");
        let st = demo.state();
        assert_eq!(
            (&st["marks"], &st["badge"]),
            (&json!({ "inner": "cut" }), &json!(true)),
            "{st}"
        );
        // A paste that fails partway: what moved leaves the cut, the rest stays cut.
        demo.open("src");
        demo.copy(&s, &["a-moves", "b-locked"], true);
        demo.open("Music");
        let at = demo.app.line_count();
        let v = demo.paste(&s);
        set_mode(&locked, 0o755);
        assert_eq!(
            (&v["ev"], &v["written"], &v["failed"]["name"]),
            (&json!("paste"), &json!(["a-moves"]), &json!("b-locked")),
            "{v}"
        );
        assert!(root.join("Music/a-moves").exists() && !root.join("src/a-moves").exists());
        wait_read(&peer, b.via(), GNOME, &format!("cut\n{}", uri(&locked)));
        demo.wait(at, "the cut consumed", |v| v["ev"] == "cut-consumed");
        let st = demo.state();
        assert_eq!(
            (&st["folder"], &st["selected"]),
            (&json!("Music"), &json!(["Music/a-moves"])),
            "the destination is shown: {st}"
        );
        // Once it can move, the rest pastes, and the clipboard empties.
        demo.select(&[]);
        let at = demo.app.line_count();
        let v = demo.paste(&s);
        assert_eq!(
            (&v["written"], &v["failed"]),
            (&json!(["b-locked"]), &Value::Null),
            "{v}"
        );
        assert!(root.join("Music/b-locked").exists() && !locked.exists());
        wait_targets(&peer, b.via(), None);
        demo.badge(at, false);
        demo.no_errors();
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_a_partial_cut_paste_keeps_the_rest_cut() {
        a_partial_cut_paste_keeps_the_rest_cut(B::W);
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn x11_a_partial_cut_paste_keeps_the_rest_cut() {
        a_partial_cut_paste_keeps_the_rest_cut(B::X);
    }

    // ---- An image ----

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn wayland_an_image_pastes_as_image_png() {
        // operations/clipboard: Pasting while the clipboard holds an image and no files writes `image.png`, then `image (1).png`, into a local destination.
        let s = Session::start();
        let demo = Demo::start(&s, B::W);
        let peer = s.peer(Display::Wayland);
        let png = Payload::Pattern {
            len: 300_000,
            seed: 7,
        };
        let want = fnv1a64(&png.bytes().expect("the pattern"));
        // A file list first, so the badge's turning off shows the image's offer was heard.
        demo.peer_copies(
            &peer,
            Via::DataControl,
            &[uri_list(&demo.root.join("todo.txt"))],
            true,
        );
        let image = Offer {
            mime: "image/png".to_owned(),
            data: png,
        };
        demo.peer_copies(&peer, Via::DataControl, std::slice::from_ref(&image), false);
        for name in ["image.png", "image (1).png"] {
            let v = demo.paste(&s);
            assert_eq!(v["written"], json!([name]), "{v}");
            let bytes = std::fs::read(demo.root.join(name)).expect("the image");
            assert_eq!(fnv1a64(&bytes), want, "{name}");
        }
        demo.no_errors();
    }

    // ---- Nautilus (opt-in) ----

    fn nautilus(s: &Session, display: Display, folder: &Path) -> (Proc, Window) {
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
            .arg(folder);
        let app = s.spawn("nautilus", cmd);
        let window = s.wait_window_where("Nautilus", |w| w.pid == app.pid());
        (app, window)
    }

    /// Says a test ran nothing, on stderr past libtest's capture (which takes only `print!`
    /// and `eprint!`), so it shows in every run: libtest has no "skipped" result, so the test
    /// still reports ok (docs/testing.md).
    fn skip(test: &str, why: &str) {
        use std::io::Write;
        // One write on the locked handle, so a parallel test's output can't split the line.
        let line = format!("SKIP x3::{test}: {why}\n");
        let _ = std::io::stderr().lock().write_all(line.as_bytes());
    }

    /// Nautilus in the nested sway: a file it copies pastes into the demo by Ctrl+V, and the
    /// demo's cut pastes into Nautilus as a move. Opt-in, with Nautilus installed:
    /// `STRATA_TEST_NAUTILUS=1 mise run test-platform x3::nautilus`. Best effort: Nautilus
    /// lists a folder some time after it maps, so its keys are repeated until they take.
    fn nautilus_round_trip(b: B, test: &str) {
        if std::env::var_os("STRATA_TEST_NAUTILUS").is_none() {
            return skip(
                test,
                "set STRATA_TEST_NAUTILUS=1 to run the Nautilus round trip",
            );
        }
        let s = Session::start();
        let root = fixture(&s);
        let from = root.join("from-nautilus");
        std::fs::create_dir_all(&from).expect("a folder");
        std::fs::write(from.join("alpha.txt"), "alpha").expect("a file");
        let demo = Demo::start_on(&s, b, root.clone());
        demo.open("dest");
        // Nautilus copies alpha.txt: the demo shows the badge, and Ctrl+V copies it into dest.
        let (n, window) = nautilus(&s, b.display(), &from);
        let deadline = Instant::now() + timeout() * 3;
        loop {
            s.focus(&window);
            std::thread::sleep(Duration::from_millis(1000));
            s.input().pointer_move(window.at(0.5, 0.5));
            s.input().key("ctrl+a");
            std::thread::sleep(Duration::from_millis(500));
            let at = demo.app.line_count();
            s.input().key("ctrl+c");
            let wait = Instant::now() + Duration::from_secs(3);
            let mut heard = false;
            while !heard && Instant::now() < wait {
                heard = demo
                    .lines_from(at)
                    .iter()
                    .any(|v| v["ev"] == "badge" && v["on"] == true);
                std::thread::sleep(Duration::from_millis(50));
            }
            if heard {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the demo never saw Nautilus's copy"
            );
        }
        let v = demo.paste(&s);
        assert_eq!(
            (&v["moving"], &v["written"]),
            (&json!(false), &json!(["alpha.txt"])),
            "{v}"
        );
        assert!(root.join("dest/alpha.txt").exists() && from.join("alpha.txt").exists());
        drop(n);
        // operations/clipboard/file-manager-formats: A file cut in Strata and pasted in Dolphin or Nautilus is moved, not copied.
        demo.open("");
        demo.copy(&s, &["todo.txt"], true);
        let (_n, window) = nautilus(&s, b.display(), &root.join("Music"));
        s.focus(&window);
        std::thread::sleep(Duration::from_millis(1500));
        s.input().pointer_move(window.at(0.5, 0.5));
        s.input().key("ctrl+v");
        let again = Instant::now() + Duration::from_secs(4);
        let mut pasted_again = false;
        let deadline = Instant::now() + timeout();
        while !(root.join("Music/todo.txt").exists() && !root.join("todo.txt").exists()) {
            if !pasted_again && Instant::now() > again {
                s.focus(&window);
                s.input().key("ctrl+v");
                pasted_again = true;
            }
            assert!(
                Instant::now() < deadline,
                "Nautilus didn't move the cut file: in Music {}, at the root {}",
                root.join("Music/todo.txt").exists(),
                root.join("todo.txt").exists()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn nautilus_wayland_round_trip() {
        nautilus_round_trip(B::W, "nautilus_wayland_round_trip");
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn nautilus_x11_round_trip() {
        nautilus_round_trip(B::X, "nautilus_x11_round_trip");
    }
}

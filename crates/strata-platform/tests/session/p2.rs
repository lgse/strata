//! P2's session tests: clipboard round-trips with the peer, on Wayland and X11, through the
//! public `clipboard` API. The app side is `examples/clipboard.rs`.
//!
//! Each scope item runs on each route it applies to:
//! - **W**: Wayland data control (sway's default);
//! - **D**: Wayland `wl_data_device` on winit's display, on a compositor that seems to have
//!   no data control (the test switch hides its globals), as mutter;
//! - **X**: X11 selections;
//! - **XW**: a Wayland window with no Wayland route (libwayland < 1.23 and a compositor with
//!   no data control, both simulated), through Xwayland.
//!
//! D needs libwayland 1.23 or later (winit's display); on an older host its tests check only
//! that the app reports no route there, and say so on stderr.
//!
//! Work package: P2, `docs/work/phase-2-composites/P2-clipboard.md`.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use strata_platform::transfer::mime;
use strata_test_peer::protocol::{Command, Event, Offer, Payload, Selection, Via, fnv1a64};

use crate::support::{Display, Peer, Proc, Session, Window, example, timeout};

const UTF8: &str = "text/plain;charset=utf-8";
const PNG: &str = "image/png";
const GNOME: &str = mime::GNOME_COPIED_FILES;
const URIS: &str = mime::URI_LIST;
const KDE: &str = mime::KDE_CUT_SELECTION;

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

    fn args(self) -> &'static [&'static str] {
        match self {
            B::D => &["--as-if-no-data-control"],
            B::W | B::X => &[],
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

/// The five types a file manager's cut offers, with an image: the GNOME and KDE formats made
/// with P0's encoders from a path that isn't UTF-8, text, and `png_len` bytes of image.
fn five(png_len: u64) -> Vec<Offer> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    let uris = [
        mime::file_uri(Path::new(OsStr::from_bytes(b"/tmp/p2 \xff 100%.txt"))),
        mime::file_uri(Path::new("/tmp/café#2")),
    ];
    let text = |b: Vec<u8>| String::from_utf8(b).expect("ASCII");
    vec![
        Offer::text(GNOME, &text(mime::encode_gnome_copied_files(true, &uris))),
        Offer::text(URIS, &text(mime::encode_uri_list(&uris))),
        Offer::text(KDE, "1"),
        Offer::text(UTF8, "héllo from P2 ✓"),
        Offer {
            mime: PNG.into(),
            data: Payload::Pattern {
                len: png_len,
                seed: 29,
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

/// Whether this host's libwayland-client has `wl_proxy_get_display` (1.23 or later).
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
    !unsafe { libc::dlsym(lib, c"wl_proxy_get_display".as_ptr()) }.is_null()
}

/// The demo app and its window.
struct Demo {
    app: Proc,
    window: Window,
    attached: Value,
}

impl Demo {
    fn start(s: &Session, b: B) -> Option<Demo> {
        Demo::start_with(s, b.display(), b.args(), &[], Some(b))
    }

    /// The app on `display`, with more of the session's environment from `also`. None, after
    /// checking the app reports no route, for D on a libwayland without winit's display.
    fn start_with(
        s: &Session,
        display: Display,
        args: &[&str],
        also: &[Display],
        b: Option<B>,
    ) -> Option<Demo> {
        let mut cmd = s.command(example("clipboard"), display);
        for d in also {
            cmd.envs(s.env(*d));
        }
        cmd.args(["--title", "p2 clipboard"]).args(args);
        let app = s.spawn("clipboard", cmd);
        let attached = app.wait_json("attach", |v| {
            v["ev"] == "attached" || v["ev"] == "attach-error"
        });
        if b == Some(B::D) && !libwayland_has_get_display() {
            assert_eq!(attached["ev"], "attach-error", "{attached}");
            eprintln!("libwayland < 1.23 here: the data-device route didn't run");
            return None;
        }
        assert_eq!(attached["ev"], "attached", "{attached}");
        if let Some(b) = b {
            assert_eq!(attached["route"], b.route(), "{attached}");
        }
        app.wait_json("the first frame", |v| v["ev"] == "frame");
        let window = s.wait_window(app.pid());
        let demo = Demo {
            app,
            window,
            attached,
        };
        let (w, h) = (demo.window.content.width, demo.window.content.height);
        demo.app.wait_json(&format!("a layout at {w}x{h}"), |v| {
            v["ev"] == "layout"
                && (v["w"].as_f64().unwrap_or(0.0) - w).abs() < 1.0
                && (v["h"].as_f64().unwrap_or(0.0) - h).abs() < 1.0
        });
        Some(demo)
    }

    fn send(&self, cmd: Value) -> usize {
        let mark = self.app.line_count();
        self.app.send_line(&cmd.to_string());
        mark
    }

    fn wait(&self, from: usize, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        self.app.wait_json_from(from, what, pred).1
    }

    /// A click on the window: keyboard focus, and a serial for a data-device write.
    fn click(&self, s: &Session) {
        let at = self.app.line_count();
        s.input().click_at(self.window.at(0.5, 0.5));
        self.wait(at, "the click", |v| v["ev"] == "press");
    }

    /// Copies `cmd` (a `copy` or `copy-files` command); the copy's id.
    fn copy(&self, cmd: Value) -> u64 {
        let at = self.send(cmd);
        let v = self.wait(at, "the copy", |v| {
            v["ev"] == "copied" || v["ev"] == "copy-error"
        });
        assert_eq!(v["ev"], "copied", "{v}");
        v["copy"].as_u64().expect("a copy id")
    }

    fn copy_offers(&self, selection: Selection, offers: &[Offer]) -> u64 {
        self.copy(json!({ "cmd": "copy", "selection": selection, "offers": offers }))
    }

    /// Waits for a change from line `from` on, of `kind`, for `copy` when given.
    fn change(&self, from: usize, kind: &str, copy: Option<u64>) -> Value {
        self.wait(from, &format!("a change {kind} ({copy:?})"), |v| {
            v["ev"] == "change" && v["kind"] == kind && copy.is_none_or(|c| v["copy"] == c)
        })
    }

    /// Reads the first of `mimes`: the read, or its error.
    fn read(&self, selection: Selection, mimes: &[&str], max_bytes: Option<u64>) -> Value {
        let at = self.send(json!({
            "cmd": "read", "selection": selection, "mimes": mimes, "max_bytes": max_bytes,
        }));
        self.wait(at, "the read", |v| {
            v["ev"] == "read" || v["ev"] == "read-error"
        })
    }

    fn read_files(&self, selection: Selection) -> Value {
        let at = self.send(json!({ "cmd": "read-files", "selection": selection }));
        self.wait(at, "the files", |v| {
            v["ev"] == "files" || v["ev"] == "read-error"
        })
    }

    fn state(&self, copy: u64) -> String {
        let at = self.send(json!({ "cmd": "state", "copy": copy }));
        let v = self.wait(at, "the state", |v| v["ev"] == "state");
        v["state"].as_str().unwrap_or_default().to_owned()
    }

    fn writer_runs(&self, mime: &str) -> usize {
        self.app
            .lines()
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["ev"] == "writer" && v["mime"] == mime)
            .count()
    }

    /// Waits until the writer of `mime` has run `n` times (its line may trail the read it
    /// served), then checks it ran exactly `n` times.
    fn assert_writer_runs(&self, mime: &str, n: usize, why: &str) {
        let deadline = Instant::now() + timeout();
        while self.writer_runs(mime) < n && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(self.writer_runs(mime), n, "{mime}: {why}");
    }

    /// Quits through `cmd` (`quit` or `quit-dnd`); the shutdown line. The app must exit 0.
    fn quit(mut self, cmd: &str) -> Value {
        let at = self.send(json!({ "cmd": cmd }));
        let down = self.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
        let status = self.app.wait_exit();
        assert!(status.success(), "the app exited with {status}");
        self.no_errors();
        down
    }

    fn no_errors(&self) {
        let bad: Vec<String> = self
            .app
            .lines()
            .into_iter()
            .filter(|l| {
                ["exit-error", "copy-error", "attach-error", "bad-command"]
                    .iter()
                    .any(|e| l.contains(e))
            })
            .collect();
        assert!(bad.is_empty(), "the app reported errors: {bad:?}");
    }
}

/// Polls the peer until the selection lists every one of `want` (None: until it is empty).
fn wait_targets(peer: &Peer, selection: Selection, via: Via, want: Option<&[String]>) {
    let deadline = Instant::now() + timeout();
    loop {
        peer.sync();
        let got = peer.targets(selection, via);
        let done = match (want, &got) {
            (Some(want), Some(got)) => want.iter().all(|m| got.contains(m)),
            (None, None) => true,
            (None, Some(got)) => got.is_empty(),
            (Some(_), None) => false,
        };
        if done {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the peer's {via:?} selection offers {got:?}, not {want:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A peer with a mapped window, tiled beside the app's: the peer and its window. The app's
/// window moved for it, so `demo` takes its new place and waits for the app's layout there.
fn peer_beside(s: &Session, display: Display, demo: &mut Demo) -> (Peer, Window) {
    let peer = s.peer(display);
    peer.map("p2 peer");
    let windows = s.wait_windows(&[demo.app.pid(), peer.pid()]);
    let find = |pid: u32| {
        windows
            .iter()
            .find(|w| w.pid == pid)
            .expect("both windows")
            .clone()
    };
    demo.window = find(demo.app.pid());
    let (w, h) = (demo.window.content.width, demo.window.content.height);
    demo.app.wait_json(&format!("a layout at {w}x{h}"), |v| {
        v["ev"] == "layout"
            && (v["w"].as_f64().unwrap_or(0.0) - w).abs() < 1.0
            && (v["h"].as_f64().unwrap_or(0.0) - h).abs() < 1.0
    });
    let w = find(peer.pid());
    (peer, w)
}

// ---- 1. Copies the peer reads ----

/// Five types with lazy writers: listed, each read byte for byte, and each writer run once
/// though the GNOME type is read twice.
fn copy_offers_five_types(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    if b == B::D {
        demo.click(&s);
    }
    let at = demo.app.line_count();
    let offers = five(4 << 20);
    let copy = demo.copy_offers(Selection::Clipboard, &offers);
    wait_targets(&peer, Selection::Clipboard, b.via(), Some(&mimes(&offers)));
    demo.change(at, "holding", Some(copy));
    for o in &offers {
        assert_eq!(
            demo.writer_runs(&o.mime),
            0,
            "{} ran before anyone asked",
            o.mime
        );
    }
    for o in offers.iter().chain(&offers[..1]) {
        let got = peer.read(Selection::Clipboard, b.via(), &o.mime);
        assert_eq!(got.fnv1a64, fnv(o), "{}", o.mime);
    }
    for o in &offers {
        demo.assert_writer_runs(&o.mime, 1, "read, written once");
    }
    assert_eq!(demo.state(copy), "Some(Holding)");
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_copy_offers_five_types_the_peer_reads() {
    copy_offers_five_types(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_device_copy_offers_five_types_the_peer_reads() {
    copy_offers_five_types(B::D);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_copy_offers_five_types_the_peer_reads() {
    copy_offers_five_types(B::X);
}

/// `Files` and `Content::text`: the peer reads what GTK writes, byte for byte.
fn copy_files_writes_the_file_manager_formats(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    if b == B::D {
        demo.click(&s);
    }
    let uris = ["file:///tmp/a%20b", "file:///tmp/c"];
    for (cut, header, flag) in [(false, "copy", "0"), (true, "cut", "1")] {
        demo.copy(json!({
            "cmd": "copy-files", "uris": uris, "cut": cut, "text": "/tmp/a b\n/tmp/c",
        }));
        let want: Vec<String> = [
            GNOME,
            URIS,
            KDE,
            UTF8,
            "UTF8_STRING",
            "text/plain",
            "STRING",
        ]
        .map(str::to_owned)
        .to_vec();
        wait_targets(&peer, Selection::Clipboard, b.via(), Some(&want));
        let read = |m: &str| peer.read(Selection::Clipboard, b.via(), m).text;
        assert_eq!(
            read(GNOME).as_deref(),
            Some(format!("{header}\nfile:///tmp/a%20b\nfile:///tmp/c").as_str())
        );
        assert_eq!(
            read(URIS).as_deref(),
            Some("file:///tmp/a%20b\r\nfile:///tmp/c\r\n")
        );
        assert_eq!(read(KDE).as_deref(), Some(flag));
        assert_eq!(read(UTF8).as_deref(), Some("/tmp/a b\n/tmp/c"));
    }
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_copy_files_writes_the_file_manager_formats() {
    copy_files_writes_the_file_manager_formats(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_device_copy_files_writes_the_file_manager_formats() {
    copy_files_writes_the_file_manager_formats(B::D);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_copy_files_writes_the_file_manager_formats() {
    copy_files_writes_the_file_manager_formats(B::X);
}

// ---- 2. The peer's copies the app reads ----

fn peer_offers_five_types(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    if b == B::D {
        // The data device hears the selection only with keyboard focus.
        demo.click(&s);
    }
    let offers = five(4 << 20);
    let at = demo.app.line_count();
    peer.set_selection(Selection::Clipboard, b.via(), &offers);
    let heard = demo.wait(at, "the peer's five types", |v| {
        v["ev"] == "change"
            && v["kind"] == "offered"
            && v["mimes"]
                .as_array()
                .is_some_and(|m| m.len() == offers.len())
    });
    assert_eq!(heard["mimes"], json!(mimes(&offers)), "{heard}");
    for o in &offers {
        let got = demo.read(Selection::Clipboard, &[o.mime.as_str()], None);
        assert_eq!(got["ev"], "read", "{got}");
        assert_eq!(got["fnv1a64"], fnv(o), "{}", o.mime);
    }
    let files = demo.read_files(Selection::Clipboard);
    assert_eq!(files["ev"], "files", "{files}");
    assert_eq!(files["cut"], true);
    assert_eq!(files["uris"].as_array().map(Vec::len), Some(2), "{files}");
    // The first of several types offered wins, in the order asked.
    let got = demo.read(Selection::Clipboard, &["text/x-none", UTF8, PNG], None);
    assert_eq!(got["mime"], UTF8);
    assert_eq!(got["text"], "héllo from P2 ✓");
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_peer_offers_five_types_the_app_reads() {
    peer_offers_five_types(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_device_peer_offers_five_types_the_app_reads() {
    peer_offers_five_types(B::D);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_peer_offers_five_types_the_app_reads() {
    peer_offers_five_types(B::X);
}

/// X11 owners list targets that are no data type (GTK lists `SAVE_TARGETS`): they are left
/// out of what the app hears is offered.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_targets_that_are_no_data_type_are_left_out_of_offers() {
    let s = Session::start();
    let demo = Demo::start(&s, B::X).expect("X11");
    let peer = s.peer(Display::X11);
    let meta = [
        "SAVE_TARGETS",
        "DELETE",
        "INSERT_SELECTION",
        "INSERT_PROPERTY",
    ];
    let mut offers: Vec<Offer> = meta.iter().map(|m| Offer::text(m, "")).collect();
    offers.push(Offer::text(UTF8, "data"));
    let at = demo.app.line_count();
    peer.set_selection(Selection::Clipboard, Via::X11, &offers);
    let heard = demo.change(at, "offered", None);
    assert_eq!(heard["mimes"], json!([UTF8]), "{heard}");
    demo.no_errors();
}

// ---- 3. A large image both ways (INCR on X11) ----

fn large_image_both_ways(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    let image = Offer {
        mime: PNG.into(),
        data: Payload::Pattern {
            len: 8 << 20,
            seed: 31,
        },
    };
    demo.copy_offers(Selection::Clipboard, std::slice::from_ref(&image));
    wait_targets(
        &peer,
        Selection::Clipboard,
        b.via(),
        Some(&[PNG.to_owned()]),
    );
    let got = peer.read(Selection::Clipboard, b.via(), PNG);
    assert_eq!(
        (got.len, got.fnv1a64.as_str()),
        (8 << 20, fnv(&image).as_str())
    );
    let theirs = Offer {
        mime: PNG.into(),
        data: Payload::Pattern {
            len: 8 << 20,
            seed: 37,
        },
    };
    let at = demo.app.line_count();
    peer.set_selection(Selection::Clipboard, b.via(), std::slice::from_ref(&theirs));
    demo.change(at, "offered", None);
    let got = demo.read(Selection::Clipboard, &[PNG], None);
    assert_eq!(got["ev"], "read", "{got}");
    assert_eq!(
        (got["len"].as_u64(), &got["fnv1a64"]),
        (Some(8 << 20), &json!(fnv(&theirs)))
    );
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_large_image_both_ways() {
    large_image_both_ways(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_large_image_both_ways_by_incr() {
    large_image_both_ways(B::X);
}

// ---- 4. Ownership loss ----

/// The app copies, then the peer's window takes focus and the peer copies: the app hears its
/// copy lost and, where the route hears unfocused, the peer's offer at once. On the data
/// device the offer waits for the app's next focus.
fn ownership_loss(b: B) {
    let s = Session::start();
    let Some(mut demo) = Demo::start(&s, b) else {
        return;
    };
    let (peer, peer_window) = peer_beside(&s, b.display(), &mut demo);
    if b == B::D {
        demo.click(&s);
    }
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "the app's")]);
    demo.change(at, "holding", Some(copy));
    s.focus(&peer_window);
    peer.wait_event("focus", |e| matches!(e, Event::Focus { focused: true }));
    let at = demo.app.line_count();
    // The data device writes with the peer's own focus; the others don't need it.
    let via = if b == B::D { Via::DataDevice } else { b.via() };
    peer.set_selection(
        Selection::Clipboard,
        via,
        &[Offer::text(UTF8, "the peer's")],
    );
    let lost = demo.change(at, "lost", Some(copy));
    assert_eq!(lost["why"], "taken", "{lost}");
    assert_eq!(demo.state(copy), "Some(Lost(Taken))");
    if b == B::D {
        let early = demo.app.lines()[at..]
            .iter()
            .any(|l| l.contains("\"offered\""));
        assert!(!early, "the data device heard the offer without focus");
        let at = demo.app.line_count();
        s.focus(&demo.window);
        demo.change(at, "offered", None);
    } else {
        demo.change(at, "offered", None);
    }
    let got = demo.read(Selection::Clipboard, &[UTF8], None);
    assert_eq!(got["text"], "the peer's");
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_hears_its_copy_lost_and_the_peers_offer_unfocused() {
    ownership_loss(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_device_hears_its_copy_lost_at_once_and_the_offer_at_focus() {
    ownership_loss(B::D);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_hears_its_copy_lost_and_the_peers_offer() {
    ownership_loss(B::X);
}

/// On another workspace, where sway stops the window's frames, the app's state still hears
/// the loss.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_a_window_on_another_workspace_hears_its_copy_lost() {
    let s = Session::start();
    let demo = Demo::start(&s, B::W).expect("data control");
    let peer = s.peer(Display::Wayland);
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "the app's")]);
    demo.change(at, "holding", Some(copy));
    s.sway_command(&format!(
        "[con_id={}] move container to workspace 9",
        demo.window.id
    ));
    let at = demo.app.line_count();
    peer.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[Offer::text(UTF8, "the peer's")],
    );
    demo.change(at, "lost", Some(copy));
    demo.no_errors();
}

// ---- 5. Rune's own Tx::copy ----

/// Rune's copy takes the clipboard from the app's own (lost as taken) and other apps read
/// it; a copy here then takes it back.
fn runes_copy_and_ours(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &five(64));
    demo.change(at, "holding", Some(copy));
    let at = demo.send(json!({ "cmd": "rune-copy", "text": "text from Rune" }));
    let lost = demo.change(at, "lost", Some(copy));
    assert_eq!(lost["why"], "taken", "{lost}");
    demo.change(at, "offered", None);
    let text = if b == B::X { "UTF8_STRING" } else { UTF8 };
    wait_targets(
        &peer,
        Selection::Clipboard,
        b.via(),
        Some(&[text.to_owned()]),
    );
    assert_eq!(
        peer.read(Selection::Clipboard, b.via(), text)
            .text
            .as_deref(),
        Some("text from Rune")
    );
    let got = demo.read(Selection::Clipboard, &[text], None);
    assert_eq!(got["text"], "text from Rune", "{got}");
    let at = demo.app.line_count();
    let again = demo.copy_offers(Selection::Clipboard, &five(64));
    demo.change(at, "holding", Some(again));
    wait_targets(
        &peer,
        Selection::Clipboard,
        b.via(),
        Some(&mimes(&five(64))),
    );
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_runes_copy_takes_the_clipboard_and_ours_takes_it_back() {
    runes_copy_and_ours(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_runes_copy_takes_the_clipboard_and_ours_takes_it_back() {
    runes_copy_and_ours(B::X);
}

// ---- 6. Clearing ----

/// A clear empties the selection the copy holds; a clear of a copy another app replaced
/// leaves that app's copy alone.
fn clearing(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    if b == B::D {
        demo.click(&s);
    }
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "the app's")]);
    demo.change(at, "holding", Some(copy));
    wait_targets(
        &peer,
        Selection::Clipboard,
        b.via(),
        Some(&[UTF8.to_owned()]),
    );
    let at = demo.send(json!({ "cmd": "clear", "copy": copy }));
    let cleared = demo.wait(at, "the clear", |v| v["ev"] == "cleared");
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert_eq!(demo.change(at, "lost", Some(copy))["why"], "cleared");
    wait_targets(&peer, Selection::Clipboard, b.via(), None);

    let at = demo.app.line_count();
    let second = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "again")]);
    demo.change(at, "holding", Some(second));
    let at = demo.app.line_count();
    peer.set_selection(
        Selection::Clipboard,
        b.via(),
        &[Offer::text(UTF8, "the peer's")],
    );
    demo.change(at, "lost", Some(second));
    let at = demo.send(json!({ "cmd": "clear", "copy": second }));
    let cleared = demo.wait(at, "the clear", |v| v["ev"] == "cleared");
    assert_eq!(cleared["ok"], false, "{cleared}");
    assert_eq!(
        peer.read(Selection::Clipboard, b.via(), UTF8)
            .text
            .as_deref(),
        Some("the peer's")
    );
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_clear_empties_only_a_selection_the_copy_holds() {
    clearing(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_device_clear_empties_only_a_selection_the_copy_holds() {
    clearing(B::D);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_clear_empties_only_a_selection_the_copy_holds() {
    clearing(B::X);
}

/// Another app takes the clipboard while the app's clear is on its way (the app hasn't heard
/// yet): the clear must leave the other app's copy alone. Data control on winit's display, so
/// the app hears nothing while its UI thread is held.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_a_clear_racing_another_apps_copy_leaves_that_copy() {
    let s = Session::start();
    let demo = Demo::start(&s, B::W).expect("data control");
    let peer = s.peer(Display::Wayland);
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "the app's")]);
    demo.change(at, "holding", Some(copy));
    let at = demo.send(json!({ "cmd": "clear-later", "copy": copy, "ms": 1500 }));
    peer.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[Offer::text(UTF8, "the peer's")],
    );
    let cleared = demo.wait(at, "the late clear", |v| v["ev"] == "cleared");
    // The app still thought it held the copy, so its clear went out.
    assert_eq!(cleared["ok"], true, "{cleared}");
    peer.sync();
    assert_eq!(
        peer.read(Selection::Clipboard, Via::DataControl, UTF8)
            .text
            .as_deref(),
        Some("the peer's"),
        "the clear wiped the other app's copy"
    );
    demo.no_errors();
}

/// On X11 a copy waits a round trip for its timestamp: a clear meanwhile cancels the take, so
/// the copy never holds and nobody is left offering it.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_a_clear_before_the_copy_holds_cancels_it() {
    let s = Session::start();
    let demo = Demo::start(&s, B::X).expect("X11");
    let peer = s.peer(Display::X11);
    let at = demo.send(json!({
        "cmd": "copy-then-clear", "selection": Selection::Clipboard,
        "offers": [Offer::text(UTF8, "never held")],
    }));
    let cleared = demo.wait(at, "the clear", |v| v["ev"] == "cleared");
    assert_eq!(cleared["ok"], true, "{cleared}");
    let copied = demo.wait(at, "the copy", |v| v["ev"] == "copied");
    let copy = copied["copy"].as_u64().expect("a copy id");
    assert_eq!(demo.change(at, "lost", Some(copy))["why"], "cleared");
    // A take still on its way would hold by now: a later round trip shows nobody owns it.
    demo.read(Selection::Clipboard, &[UTF8], None);
    wait_targets(&peer, Selection::Clipboard, Via::X11, None);
    assert!(
        !demo.app.lines()[at..]
            .iter()
            .any(|l| l.contains("\"holding\"")),
        "the cleared copy held"
    );
    demo.no_errors();
}

// ---- 7. The data device's serials ----

/// A write with a serial older than the current selection's is dropped by the compositor:
/// the copy is lost as refused. A real Ctrl+C in the window copies with the key's serial.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_device_a_stale_write_is_refused_and_ctrl_c_holds() {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, B::D) else {
        return;
    };
    let peer = s.peer(Display::Wayland);
    demo.click(&s);
    let at = demo.app.line_count();
    peer.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[Offer::text(UTF8, "the peer's, newer than the click")],
    );
    demo.change(at, "offered", None);
    let at = demo.app.line_count();
    let stale = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "stale")]);
    let lost = demo.change(at, "lost", Some(stale));
    assert_eq!(lost["why"], "refused", "{lost}");
    assert_eq!(demo.state(stale), "Some(Lost(Refused))");

    let offers = [Offer::text(UTF8, "from ctrl+c")];
    let at = demo.send(json!({ "cmd": "arm", "offers": offers }));
    demo.wait(at, "armed", |v| v["ev"] == "armed");
    let at = demo.app.line_count();
    s.input().key("ctrl+c");
    let copied = demo.wait(at, "the ctrl+c copy", |v| v["ev"] == "copied");
    let copy = copied["copy"].as_u64().expect("a copy id");
    demo.change(at, "holding", Some(copy));
    assert_eq!(
        peer.read(Selection::Clipboard, Via::DataControl, UTF8)
            .text
            .as_deref(),
        Some("from ctrl+c")
    );
    demo.no_errors();
}

/// A copy made while the app has no keyboard focus gets no echo and stays pending (wlroots).
/// When another app then takes the clipboard, the copy was taken, not refused: only a write
/// P0 judged dropped is refused.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_device_an_unfocused_copy_another_app_takes_is_lost_as_taken() {
    let s = Session::start();
    let Some(mut demo) = Demo::start(&s, B::D) else {
        return;
    };
    let (peer, peer_window) = peer_beside(&s, Display::Wayland, &mut demo);
    demo.click(&s);
    s.focus(&peer_window);
    peer.wait_event("focus", |e| matches!(e, Event::Focus { focused: true }));
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "unfocused")]);
    // Its write took: the peer reads it.
    wait_targets(
        &peer,
        Selection::Clipboard,
        Via::DataControl,
        Some(&[UTF8.to_owned()]),
    );
    assert_eq!(
        peer.read(Selection::Clipboard, Via::DataControl, UTF8)
            .text
            .as_deref(),
        Some("unfocused")
    );
    assert_eq!(demo.state(copy), "Some(Pending)", "no echo without focus");
    peer.set_selection(
        Selection::Clipboard,
        Via::DataDevice,
        &[Offer::text(UTF8, "the peer's")],
    );
    let lost = demo.change(at, "lost", Some(copy));
    assert_eq!(lost["why"], "taken", "{lost}");
    demo.no_errors();
}

// ---- 8. The primary selection ----

fn primary_round_trip(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Primary, &[Offer::text(UTF8, "primary from P2")]);
    let held = demo.change(at, "holding", Some(copy));
    assert_eq!(held["selection"], "primary");
    wait_targets(&peer, Selection::Primary, b.via(), Some(&[UTF8.to_owned()]));
    assert_eq!(
        peer.read(Selection::Primary, b.via(), UTF8).text.as_deref(),
        Some("primary from P2")
    );
    let at = demo.app.line_count();
    peer.set_selection(
        Selection::Primary,
        b.via(),
        &[Offer::text(UTF8, "the peer's primary")],
    );
    demo.change(at, "lost", Some(copy));
    let got = demo.read(Selection::Primary, &[UTF8], None);
    assert_eq!(got["text"], "the peer's primary");
    // The clipboard is apart from it.
    let got = demo.read(Selection::Clipboard, &[UTF8], None);
    assert_eq!(got["ev"], "read-error", "{got}");
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_primary_selection_round_trip() {
    primary_round_trip(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_primary_selection_round_trip() {
    primary_round_trip(B::X);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_device_has_no_primary_selection() {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, B::D) else {
        return;
    };
    assert_eq!(demo.attached["primary"], false);
    demo.click(&s);
    let at = demo.send(json!({
        "cmd": "copy", "selection": Selection::Primary, "offers": [Offer::text(UTF8, "x")],
    }));
    let v = demo.wait(at, "the copy", |v| {
        v["ev"] == "copied" || v["ev"] == "copy-error"
    });
    assert_eq!(v["ev"], "copy-error", "{v}");
    assert!(
        v["error"].as_str().unwrap_or_default().contains("refused"),
        "{v}"
    );
}

// ---- 9. After the app exits (X11) ----

/// With a clipboard manager running, quitting hands it every type (one MULTIPLE, the large
/// image by INCR): another app reads them all after the app has gone.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_a_clipboard_manager_keeps_every_type_after_the_app_exits() {
    let s = Session::start();
    let manager = s.peer(Display::X11);
    manager.clipboard_manager(true);
    let demo = Demo::start(&s, B::X).expect("X11");
    assert_eq!(demo.attached["keeps_after_exit"], true);
    let offers = five(4 << 20);
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &offers);
    demo.change(at, "holding", Some(copy));
    let started = Instant::now();
    let down = demo.quit("quit");
    assert_eq!(down["hand_off"], "Saved", "{down}");
    let saved = manager.wait_event("manager-saved", |e| matches!(e, Event::ManagerSaved { .. }));
    let Event::ManagerSaved { saved, refused } = saved else {
        unreachable!()
    };
    assert_eq!(saved, mimes(&offers));
    assert!(refused.is_empty(), "{refused:?}");
    assert!(started.elapsed() < timeout(), "the exit waited too long");
    let reader = s.peer(Display::X11);
    for o in &offers {
        assert_eq!(
            reader.read(Selection::Clipboard, Via::X11, &o.mime).fnv1a64,
            fnv(o),
            "{}",
            o.mime
        );
    }
}

/// Without a manager the exit doesn't wait. A copy that asks not to be kept isn't handed
/// over even with one.
///
/// sway's Xwayland window manager (wlroots' xwm) owns `CLIPBOARD_MANAGER` itself, and answers
/// every hand-off at once without saving anything (its own comment calls it a race). So a
/// peer takes the selection from it and exits, which leaves no manager at all.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_without_a_manager_or_with_keep_off_nothing_is_handed_over() {
    let s = Session::start();
    {
        let gone = s.peer(Display::X11);
        gone.clipboard_manager(false);
        gone.sync();
    }
    let demo = Demo::start(&s, B::X).expect("X11");
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "a")]);
    demo.change(at, "holding", Some(copy));
    let down = demo.quit("quit");
    assert_eq!(down["hand_off"], "NoManager", "{down}");
    assert!(down["ms"].as_f64().unwrap_or(f64::MAX) < 500.0, "{down}");

    let manager = s.peer(Display::X11);
    manager.clipboard_manager(true);
    let demo = Demo::start(&s, B::X).expect("X11");
    let at = demo.app.line_count();
    let copy = demo.copy(json!({
        "cmd": "copy", "selection": Selection::Clipboard, "offers": [Offer::text(UTF8, "b")],
        "keep_ms": null,
    }));
    demo.change(at, "holding", Some(copy));
    let mark = manager.line_count();
    let down = demo.quit("quit");
    assert_eq!(down["hand_off"], "NotHeld", "{down}");
    assert!(
        !manager.saw_since(mark, Duration::from_millis(300), |e| matches!(
            e,
            Event::ManagerSaved { .. }
        )),
        "the manager was handed a copy that asked not to be kept"
    );
}

/// A manager that never answers: the exit waits the copy's own wait (600 ms here), measured
/// from the shutdown's start to its end, and no more.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_a_silent_manager_holds_the_exit_only_as_long_as_the_copy_asked() {
    let s = Session::start();
    let manager = s.peer(Display::X11);
    manager.clipboard_manager(false);
    let demo = Demo::start(&s, B::X).expect("X11");
    let at = demo.app.line_count();
    let copy = demo.copy(json!({
        "cmd": "copy", "selection": Selection::Clipboard, "offers": [Offer::text(UTF8, "a")],
        "keep_ms": 600,
    }));
    demo.change(at, "holding", Some(copy));
    let down = demo.quit("quit");
    assert_eq!(down["hand_off"], "TimedOut", "{down}");
    let ms = down["ms"].as_f64().expect("ms");
    assert!((600.0..1600.0).contains(&ms), "the wait took {ms} ms");
}

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

/// A `MULTIPLE` request naming one lazy type 50 times, whose writer never returns, beside
/// text: the writer runs on one worker, not one per pair, and the request is answered by its
/// deadline, the stuck type refused and the text given.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_multiple_runs_each_writer_once_and_answers_by_its_deadline() {
    let s = Session::start();
    let demo = Demo::start(&s, B::X).expect("X11");
    let peer = s.peer(Display::X11);
    let stuck = "application/x-p2-stuck";
    let at = demo.app.line_count();
    let copy = demo.copy(json!({
        "cmd": "copy", "selection": Selection::Clipboard, "hang": [stuck],
        "offers": [Offer::text(stuck, "never"), Offer::text(UTF8, "given")],
    }));
    demo.change(at, "holding", Some(copy));
    let mut mimes = vec![stuck; 50];
    mimes.push(UTF8);
    let started = Instant::now();
    let results = peer.read_multiple(Selection::Clipboard, &mimes);
    let took = started.elapsed();
    assert!(
        results[..50].iter().all(Option::is_none),
        "the stuck type is refused"
    );
    assert_eq!(
        results[50].as_ref().and_then(|r| r.text.as_deref()),
        Some("given")
    );
    assert!(
        took >= Duration::from_secs(4),
        "answered before the deadline: {took:?}"
    );
    assert_eq!(demo.writer_runs(stuck), 1, "one run for 50 pairs");
    assert!(
        threads_named(demo.app.pid(), "transfer-write") <= 1,
        "a worker per pair: {}",
        threads_named(demo.app.pid(), "transfer-write")
    );
}

/// A `MULTIPLE` request may name 1024 pairs; one with more is refused whole, as a request can
/// carry 32k and each target costs the owner a round trip.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_multiple_with_more_than_1024_pairs_is_refused() {
    let s = Session::start();
    let demo = Demo::start(&s, B::X).expect("X11");
    let peer = s.peer(Display::X11);
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "t")]);
    demo.change(at, "holding", Some(copy));
    let at_most = peer.read_multiple(Selection::Clipboard, &[UTF8; 1024]);
    assert!(
        at_most.iter().all(Option::is_some),
        "1024 pairs are answered"
    );
    let over = peer.try_request(Command::SelectionReadMultiple {
        selection: Selection::Clipboard,
        mimes: vec![UTF8.to_owned(); 1025],
    });
    let refused = over.as_ref().err().is_some_and(|e| e.contains("refused"));
    assert!(refused, "1025 pairs are refused whole: {over:?}");
    demo.no_errors();
}

// ---- 10. Read limits and reads of our own copy ----

fn read_limits(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    let big = Offer {
        mime: PNG.into(),
        data: Payload::Pattern {
            len: 2 << 20,
            seed: 41,
        },
    };
    let at = demo.app.line_count();
    peer.set_selection(
        Selection::Clipboard,
        b.via(),
        &[big.clone(), Offer::text(UTF8, "t")],
    );
    demo.change(at, "offered", None);
    let got = demo.read(Selection::Clipboard, &[PNG], Some(1 << 20));
    assert_eq!(
        (&got["ev"], &got["error"]),
        (&json!("read-error"), &json!("TooLarge")),
        "{got}"
    );
    let got = demo.read(Selection::Clipboard, &["text/x-none"], None);
    assert_eq!(got["error"], "NothingOffered", "{got}");
    let got = demo.read(Selection::Clipboard, &[PNG], Some(2 << 20));
    assert_eq!(got["fnv1a64"], fnv(&big), "{got}");

    // Our own copy is read back from its writers, in process.
    let at = demo.app.line_count();
    let copy = demo.copy_offers(
        Selection::Clipboard,
        &[Offer::text(UTF8, "ours"), big.clone()],
    );
    demo.change(at, "holding", Some(copy));
    let at = demo.send(json!({ "cmd": "offered", "selection": Selection::Clipboard }));
    let offered = demo.wait(at, "offered", |v| v["ev"] == "offered");
    assert_eq!(offered["ours"], copy, "{offered}");
    let got = demo.read(Selection::Clipboard, &[UTF8], None);
    assert_eq!(got["text"], "ours", "{got}");
    let got = demo.read(Selection::Clipboard, &[PNG], Some(1 << 20));
    assert_eq!(got["error"], "TooLarge", "{got}");
    demo.assert_writer_runs(UTF8, 1, "read once, in process");
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_reads_keep_their_limits_and_our_own_copy_reads_back() {
    read_limits(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_reads_keep_their_limits_and_our_own_copy_reads_back() {
    read_limits(B::X);
}

/// A read right after a copy, before its echo, reads the copy, not the app it replaced.
fn a_read_right_after_a_copy_reads_the_copy(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    let at = demo.app.line_count();
    peer.set_selection(
        Selection::Clipboard,
        b.via(),
        &[Offer::text(UTF8, "the peer's")],
    );
    demo.change(at, "offered", None);
    let at = demo.send(json!({
        "cmd": "copy-then-read", "selection": Selection::Clipboard,
        "offers": [Offer::text(UTF8, "the app's")], "mimes": [UTF8],
    }));
    let got = demo.wait(at, "the read", |v| {
        v["ev"] == "read" || v["ev"] == "read-error"
    });
    assert_eq!(got["text"], "the app's", "{got}");
    demo.no_errors();
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_a_read_right_after_a_copy_reads_the_copy() {
    a_read_right_after_a_copy_reads_the_copy(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_a_read_right_after_a_copy_reads_the_copy() {
    a_read_right_after_a_copy_reads_the_copy(B::X);
}

// ---- 11. Teardown ----

/// Quitting with a copy held and a large read in flight tears everything down before `run`
/// returns, and the app exits cleanly.
fn quit_mid_read(b: B) {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, b) else {
        return;
    };
    let peer = s.peer(b.display());
    if b == B::D {
        demo.click(&s);
    }
    let at = demo.app.line_count();
    peer.set_selection(
        Selection::Clipboard,
        b.via(),
        &[Offer {
            mime: PNG.into(),
            data: Payload::Pattern {
                len: 16 << 20,
                seed: 43,
            },
        }],
    );
    demo.change(at, "offered", None);
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Primary, &[Offer::text(UTF8, "held")]);
    if b != B::D {
        demo.change(at, "holding", Some(copy));
    }
    demo.send(json!({ "cmd": "read", "selection": Selection::Clipboard, "mimes": [PNG] }));
    let down = demo.quit("quit");
    assert_eq!(down["bound"], true, "{down}");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_control_quit_mid_read_tears_down_cleanly() {
    quit_mid_read(B::W);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_data_device_quit_mid_read_tears_down_cleanly() {
    let s = Session::start();
    let Some(demo) = Demo::start(&s, B::D) else {
        return;
    };
    let peer = s.peer(Display::Wayland);
    demo.click(&s);
    let at = demo.app.line_count();
    peer.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[Offer {
            mime: PNG.into(),
            data: Payload::Pattern {
                len: 16 << 20,
                seed: 43,
            },
        }],
    );
    demo.change(at, "offered", None);
    demo.send(json!({ "cmd": "read", "selection": Selection::Clipboard, "mimes": [PNG] }));
    let down = demo.quit("quit");
    assert_eq!(down["bound"], true, "{down}");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_quit_mid_read_tears_down_cleanly() {
    quit_mid_read(B::X);
}

/// With drag and drop attached too, drag and drop's shutdown runs the clipboard's part: the
/// hand-off to the clipboard manager.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_drag_and_drops_shutdown_hands_the_clipboard_over_too() {
    let s = Session::start();
    let manager = s.peer(Display::X11);
    manager.clipboard_manager(true);
    let demo = Demo::start_with(&s, Display::X11, &["--with-dnd"], &[], Some(B::X)).expect("X11");
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "kept")]);
    demo.change(at, "holding", Some(copy));
    let down = demo.quit("quit-dnd");
    assert_eq!(down["bound"], true, "{down}");
    manager.wait_event(
        "manager-saved",
        |e| matches!(e, Event::ManagerSaved { saved, .. } if saved == &[UTF8.to_owned()]),
    );
    let reader = s.peer(Display::X11);
    assert_eq!(
        reader
            .read(Selection::Clipboard, Via::X11, UTF8)
            .text
            .as_deref(),
        Some("kept")
    );
}

/// `Tx::quit` alone: Rune runs `App::on_quit` before any window closes, and the shutdown there
/// hands the clipboard over as an explicit one does.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_on_quit_shuts_down_and_hands_the_clipboard_over() {
    let s = Session::start();
    let manager = s.peer(Display::X11);
    manager.clipboard_manager(true);
    let demo = Demo::start(&s, B::X).expect("X11");
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "kept")]);
    demo.change(at, "holding", Some(copy));
    let down = demo.quit("quit-only");
    assert_eq!(
        (&down["why"], &down["hand_off"]),
        (&json!("on-quit"), &json!("Saved")),
        "{down}"
    );
    let saved = manager.wait_event("manager-saved", |e| matches!(e, Event::ManagerSaved { .. }));
    let Event::ManagerSaved { saved, refused } = saved else {
        unreachable!()
    };
    assert_eq!(
        (saved, refused),
        (vec![UTF8.to_owned()], Vec::<String>::new())
    );
    let reader = s.peer(Display::X11);
    assert_eq!(
        reader
            .read(Selection::Clipboard, Via::X11, UTF8)
            .text
            .as_deref(),
        Some("kept")
    );
}

/// The normal way out: the last window closes, `App::on_quit` runs in no window, and the
/// shutdown in that window's `on_close` hands the clipboard over.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_closing_the_last_window_shuts_down_and_hands_the_clipboard_over() {
    let s = Session::start();
    let manager = s.peer(Display::X11);
    manager.clipboard_manager(true);
    let mut demo = Demo::start(&s, B::X).expect("X11");
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "kept")]);
    demo.change(at, "holding", Some(copy));
    s.sway_command(&format!("[con_id={}] kill", demo.window.id));
    let down = demo.wait(at, "the shutdown", |v| v["ev"] == "shutdown");
    assert_eq!(
        (&down["why"], &down["hand_off"]),
        (&json!("close"), &json!("Saved")),
        "{down}"
    );
    let status = demo.app.wait_exit();
    assert!(status.success(), "the app exited with {status}");
    demo.no_errors();
    let saved = manager.wait_event("manager-saved", |e| matches!(e, Event::ManagerSaved { .. }));
    let Event::ManagerSaved { saved, refused } = saved else {
        unreachable!()
    };
    assert_eq!(
        (saved, refused),
        (vec![UTF8.to_owned()], Vec::<String>::new())
    );
    let reader = s.peer(Display::X11);
    assert_eq!(
        reader
            .read(Selection::Clipboard, Via::X11, UTF8)
            .text
            .as_deref(),
        Some("kept")
    );
}

/// The X11 connection fails: the held copy is lost as failed, and the diagnostic reaches the
/// app's handler, which may call the clipboard back (it asks for the status) while the app
/// lives on.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn x11_a_failed_connection_loses_the_copy_and_the_diagnostic_may_call_back() {
    let s = Session::start();
    let demo = Demo::start(&s, B::X).expect("X11");
    let at = demo.app.line_count();
    let copy = demo.copy_offers(Selection::Clipboard, &[Offer::text(UTF8, "a")]);
    demo.change(at, "holding", Some(copy));
    let at = demo.send(json!({ "cmd": "break" }));
    assert_eq!(
        demo.wait(at, "the break", |v| v["ev"] == "broke")["ok"],
        true
    );
    let lost = demo.change(at, "lost", Some(copy));
    assert_eq!(lost["why"], "failed", "{lost}");
    let diag = demo.wait(at, "the diagnostic", |v| v["ev"] == "diagnostic");
    assert_eq!(diag["route"], "X11", "{diag}");
    assert_eq!(demo.state(copy), "Some(Lost(Failed))", "the app lives on");
    let down = demo.quit("quit");
    assert_eq!(down["bound"], true, "{down}");
}

// ---- 12. Through Xwayland (XW) ----

/// libwayland < 1.23 and no data control, simulated: the app's clipboard goes through
/// Xwayland. Its copy reaches an X11 peer and a Wayland peer, and it reads the Wayland peer's
/// copy back. wlroots bridges the clipboard only while an Xwayland window has focus, so the
/// X11 peer's window has it (see the module docs: wlroots never takes this route; mutter's
/// bridge is a hand test).
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn wayland_without_any_wayland_route_copies_and_pastes_through_xwayland() {
    let s = Session::start();
    let x_peer = s.peer(Display::X11);
    let demo = Demo::start_with(
        &s,
        Display::Wayland,
        &["--as-if-libwayland-1-22", "--as-if-no-data-control"],
        &[Display::X11],
        None,
    )
    .expect("the app");
    let a = &demo.attached;
    assert_eq!(
        (&a["route"], &a["xwayland"], &a["backend"]),
        (&json!("X11"), &json!(true), &json!("wayland")),
        "{a}"
    );
    x_peer.map("p2 x11 peer");
    let windows = s.wait_windows(&[demo.app.pid(), x_peer.pid()]);
    let x_window = windows
        .iter()
        .find(|w| w.pid == x_peer.pid())
        .expect("the X11 peer's window");
    s.focus(x_window);
    x_peer.wait_event("focus", |e| matches!(e, Event::Focus { focused: true }));
    let wl_peer = s.peer(Display::Wayland);
    let uris = ["file:///tmp/a", "file:///tmp/b"];
    let at = demo.app.line_count();
    let copy = demo.copy(json!({ "cmd": "copy-files", "uris": uris, "cut": true, "text": null }));
    demo.change(at, "holding", Some(copy));
    let want = [GNOME, URIS, KDE].map(str::to_owned).to_vec();
    wait_targets(&x_peer, Selection::Clipboard, Via::X11, Some(&want));
    wait_targets(
        &wl_peer,
        Selection::Clipboard,
        Via::DataControl,
        Some(&want),
    );
    for (peer, via) in [(&x_peer, Via::X11), (&wl_peer, Via::DataControl)] {
        assert_eq!(
            peer.read(Selection::Clipboard, via, GNOME).text.as_deref(),
            Some("cut\nfile:///tmp/a\nfile:///tmp/b"),
            "{via:?}"
        );
        assert_eq!(
            peer.read(Selection::Clipboard, via, URIS).text.as_deref(),
            Some("file:///tmp/a\r\nfile:///tmp/b\r\n")
        );
    }
    let at = demo.app.line_count();
    wl_peer.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[
            Offer::text(GNOME, "copy\nfile:///tmp/c"),
            Offer::text(UTF8, "the Wayland peer's"),
        ],
    );
    demo.change(at, "lost", Some(copy));
    demo.change(at, "offered", None);
    let files = demo.read_files(Selection::Clipboard);
    assert_eq!(
        (&files["uris"], &files["cut"]),
        (&json!(["file:///tmp/c"]), &json!(false)),
        "{files}"
    );
    // Through the bridge, UTF-8 text goes by its X11 name.
    let got = demo.read(Selection::Clipboard, &[UTF8, "UTF8_STRING"], None);
    assert_eq!(
        (&got["mime"], &got["text"]),
        (&json!("UTF8_STRING"), &json!("the Wayland peer's")),
        "{got}"
    );
    demo.no_errors();
}

// ---- 13. Nautilus (opt-in) ----

/// Nautilus in the nested sway: a file it copies pastes into the app as files, and the app's
/// cut pastes into Nautilus as a move. Opt-in, with Nautilus installed:
/// `STRATA_TEST_NAUTILUS=1 mise run test-platform p2::nautilus`.
fn nautilus_round_trip(display: Display) {
    if std::env::var_os("STRATA_TEST_NAUTILUS").is_none() {
        eprintln!("skipped: set STRATA_TEST_NAUTILUS=1 to run the Nautilus round trip");
        return;
    }
    let s = Session::start();
    let home = s.runtime_dir().join("home");
    let src = home.join("from-nautilus");
    let dst = home.join("into-nautilus");
    for d in [&src, &dst] {
        std::fs::create_dir_all(d).expect("a folder");
    }
    std::fs::write(src.join("alpha.txt"), "alpha").expect("a file");
    let theirs = home.join("theirs.txt");
    std::fs::write(&theirs, "theirs").expect("a file");
    let demo = Demo::start_with(&s, display, &[], &[], None).expect("the app");
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
        .arg(&src);
    let nautilus = s.spawn("nautilus", cmd);
    let window = s.wait_window(nautilus.pid());
    // Nautilus lists the folder some time after its window maps, and a copy before the
    // selection settles names no file: select all, then copy, until a read finds the file
    // (both keys are safe to repeat).
    let uri = mime::file_uri(&src.join("alpha.txt"));
    let deadline = Instant::now() + timeout() * 2;
    let files = loop {
        s.focus(&window);
        std::thread::sleep(Duration::from_millis(1000));
        s.input().pointer_move(window.at(0.5, 0.5));
        s.input().key("ctrl+a");
        std::thread::sleep(Duration::from_millis(500));
        let at = demo.app.line_count();
        s.input().key("ctrl+c");
        let wait = Instant::now() + Duration::from_secs(3);
        let heard = loop {
            if demo.app.lines()[at..]
                .iter()
                .any(|l| l.contains("\"offered\""))
            {
                break true;
            }
            if Instant::now() > wait {
                break false;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let files = heard.then(|| demo.read_files(Selection::Clipboard));
        if let Some(f) = files.filter(|f| f["uris"] == json!([uri])) {
            break f;
        }
        assert!(
            Instant::now() < deadline,
            "the app never read Nautilus's copy of the file"
        );
    };
    assert_eq!(
        (&files["uris"], &files["cut"]),
        (&json!([uri]), &json!(false)),
        "{files}"
    );

    // The app cuts a file; Nautilus pastes it into another folder as a move.
    let at = demo.app.line_count();
    let copy = demo.copy(json!({
        "cmd": "copy-files", "uris": [mime::file_uri(&theirs)], "cut": true, "text": null,
    }));
    demo.change(at, "holding", Some(copy));
    drop(nautilus);
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
        .arg(&dst);
    let nautilus = s.spawn("nautilus-2", cmd);
    let window = s.wait_window_where("Nautilus on the target folder", |w| w.pid == nautilus.pid());
    s.focus(&window);
    std::thread::sleep(Duration::from_millis(1500));
    s.input().pointer_move(window.at(0.5, 0.5));
    s.input().key("ctrl+v");
    // Once more if nothing moved: a paste before Nautilus listed the folder goes nowhere, and a
    // second paste of a file already moved can't move it twice.
    let again = Instant::now() + Duration::from_secs(4);
    let mut pasted_again = false;
    let deadline = Instant::now() + timeout();
    while !(dst.join("theirs.txt").exists() && !theirs.exists()) {
        if !pasted_again && Instant::now() > again {
            s.focus(&window);
            s.input().key("ctrl+v");
            pasted_again = true;
        }
        assert!(
            Instant::now() < deadline,
            "Nautilus didn't move the cut file: in target {}, at source {}",
            dst.join("theirs.txt").exists(),
            theirs.exists()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn nautilus_wayland_round_trip() {
    nautilus_round_trip(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn nautilus_x11_round_trip() {
    nautilus_round_trip(Display::X11);
}

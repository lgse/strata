//! Copy and paste with any MIME types between a Rune window and other apps: a demo of the
//! `clipboard` feature, and the app strata-platform's session tests drive
//! (`tests/session/p2.rs`). Commands arrive as JSON lines on stdin; what happens goes to
//! stdout as JSON lines (`{"ev": …}`).
//!
//! Commands:
//! - `{"cmd":"copy","selection":"clipboard"|"primary","offers":[Offer],"keep_ms":n|null}`:
//!   copies the offers, each with a lazy writer that logs `writer` when it runs (those named in
//!   `"hang":[…]` then never return);
//!   `keep_ms` is how long the exit waits for a clipboard manager (`null`: don't hand over;
//!   absent: the default);
//! - `{"cmd":"arm","offers":[Offer]}`: what Ctrl+C in the window copies;
//! - `{"cmd":"copy-then-read","selection":…,"offers":[Offer],"mimes":[…]}`: a copy, then a read,
//!   in one handler; `{"cmd":"copy-then-clear",…}`: a copy and its clear, in one handler;
//! - `{"cmd":"copy-files","uris":[…],"cut":bool,"text":"…"|null}`;
//! - `{"cmd":"clear","copy":n}`, `{"cmd":"state","copy":n}`, `{"cmd":"offered","selection":…}`;
//! - `{"cmd":"clear-later","copy":n,"ms":n}`: holds the UI thread `ms`, then clears;
//! - `{"cmd":"read","selection":…,"mimes":[…],"max_bytes":n}`, `{"cmd":"read-files","selection":…}`;
//! - `{"cmd":"rune-copy","text":"…"}`: Rune's own `Tx::copy`;
//! - `{"cmd":"break"}`: the platform ends the clipboard's connection (a test switch);
//! - `{"cmd":"quit"}`: the clipboard's shutdown, then `Tx::quit`; `{"cmd":"quit-dnd"}`: drag
//!   and drop's shutdown instead (with `--with-dnd`); `{"cmd":"quit-only"}`: `Tx::quit` alone,
//!   so `App::on_quit` shuts down.
//!
//! Flags: `--title T`, `--with-dnd` (drag and drop attached too, sharing the substrate),
//! and the test switches `--as-if-no-data-control` (a compositor without data control, as
//! mutter: the data device) and
//! `--as-if-libwayland-1-22` (no route to winit's display).

#[cfg(feature = "clipboard")]
fn main() {
    demo::main();
}

#[cfg(not(feature = "clipboard"))]
fn main() {
    eprintln!("run with --features clipboard");
}

#[cfg(feature = "clipboard")]
mod demo {
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use rune::{App, AppHandle, AppSpec, Cx, El, KeyEvent, Tx, WindowHandle, col, text};
    use serde_json::{Value, json};
    use strata_platform::clipboard::{
        self, Change, Clipboard, Content, CopyId, Files, Limits, Selection, TestRoute,
    };
    use strata_test_peer::protocol::{self, Offer, fnv1a64};

    struct State {
        clip: Clipboard<State>,
        armed: Vec<Offer>,
        /// Kept alive beside the clipboard, sharing its substrate (`--with-dnd`).
        #[cfg(feature = "dnd")]
        _dnd: Option<strata_platform::dnd::Dnd<State>>,
    }

    fn log(event: &str, fields: Value) {
        let mut line = json!({ "ev": event });
        if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
            obj.extend(fields);
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    fn selection(v: &Value) -> Selection {
        match serde_json::from_value::<protocol::Selection>(v.clone()) {
            Ok(protocol::Selection::Primary) => Selection::Primary,
            _ => Selection::Clipboard,
        }
    }

    fn selection_name(s: Selection) -> &'static str {
        match s {
            Selection::Clipboard => "clipboard",
            Selection::Primary => "primary",
        }
    }

    fn change(c: &Change) -> Value {
        match c {
            Change::Offered { selection, mimes } => {
                json!({ "kind": "offered", "selection": selection_name(*selection), "mimes": mimes })
            }
            Change::Emptied { selection } => {
                json!({ "kind": "emptied", "selection": selection_name(*selection) })
            }
            Change::Holding { selection, copy } => json!({
                "kind": "holding", "selection": selection_name(*selection), "copy": copy.get(),
            }),
            Change::Lost {
                selection,
                copy,
                why,
            } => json!({
                "kind": "lost", "selection": selection_name(*selection), "copy": copy.get(),
                "why": format!("{why:?}").to_lowercase(),
            }),
        }
    }

    fn content(offers: Vec<Offer>) -> Content {
        content_with(offers, &[])
    }

    /// The offers with lazy writers that log `writer`; those of `hang` never return.
    fn content_with(offers: Vec<Offer>, hang: &[String]) -> Content {
        offers.into_iter().fold(Content::new(), |c, offer| {
            let mime = offer.mime.clone();
            let hangs = hang.contains(&mime);
            c.lazy(offer.mime, move || {
                log("writer", json!({ "mime": mime }));
                if hangs {
                    loop {
                        std::thread::park();
                    }
                }
                offer.data.bytes().unwrap_or_default()
            })
        })
    }

    fn copied(r: Result<CopyId, clipboard::Error>) {
        match r {
            Ok(id) => log("copied", json!({ "copy": id.get() })),
            Err(e) => log("copy-error", json!({ "error": e.to_string() })),
        }
    }

    static SHUT_DOWN: AtomicBool = AtomicBool::new(false);

    fn shutdown(why: &str) {
        if SHUT_DOWN.swap(true, Ordering::SeqCst) {
            return;
        }
        let started = Instant::now();
        #[cfg(feature = "dnd")]
        if why == "quit-dnd" {
            let r = strata_platform::dnd::shutdown();
            return log(
                "shutdown",
                json!({ "why": why, "bound": r.as_ref().ok(), "ms": started.elapsed().as_secs_f64() * 1e3 }),
            );
        }
        let r = clipboard::shutdown();
        log(
            "shutdown",
            json!({
                "why": why,
                "bound": r.as_ref().ok().map(|c| c.bound),
                "hand_off": r.as_ref().ok().and_then(|c| c.hand_off).map(|h| format!("{h:?}")),
                "error": r.as_ref().err().map(ToString::to_string),
                "ms": started.elapsed().as_secs_f64() * 1e3,
            }),
        );
    }

    fn command(s: &mut State, cmd: &Value, tx: &mut Tx) {
        let field = |k: &str| cmd.get(k).cloned().unwrap_or(Value::Null);
        let sel = selection(&field("selection"));
        match cmd["cmd"].as_str().unwrap_or_default() {
            "copy" => {
                let offers: Vec<Offer> =
                    serde_json::from_value(field("offers")).unwrap_or_default();
                let hang: Vec<String> = serde_json::from_value(field("hang")).unwrap_or_default();
                let mut c = content_with(offers, &hang);
                if let Some(keep) = cmd.get("keep_ms") {
                    c = c.keep_after_exit(keep.as_u64().map(Duration::from_millis));
                }
                copied(s.clip.copy(sel, c));
            }
            // A copy, then a read of the same selection in the same handler, before any echo.
            "copy-then-read" => {
                let offers: Vec<Offer> =
                    serde_json::from_value(field("offers")).unwrap_or_default();
                copied(s.clip.copy(sel, content(offers)));
                let mimes: Vec<String> = serde_json::from_value(field("mimes")).unwrap_or_default();
                let mimes: Vec<&str> = mimes.iter().map(String::as_str).collect();
                let r = s
                    .clip
                    .read(sel, &mimes, Limits::default(), |_, r, _| match r {
                        Ok(d) => log("read", json!({ "mime": d.mime, "text": d.text() })),
                        Err(e) => log("read-error", json!({ "error": format!("{e:?}") })),
                    });
                if let Err(e) = r {
                    log("read-error", json!({ "error": format!("{e:?}") }));
                }
            }
            // A copy, then its clear in the same handler: on X11, while it waits for its
            // timestamp.
            "copy-then-clear" => {
                let offers: Vec<Offer> =
                    serde_json::from_value(field("offers")).unwrap_or_default();
                let copy = s.clip.copy(sel, content(offers));
                let cleared = copy.as_ref().ok().map(|c| s.clip.clear(*c));
                copied(copy);
                log(
                    "cleared",
                    json!({ "ok": cleared.as_ref().and_then(|r| r.as_ref().ok()) }),
                );
            }
            "arm" => {
                s.armed = serde_json::from_value(field("offers")).unwrap_or_default();
                log("armed", json!({}));
            }
            "copy-files" => {
                let uris: Vec<String> = serde_json::from_value(field("uris")).unwrap_or_default();
                let files = Files::new(uris, field("cut").as_bool().unwrap_or(false));
                let mut c = Content::new().files(&files);
                if let Some(t) = field("text").as_str() {
                    c = c.text(t);
                }
                copied(s.clip.copy(sel, c));
            }
            "clear" => {
                let copy = s
                    .clip
                    .state(CopyId::from_raw(field("copy").as_u64().unwrap_or_default()));
                let r = s
                    .clip
                    .clear(CopyId::from_raw(field("copy").as_u64().unwrap_or_default()));
                log(
                    "cleared",
                    json!({ "was": format!("{copy:?}"), "ok": r.as_ref().ok(), "error": r.err().map(|e| e.to_string()) }),
                );
            }
            // Holds the UI thread first, so the platform's news (another app took the
            // clipboard) waits behind this update: the clear can't know of it.
            "clear-later" => {
                std::thread::sleep(Duration::from_millis(field("ms").as_u64().unwrap_or(500)));
                let copy = CopyId::from_raw(field("copy").as_u64().unwrap_or_default());
                let r = s.clip.clear(copy);
                log(
                    "cleared",
                    json!({ "ok": r.as_ref().ok(), "error": r.err().map(|e| e.to_string()) }),
                );
            }
            "state" => {
                let copy = CopyId::from_raw(field("copy").as_u64().unwrap_or_default());
                log(
                    "state",
                    json!({ "copy": copy.get(), "state": format!("{:?}", s.clip.state(copy)) }),
                );
            }
            "offered" => {
                let o = s.clip.offered(sel);
                log(
                    "offered",
                    json!({
                        "selection": selection_name(sel),
                        "mimes": o.as_ref().map(|o| o.mimes.clone()),
                        "ours": o.and_then(|o| o.ours).map(CopyId::get),
                    }),
                );
            }
            "read" => {
                let mimes: Vec<String> = serde_json::from_value(field("mimes")).unwrap_or_default();
                let mimes: Vec<&str> = mimes.iter().map(String::as_str).collect();
                let mut limits = Limits::default();
                if let Some(max) = field("max_bytes").as_u64() {
                    limits.max_bytes = max as usize;
                }
                let r = s.clip.read(sel, &mimes, limits, |_, r, _| match r {
                    Ok(d) => {
                        let text = (d.bytes.len() <= protocol::INLINE_TEXT)
                            .then(|| std::str::from_utf8(&d.bytes).ok())
                            .flatten();
                        log(
                            "read",
                            json!({ "mime": d.mime, "len": d.bytes.len(), "fnv1a64": fnv1a64(&d.bytes), "text": text }),
                        );
                    }
                    Err(e) => log("read-error", json!({ "error": format!("{e:?}") })),
                });
                if let Err(e) = r {
                    log("read-error", json!({ "error": format!("{e:?}") }));
                }
            }
            "read-files" => {
                let r = s
                    .clip
                    .read_files(sel, Limits::default(), |_, r, _| match r {
                        Ok(f) => log("files", json!({ "uris": f.uris, "cut": f.cut })),
                        Err(e) => log("read-error", json!({ "error": format!("{e:?}") })),
                    });
                if let Err(e) = r {
                    log("read-error", json!({ "error": format!("{e:?}") }));
                }
            }
            "break" => log(
                "broke",
                json!({ "ok": clipboard::provoke_connection_failure() }),
            ),
            "rune-copy" => {
                tx.copy(field("text").as_str().unwrap_or_default());
                log("rune-copied", json!({}));
            }
            why @ ("quit" | "quit-dnd") => {
                shutdown(why);
                tx.quit();
            }
            "quit-only" => tx.quit(),
            other => log("bad-command", json!({ "cmd": other })),
        }
    }

    fn on_key(s: &mut State, ev: &KeyEvent, _: &mut Tx) -> bool {
        log("key", json!({ "key": format!("{:?}", ev.key) }));
        if ev.is_command('c') && !s.armed.is_empty() {
            copied(s.clip.copy(Selection::Clipboard, content(s.armed.clone())));
            return true;
        }
        false
    }

    fn on_close(_: &mut State, _: &mut Tx) {
        shutdown("close");
    }

    fn view(_: &State, _: &mut Cx<State>) -> El {
        col()
            .key("window-button")
            .w_full()
            .h_full()
            .center()
            .role("button")
            .label("Press")
            .child(text("clipboard"))
            .on_press(|_: &mut State| log("press", json!({})))
            .on_layout(|_: &mut State, r: [f32; 4]| log("layout", json!({ "w": r[2], "h": r[3] })))
            .on_window_focus(|_: &mut State, focused: bool, _: &mut Tx| {
                log("focus", json!({ "focused": focused }));
            })
    }

    pub(super) fn main() {
        let args = rune::tooling::extra_args();
        let flag = |f: &str| args.iter().any(|a| a == f);
        let title: &'static str = args
            .iter()
            .position(|a| a == "--title")
            .and_then(|i| args.get(i + 1))
            .map_or("clipboard", |t| Box::leak(t.clone().into_boxed_str()));
        rune::tooling::set_app_id("strata-clipboard-demo");
        let route = TestRoute {
            as_if_no_data_control: flag("--as-if-no-data-control"),
            as_if_libwayland_below_1_23: flag("--as-if-libwayland-1-22"),
        };
        let clip = Clipboard::<State>::new()
            .test_route(route)
            .on_change(|_, c, _| log("change", change(c)));
        // It asks the clipboard back, as an app's handler may: no borrow of the clipboard's may
        // be held while it runs.
        let diag_clip = clip.clone();
        clipboard::on_diagnostic(move |m| {
            let route = diag_clip.status().map(|s| format!("{:?}", s.route));
            log("diagnostic", json!({ "message": m, "route": route }));
        });
        let with_dnd = flag("--with-dnd");
        let state_clip = clip.clone();
        #[cfg(feature = "dnd")]
        let dnd = with_dnd.then(strata_platform::dnd::Dnd::<State>::new);
        #[cfg(not(feature = "dnd"))]
        if with_dnd {
            log(
                "bad-flag",
                json!({ "flag": "--with-dnd needs the dnd feature" }),
            );
        }
        #[cfg(feature = "dnd")]
        let state_dnd = dnd.clone();
        let mut spec = AppSpec::new(title, (640.0, 400.0), || unreachable!("init is set"), view);
        spec.on_key = Some(on_key);
        let app = App::new(spec)
            .init(move || State {
                clip: state_clip,
                armed: Vec::new(),
                #[cfg(feature = "dnd")]
                _dnd: state_dnd,
            })
            .on_quit(|_, _| shutdown("on-quit"))
            .on_close(on_close);
        let handle: AppHandle<State> = app.handle();
        let stdin_handle = handle.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let Ok(cmd) = serde_json::from_str::<Value>(&line) else {
                    log("bad-command", json!({ "line": line }));
                    continue;
                };
                stdin_handle.update(move |s, tx| command(s, &cmd, tx));
            }
        });
        static DRAWN: AtomicBool = AtomicBool::new(false);
        let result = app
            .on_window(move |w: &WindowHandle| {
                #[cfg(feature = "dnd")]
                if let Some(dnd) = &dnd
                    && let Err(e) = dnd.attach(w, &handle)
                {
                    log("dnd-attach-error", json!({ "error": e.to_string() }));
                }
                match clip.attach(w, &handle) {
                    Ok(st) => log(
                        "attached",
                        json!({
                            "backend": st.backend,
                            "route": format!("{:?}", st.route),
                            "hears_unfocused": st.hears_unfocused,
                            "primary": st.primary,
                            "keeps_after_exit": st.keeps_after_exit,
                            "xwayland": st.xwayland,
                            "unsupported": st.unsupported,
                        }),
                    ),
                    Err(e) => log("attach-error", json!({ "error": e.to_string() })),
                }
            })
            .on_ready(|_: &WindowHandle| log("ready", json!({})))
            .on_frame_drawn(|_| {
                if !DRAWN.swap(true, Ordering::Relaxed) {
                    log("frame", json!({}));
                }
            })
            .try_run();
        log(
            "exit",
            json!({ "shut_down": SHUT_DOWN.load(Ordering::SeqCst) }),
        );
        if let Err(error) = result {
            log("exit-error", json!({ "error": error }));
            std::process::exit(1);
        }
    }
}

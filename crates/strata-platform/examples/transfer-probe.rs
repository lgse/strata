//! A Rune window with P0's transfer substrate attached, for strata-platform's session tests
//! (`tests/session/p0.rs`). It takes commands as JSON lines on stdin and reports what it sees
//! as JSON lines on stdout. P0's substrate is internal: this example reaches it through the
//! hidden `strata_platform::transfer` module, which is no public API.
//!
//! Commands:
//! - `{"cmd":"own","via":"data-control"|"data-device"|"x11","selection":"clipboard"|"primary","offers":[Offer]}`:
//!   takes the selection with lazy writers that log `writer` when they run;
//! - `{"cmd":"targets","via":…,"selection":…}` and `{"cmd":"read","via":…,"selection":…,"mime":…}`;
//! - `{"cmd":"hide"}`, `{"cmd":"show"}`, `{"cmd":"quit"}`;
//! - `{"cmd":"open-window","key":k}` opens window `k` (title `p0 k`), its own app, which
//!   attaches from `App::on_window` with a turn from that app, as a later P1 or P2 window does
//!   (`attached` carries its `key`); `{"cmd":"close-window","key":k}` closes it
//!   (`window-closed`, from its `on_close`).
//!
//! Drags into the window are accepted with their first type and Copy; on the drop every type
//! is read, then the drop is finished.
//!
//! Flags: `--title T`, `--own-data-control` (data control on a connection of its own),
//! `--data-control ext|wlr` (which protocol first), `--as-if-no-data-control` (a compositor
//! without data control, as mutter: a test switch), `--x11-fallback` (X11 selections through
//! Xwayland when Wayland has no route, as the clipboard asks), `--no-shared` (don't bind on
//! winit's display), `--as-if-libwayland-1-22` (behave as on libwayland < 1.23, which has no route to
//! winit's display: a test switch), `--answer-ask-with-move` (prefer ask on enter, answer the
//! drop with move), `--clear-actions-after-drop` (set no actions once dropped), `--host` (the
//! main window attaches nothing: only the windows `open-window` opens do). The `break-own` command makes the compositor end the Wayland connection
//! of our own with a protocol error (a test switch); `shutdown-elsewhere` calls the shutdown
//! from another thread, which must refuse; `break-x11` has the X server end the X11
//! connection (a test switch).

#[cfg(feature = "_transfer")]
fn main() {
    probe::main();
}

#[cfg(not(feature = "_transfer"))]
fn main() {
    eprintln!("run with --features clipboard (or dnd, or foreign-parent)");
}

#[cfg(feature = "_transfer")]
mod probe {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Instant;

    use rune::{App, AppHandle, AppSpec, Cx, El, Tx, WindowHandle, WindowSpec, col, text};
    use serde_json::{Value, json};
    use strata_platform::transfer::{
        Action, DataControl, DragOffer, Event, Prefer, ReadLimits, Route, Selection, SourceId,
        SourceSpec, Transfer, Turn, Wants, Writer,
    };
    use strata_test_peer::protocol::{self, Offer, Via, fnv1a64};

    #[derive(Default)]
    struct State {
        presses: u32,
    }

    /// A handle and the newest selection offer, kept past the event loop on purpose: `main`
    /// drops them after `run` returns, once winit has closed its display, so the quit and
    /// close tests check that dropping a shut-down substrate then makes no libwayland call
    /// (S2's late drops crashed every run).
    static LATE: Mutex<Option<(Transfer, Option<strata_platform::transfer::SelectionOffer>)>> =
        Mutex::new(None);

    fn log(event: &str, fields: Value) {
        let mut line = json!({ "ev": event });
        if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
            obj.extend(fields);
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    /// The bytes a read got, as the peer reports them.
    fn received(mime: &str, bytes: &[u8]) -> Value {
        let text = (bytes.len() <= protocol::INLINE_TEXT)
            .then(|| std::str::from_utf8(bytes).ok())
            .flatten();
        json!({ "mime": mime, "len": bytes.len(), "fnv1a64": fnv1a64(bytes), "text": text })
    }

    fn route(via: Via) -> Route {
        match via {
            Via::DataDevice => Route::DataDevice,
            Via::DataControl => Route::DataControl,
            Via::X11 => Route::X11,
        }
    }

    fn via_name(route: Route) -> &'static str {
        match route {
            Route::DataDevice => "data-device",
            Route::DataControl => "data-control",
            Route::X11 => "x11",
        }
    }

    fn selection(s: protocol::Selection) -> Selection {
        match s {
            protocol::Selection::Clipboard => Selection::Clipboard,
            protocol::Selection::Primary => Selection::Primary,
        }
    }

    fn selection_name(s: Selection) -> &'static str {
        match s {
            Selection::Clipboard => "clipboard",
            Selection::Primary => "primary",
        }
    }

    /// This app's subscription to the substrate, once attached. The substrate itself is
    /// `Transfer::current()` until it is shut down.
    static ATTACHED: OnceLock<strata_platform::transfer::Subscription> = OnceLock::new();
    static SHUT_DOWN: AtomicBool = AtomicBool::new(false);

    fn shutdown(why: &str) {
        if SHUT_DOWN.swap(true, Ordering::SeqCst) {
            return;
        }
        let started = Instant::now();
        let was = Transfer::shutdown();
        log(
            "shutdown",
            json!({
                "why": why,
                "bound": was.as_ref().ok(),
                "error": was.as_ref().err().map(ToString::to_string),
                "ms": started.elapsed().as_secs_f64() * 1e3,
            }),
        );
    }

    fn transfer() -> Option<Transfer> {
        Transfer::current()
    }

    /// Attaches `w` with a turn from its own app, `handle`. Events reach the main window's app
    /// whichever window bound the substrate, so they outlive any window `open-window` opened.
    fn attach<S: 'static>(
        w: &WindowHandle,
        handle: &AppHandle<S>,
        wants: Wants,
        key: Option<&str>,
    ) {
        let turn_handle = handle.clone();
        let turn: Turn = Arc::new(move |f| {
            turn_handle.update_if(move |_, _| {
                f();
                false
            });
        });
        match Transfer::attach(w, turn, wants) {
            Ok((t, fresh)) => {
                let s = t.status();
                log(
                    "attached",
                    json!({
                        "fresh": fresh,
                        "backend": s.backend,
                        "shared": s.shared,
                        "own": s.own,
                        "data_control": s.data_control,
                        "unsupported": s.unsupported.as_ref().map(ToString::to_string),
                        "x11": s.x11,
                        "key": key,
                    }),
                );
                if fresh {
                    *LATE.lock().unwrap_or_else(|e| e.into_inner()) = Some((t.clone(), None));
                    let notify = HANDLE.get().expect("set before the event loop").clone();
                    let sub = t.subscribe(move || notify.update_if(|_, _| events()));
                    let _ = ATTACHED.set(sub);
                    // Events heard while binding wait for a dispatch.
                    let _ = t.dispatch();
                    events();
                }
            }
            Err(e) => log("attach-error", json!({ "error": e.to_string() })),
        }
    }

    /// The drag over the window, while one is.
    static DRAG: Mutex<Option<DragOffer>> = Mutex::new(None);
    /// `--answer-ask-with-move`: prefer ask on enter, then answer the drop with move.
    static ANSWER_ASK_WITH_MOVE: AtomicBool = AtomicBool::new(false);
    /// `--clear-actions-after-drop`: after the drop, set no actions (which must be refused).
    static CLEAR_ACTIONS_AFTER_DROP: AtomicBool = AtomicBool::new(false);

    fn events() -> bool {
        let Some(sub) = ATTACHED.get() else {
            return false;
        };
        for event in sub.drain() {
            match event {
                Event::SelectionChanged {
                    selection,
                    route,
                    offer,
                } => {
                    log(
                        "selection",
                        json!({
                            "via": via_name(route),
                            "selection": selection_name(selection),
                            "mimes": offer.as_ref().map(|o| o.mimes().to_vec()),
                        }),
                    );
                    if let Some((_, kept)) = LATE.lock().unwrap_or_else(|e| e.into_inner()).as_mut()
                    {
                        *kept = offer;
                    }
                }
                Event::SourceAsked { source, mime } => {
                    log("asked", json!({ "source": source.get(), "mime": mime }))
                }
                Event::SourceCancelled { source } => {
                    log("cancelled", json!({ "source": source.get() }))
                }
                Event::DragEnter {
                    window,
                    offer,
                    x,
                    y,
                } => {
                    log(
                        "drag-enter",
                        json!({ "window": window, "mimes": offer.mimes(), "x": x, "y": y }),
                    );
                    let first = offer.mimes().first().cloned();
                    let accepted = offer.accept(first.as_deref());
                    let actions = if ANSWER_ASK_WITH_MOVE.load(Ordering::SeqCst) {
                        offer.set_actions(
                            &[Action::Copy, Action::Move, Action::Ask],
                            Some(Action::Ask),
                        )
                    } else {
                        offer.set_actions(&[Action::Copy], Some(Action::Copy))
                    };
                    if let Err(e) = accepted.and(actions) {
                        log("drag-error", json!({ "error": e.to_string() }));
                    }
                    *DRAG.lock().unwrap_or_else(|e| e.into_inner()) = Some(offer);
                }
                Event::DragMotion { x, y } => {
                    log("drag-motion", json!({ "x": x, "y": y }));
                    if let Some(offer) = DRAG.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                        let _ = offer.accept(offer.mimes().first().map(String::as_str));
                    }
                }
                Event::DragLeave => log("drag-leave", json!({})),
                Event::DragActions { source, chosen } => log(
                    "drag-actions",
                    json!({ "source": format!("{source:?}"), "chosen": chosen.map(|a| format!("{a:?}")) }),
                ),
                Event::Drop => {
                    log("drop", json!({}));
                    if let Some(offer) = DRAG.lock().unwrap_or_else(|e| e.into_inner()).take() {
                        if ANSWER_ASK_WITH_MOVE.load(Ordering::SeqCst) {
                            // Answer the ask with move, which a source of [copy, ask] lacks: the
                            // compositor keeps ask, so the finish must be refused.
                            let r =
                                offer.set_actions(&[Action::Move, Action::Ask], Some(Action::Move));
                            log("ask-answered", json!({ "ok": r.is_ok() }));
                        }
                        if CLEAR_ACTIONS_AFTER_DROP.load(Ordering::SeqCst) {
                            let r = offer.set_actions(&[], None);
                            log(
                                "actions-cleared",
                                json!({ "ok": r.is_ok(), "error": r.err().map(|e| e.to_string()) }),
                            );
                        }
                        read_drop(offer);
                    }
                }
                Event::Failed { message } => log("failed", json!({ "message": message })),
            }
        }
        false
    }

    static HANDLE: OnceLock<AppHandle<State>> = OnceLock::new();

    /// Reads every type of the drop, then finishes it on the UI thread.
    fn read_drop(offer: DragOffer) {
        let mimes = offer.mimes().to_vec();
        let left = Arc::new(AtomicUsize::new(mimes.len()));
        for mime in mimes {
            let (left, offer2) = (left.clone(), offer.clone());
            let m = mime.clone();
            offer.read(&mime, ReadLimits::default(), move |r| {
                match r {
                    Ok(bytes) => log("dropped", received(&m, &bytes)),
                    Err(e) => log(
                        "drop-read-error",
                        json!({ "mime": m, "error": e.to_string() }),
                    ),
                }
                if left.fetch_sub(1, Ordering::SeqCst) == 1
                    && let Some(h) = HANDLE.get()
                {
                    h.update_if(move |_, _| {
                        match offer2.finish() {
                            Ok(()) => log("drop-finished", json!({})),
                            Err(e) => log("drop-error", json!({ "error": e.to_string() })),
                        }
                        false
                    });
                }
            });
        }
    }

    fn spec(offers: Vec<Offer>) -> SourceSpec {
        offers.into_iter().fold(SourceSpec::new(), |spec, offer| {
            let mime = offer.mime.clone();
            spec.with(
                offer.mime,
                Writer::lazy(move || {
                    log("writer", json!({ "mime": mime }));
                    offer.data.bytes().unwrap_or_default()
                }),
            )
        })
    }

    fn command(line: &str, tx: &mut Tx, handle: &AppHandle<State>) {
        let Ok(cmd) = serde_json::from_str::<Value>(line) else {
            return log("bad-command", json!({ "line": line }));
        };
        let field = |k: &str| cmd.get(k).cloned().unwrap_or(Value::Null);
        let via = serde_json::from_value::<Via>(field("via")).map(route);
        let sel = serde_json::from_value::<protocol::Selection>(field("selection"))
            .map(selection)
            .unwrap_or(Selection::Clipboard);
        if cmd["cmd"] == "open-window" {
            return open_window(cmd["key"].as_str().unwrap_or("w1"));
        }
        let Some(t) = transfer() else {
            return log("command-error", json!({ "error": "not attached" }));
        };
        match cmd["cmd"].as_str().unwrap_or_default() {
            "own" => {
                let Ok(via) = via else {
                    return log("command-error", json!({ "error": "no via" }));
                };
                let offers: Vec<Offer> =
                    serde_json::from_value(field("offers")).unwrap_or_default();
                // The click or key that asked for the copy must be in before its serial is read.
                let _ = t.dispatch();
                match t.set_selection(sel, via, spec(offers)) {
                    Ok(id) => log(
                        "owned",
                        json!({ "source": id.get(), "via": via_name(via), "serial": t.latest_serial() }),
                    ),
                    Err(e) => log("own-error", json!({ "error": e.to_string() })),
                }
            }
            "targets" => {
                let Ok(via) = via else {
                    return log("command-error", json!({ "error": "no via" }));
                };
                let mimes = t.selection(sel, via).map(|o| o.mimes().to_vec());
                log("targets", json!({ "via": via_name(via), "mimes": mimes }));
            }
            "read" => {
                let Ok(via) = via else {
                    return log("command-error", json!({ "error": "no via" }));
                };
                let mime = cmd["mime"].as_str().unwrap_or_default().to_owned();
                match t.selection(sel, via) {
                    Some(offer) => {
                        offer.read(&mime.clone(), ReadLimits::default(), move |r| match r {
                            Ok(bytes) => log("read", received(&mime, &bytes)),
                            Err(e) => log(
                                "read-error",
                                json!({ "mime": mime, "error": e.to_string() }),
                            ),
                        })
                    }
                    None => log("read-error", json!({ "mime": mime, "error": "no offer" })),
                }
            }
            "owns" => {
                let source = cmd["source"].as_u64().unwrap_or_default();
                log(
                    "owns",
                    json!({ "source": source, "owns": t.owns(SourceId::from_raw(source)) }),
                );
            }
            "break-x11" => log(
                "broke-x11",
                json!({ "ok": t.provoke_x11_connection_failure() }),
            ),
            "break-own" => log(
                "broke-own",
                json!({ "ok": t.provoke_protocol_error_on_own_connection() }),
            ),
            // Shutdown from a thread that isn't the UI thread: refused, the substrate stays.
            "shutdown-elsewhere" => {
                let r = std::thread::spawn(Transfer::shutdown).join();
                let refused = matches!(r, Ok(Err(_)));
                log(
                    "shutdown-elsewhere",
                    json!({ "refused": refused, "still_bound": Transfer::current().is_some() }),
                );
            }
            "close-window" => {
                let key = cmd["key"].as_str().unwrap_or("w1");
                match WINDOWS.with(|w| w.borrow_mut().remove(key)) {
                    Some(w) => w.close(),
                    None => log(
                        "command-error",
                        json!({ "error": format!("no window {key}") }),
                    ),
                }
            }
            "hide" => handle.hide(),
            "show" => handle.show(),
            "quit" => {
                shutdown("quit");
                tx.quit();
            }
            other => log("bad-command", json!({ "cmd": other })),
        }
    }

    thread_local! {
        /// The windows `open-window` opened, by key.
        static WINDOWS: RefCell<HashMap<String, WindowHandle>> = RefCell::new(HashMap::new());
    }

    /// The wants of the main window, for the windows `open-window` opens.
    static WANTS: OnceLock<Wants> = OnceLock::new();

    struct Extra {
        key: String,
    }

    fn extra_view(s: &Extra, _: &mut Cx<Extra>) -> El {
        col()
            .w_full()
            .h_full()
            .center()
            .child(text(format!("window {}", s.key)))
    }

    fn open_window(key: &str) {
        if WINDOWS.with(|w| w.borrow().contains_key(key)) {
            return log(
                "command-error",
                json!({ "error": format!("window {key} is open") }),
            );
        }
        let title: &'static str = Box::leak(format!("p0 {key}").into_boxed_str());
        let app = App::new(AppSpec::new(
            title,
            (320.0, 200.0),
            || Extra { key: String::new() },
            extra_view,
        ))
        .on_close(|s: &mut Extra, _: &mut Tx| log("window-closed", json!({ "key": s.key })));
        let handle = app.handle();
        let k = key.to_owned();
        let wants = WANTS.get().copied().unwrap_or_default();
        let app = app.on_window(move |w: &WindowHandle| attach(w, &handle, wants, Some(&k)));
        let spec = WindowSpec::from_app(key, app).state(Extra {
            key: key.to_owned(),
        });
        let window = rune::open_window(spec);
        WINDOWS.with(|w| w.borrow_mut().insert(key.to_owned(), window));
    }

    fn view(s: &State, _: &mut Cx<State>) -> El {
        col()
            .key("window-button")
            .w_full()
            .h_full()
            .center()
            .role("button")
            .label("Press")
            .child(text(format!("presses {}", s.presses)))
            .on_press(|s: &mut State| {
                s.presses += 1;
                log("press", json!({ "n": s.presses }));
            })
            .on_layout(|_: &mut State, r: [f32; 4]| log("layout", json!({ "w": r[2], "h": r[3] })))
            .on_window_focus(|_: &mut State, focused: bool, _: &mut Tx| {
                log("focus", json!({ "focused": focused }));
            })
    }

    fn on_close(_: &mut State, _: &mut Tx) {
        shutdown("close");
    }

    pub(super) fn main() {
        let args = rune::tooling::extra_args();
        let flag = |f: &str| args.iter().any(|a| a == f);
        let value = |f: &str| {
            args.iter()
                .position(|a| a == f)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        let title = args
            .iter()
            .position(|a| a == "--title")
            .and_then(|i| args.get(i + 1))
            .map_or("transfer-probe", |t| t.as_str());
        ANSWER_ASK_WITH_MOVE.store(flag("--answer-ask-with-move"), Ordering::SeqCst);
        CLEAR_ACTIONS_AFTER_DROP.store(flag("--clear-actions-after-drop"), Ordering::SeqCst);
        let wants = Wants {
            shared: !flag("--no-shared"),
            data_control: if flag("--own-data-control") {
                DataControl::Own
            } else {
                DataControl::SharedElseOwn
            },
            prefer: match value("--data-control").as_deref() {
                Some("wlr") => Prefer::Wlr,
                _ => Prefer::Ext,
            },
            x11_fallback: flag("--x11-fallback"),
            as_if_libwayland_below_1_23: flag("--as-if-libwayland-1-22"),
            as_if_no_data_control: flag("--as-if-no-data-control"),
        };
        rune::tooling::set_app_id("strata-transfer-probe");
        let app =
            App::new(AppSpec::new(title, (640.0, 400.0), State::default, view)).on_close(on_close);
        let handle = app.handle();
        let _ = HANDLE.set(handle.clone());
        let stdin_handle = handle.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let h = stdin_handle.clone();
                stdin_handle.update(move |_, tx| command(&line, tx, &h));
            }
        });
        let attach_handle = handle.clone();
        let _ = WANTS.set(wants);
        let host = flag("--host");
        static DRAWN: AtomicBool = AtomicBool::new(false);
        let result = app
            .on_window(move |w: &WindowHandle| {
                if !host {
                    attach(w, &attach_handle, wants, None);
                }
            })
            .on_ready(|_: &WindowHandle| log("ready", json!({})))
            .on_frame_drawn(|_| {
                if !DRAWN.swap(true, Ordering::Relaxed) {
                    log("frame", json!({}));
                }
            })
            .try_run();
        // winit's display is closed by now.
        drop(LATE.lock().unwrap_or_else(|e| e.into_inner()).take());
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

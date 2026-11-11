//! Drag and drop between a Rune window and other apps, and between two windows of one app:
//! a demo of the `dnd` feature, and the app strata-platform's session tests drive
//! (`tests/session/p1.rs`). Commands arrive as JSON lines on stdin; what happens goes to
//! stdout as JSON lines (`{"ev": …, "window": …}`).
//!
//! Each window shows a drag tile (`tile`, a [`DragSource`]), a second tile (`manual`) whose
//! own `on_drag` starts the drag through [`Dnd::start_drag`] and logs every phase with
//! whether [`Dnd::owns`] claimed it, a drop zone (`zone`), and a plain element (`plain`)
//! that logs its hover.
//!
//! Commands (`"window"` picks one; without it, both):
//! - `{"cmd":"offer","offers":[Offer],"actions":["copy","move"],"local":"text"|null,"icon":"image"|"snapshot"|null}`:
//!   what the tiles drag (lazy writers log `writer`); `local` adds an in-process payload;
//! - `{"cmd":"zone","mimes":[…],"read_all":bool,"local":bool,"action":"copy"|"move"|"ask"|"refuse"|null,"ask_answer":"move"|null,"peek":[…]}`;
//! - `{"cmd":"cancel"}`, `{"cmd":"quit"}`.
//!
//! Flags: `--title T`, `--two-windows` (a second window, `second`, titled `T second`), and the
//! test switches `--as-if-libwayland-1-22` and `--as-if-no-data-control` (P0's).

#[cfg(feature = "dnd")]
fn main() {
    demo::main();
}

#[cfg(not(feature = "dnd"))]
fn main() {
    eprintln!("run with --features dnd");
}

#[cfg(feature = "dnd")]
mod demo {
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use rune::{
        App, AppHandle, AppSpec, Cx, Drag, DragOutcome, El, KeyEvent, Tx, WindowHandle, WindowSpec,
        col, el, row, text,
    };
    use serde_json::{Value, json};
    use strata_platform::dnd::{
        Actions, AskReply, Dnd, DragAction, DragIcon, DragImage, DragItem, DragSource, DropData,
        DropTarget, Hover, TestRoute,
    };
    use strata_test_peer::protocol::{Offer, fnv1a64};

    fn log(window: &str, event: &str, fields: Value) {
        let mut line = json!({ "ev": event, "window": window });
        if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
            obj.extend(fields);
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    fn action_name(a: DragAction) -> &'static str {
        match a {
            DragAction::Copy => "copy",
            DragAction::Move => "move",
            DragAction::Link => "link",
            DragAction::Ask => "ask",
        }
    }

    fn action(name: &str) -> Option<DragAction> {
        match name {
            "copy" => Some(DragAction::Copy),
            "move" => Some(DragAction::Move),
            "link" => Some(DragAction::Link),
            "ask" => Some(DragAction::Ask),
            _ => None,
        }
    }

    fn outcome_name(o: DragOutcome) -> &'static str {
        match o {
            DragOutcome::Copied => "copied",
            DragOutcome::Moved => "moved",
            DragOutcome::Linked => "linked",
            DragOutcome::Cancelled => "cancelled",
            _ => "other",
        }
    }

    /// What the tiles drag.
    #[derive(Clone, Default)]
    struct OfferSpec {
        offers: Vec<Offer>,
        actions: Vec<DragAction>,
        local: Option<String>,
        icon: Option<String>,
    }

    /// How the zone takes drops.
    #[derive(Clone)]
    struct ZoneSpec {
        mimes: Vec<String>,
        read_all: bool,
        local: bool,
        /// None: P1's rule; Some(None): refuse.
        action: Option<Option<DragAction>>,
        ask_answer: Option<DragAction>,
        peek: Vec<String>,
    }

    impl Default for ZoneSpec {
        fn default() -> Self {
            ZoneSpec {
                mimes: vec!["text/uri-list".into()],
                read_all: true,
                local: true,
                action: None,
                ask_answer: None,
                peek: Vec::new(),
            }
        }
    }

    struct State {
        window: String,
        dnd: Dnd<State>,
        offer: OfferSpec,
        zone: ZoneSpec,
        hover: Hover,
    }

    /// The in-process payload the tiles carry: the test checks it arrives typed.
    #[derive(Clone, Debug, PartialEq)]
    struct Payload(String);

    #[derive(Clone)]
    struct Snap(String);

    fn snap_view(s: &Snap, _: &mut Cx<Snap>) -> El {
        col()
            .w_full()
            .h_full()
            .fill(rune::oklch(0.55, 0.15, 260.0))
            .center()
            .child(text(s.0.clone()))
    }

    fn item(s: &State) -> Option<DragItem> {
        let spec = &s.offer;
        let mut item = DragItem::new().actions(spec.actions.iter().copied().collect::<Actions>());
        for o in &spec.offers {
            let (mime, data, window) = (o.mime.clone(), o.data.clone(), s.window.clone());
            item = item.lazy(o.mime.clone(), move || {
                log(&window, "writer", json!({ "mime": mime }));
                data.bytes().unwrap_or_default()
            });
        }
        if let Some(p) = &spec.local {
            item = item.local(Payload(p.clone()));
        }
        match spec.icon.as_deref() {
            Some("image") => {
                let px: Vec<u8> = (0..32 * 24).flat_map(|_| [40, 120, 220, 200]).collect();
                if let Some(img) = DragImage::new(px, 32, 24) {
                    item = item.icon(DragIcon::image(img));
                }
            }
            Some("snapshot") => {
                item = item.icon(DragIcon::snapshot(
                    Snap("drag".into()),
                    snap_view,
                    (48.0, 32.0),
                    1.0,
                    true,
                ));
            }
            _ => {}
        }
        Some(item)
    }

    fn ended(s: &mut State, outcome: DragOutcome, _: &mut Tx) {
        log(
            &s.window,
            "drag-end",
            json!({ "outcome": outcome_name(outcome) }),
        );
    }

    fn dropped(s: &mut State, data: DropData, _: &mut Tx) {
        let items: Vec<Value> = data
            .items
            .iter()
            .map(|(m, b)| json!({ "mime": m, "len": b.len(), "fnv1a64": fnv1a64(b) }))
            .collect();
        log(
            &s.window,
            "drop",
            json!({
                "action": action_name(data.action),
                "items": items,
                "failed": data.failed,
                "local": data.local::<Payload>().map(|p| p.0.clone()),
                "mods": data.offer.mods.map(|m| json!({ "ctrl": m.ctrl, "shift": m.shift })),
                "point": data.point.map(|p| [p.x, p.y, p.window_x, p.window_y]),
                "mimes": data.offer.mimes,
            }),
        );
    }

    fn zone(s: &State) -> DropTarget<State> {
        let z = &s.zone;
        let mut t = DropTarget::new(dropped)
            .mime_types(&z.mimes)
            .peek(&z.peek)
            .on_hover(|s: &mut State, hover, offer, _| {
                s.hover = hover;
                let state = match hover {
                    Hover::Left => "left".to_owned(),
                    Hover::Accepting(a) => format!("accepting {}", action_name(a)),
                    Hover::Rejecting => "rejecting".to_owned(),
                };
                let peeked: Vec<Value> = offer
                    .peeked
                    .iter()
                    .map(|(m, b)| json!({ "mime": m, "len": b.len(), "fnv1a64": fnv1a64(b) }))
                    .collect();
                log(
                    &s.window,
                    "hover",
                    json!({
                        "state": state,
                        "mimes": offer.mimes,
                        "actions": offer.actions.iter().map(action_name).collect::<Vec<_>>(),
                        "local": offer.local,
                        "mods": offer.mods.map(|m| json!({ "ctrl": m.ctrl, "shift": m.shift })),
                        "serial": offer.serial,
                        "peeked": peeked,
                    }),
                );
            })
            .on_move(|s: &mut State, p, _| {
                log(
                    &s.window,
                    "move",
                    json!({ "x": p.x, "y": p.y, "wx": p.window_x, "wy": p.window_y }),
                );
            })
            .on_reject(|s: &mut State, _, _| log(&s.window, "reject", json!({})))
            .on_ask(|s: &mut State, data: DropData, reply: AskReply, tx| {
                log(&s.window, "ask", json!({ "mimes": data.offer.mimes }));
                let answer = s.zone.ask_answer;
                match answer {
                    Some(a) => {
                        let mut data = data;
                        data.action = a;
                        dropped(s, data, tx);
                        reply.choose(a);
                    }
                    None => reply.refuse(),
                }
            });
        if z.read_all {
            t = t.read_all();
        }
        if z.local {
            t = t.local::<Payload>();
        }
        if let Some(pick) = z.action {
            t = t.action(move |_, _, _| pick);
        }
        t
    }

    fn manual_drag(s: &mut State, d: &mut Drag, tx: &mut Tx) {
        let owned = s.dnd.owns(d);
        log(
            &s.window,
            "manual-phase",
            json!({ "phase": format!("{:?}", d.phase), "cancelled": d.cancelled(), "owned": owned }),
        );
        if owned || d.phase != rune::Phase::Move {
            return;
        }
        let travelled = d.dx.hypot(d.dy);
        if travelled > 8.0
            && let Some(item) = item(s)
        {
            let dnd = s.dnd.clone();
            match dnd.start_drag(tx, d, "manual", item, ended) {
                Ok(()) => log(&s.window, "drag-start", json!({ "tile": "manual" })),
                Err(e) => log(&s.window, "drag-error", json!({ "error": e.to_string() })),
            }
        }
    }

    fn view(s: &State, _: &mut Cx<State>) -> El {
        let tile = s.dnd.drag_source(
            el().size(120.0, 80.0)
                .fill(rune::oklch(0.7, 0.15, 50.0))
                .child(text("tile"))
                .on_layout(|s: &mut State, r: [f32; 4]| {
                    log(&s.window, "rect", json!({ "key": "tile", "r": r }))
                }),
            "tile",
            DragSource::new(|s: &State, _| item(s))
                .on_start(|s: &mut State, _| {
                    log(&s.window, "drag-start", json!({ "tile": "tile" }))
                })
                .on_end(ended)
                .on_drag(|s: &mut State, d: &mut Drag, _| {
                    log(
                        &s.window,
                        "tile-phase",
                        json!({ "phase": format!("{:?}", d.phase), "cancelled": d.cancelled() }),
                    );
                }),
        );
        let manual = el()
            .key("manual")
            .size(120.0, 80.0)
            .fill(rune::oklch(0.7, 0.15, 140.0))
            .child(text("manual"))
            .on_drag(manual_drag)
            .on_layout(|s: &mut State, r: [f32; 4]| {
                log(&s.window, "rect", json!({ "key": "manual", "r": r }))
            });
        let plain = el()
            .key("plain")
            .size(120.0, 80.0)
            .fill(rune::oklch(0.6, 0.0, 0.0))
            .child(text("plain"))
            .on_hover_change(|s: &mut State, over: bool, _: &mut Tx| {
                log(&s.window, "plain-hover", json!({ "over": over }))
            })
            .on_layout(|s: &mut State, r: [f32; 4]| {
                log(&s.window, "rect", json!({ "key": "plain", "r": r }))
            });
        let zone_el = s.dnd.drop_target(
            el().w_full()
                .h_full()
                .fill(if s.hover == Hover::Left {
                    rune::oklch(0.3, 0.02, 260.0)
                } else {
                    rune::oklch(0.5, 0.12, 150.0)
                })
                .child(text("zone"))
                .on_layout(|s: &mut State, r: [f32; 4]| {
                    log(&s.window, "rect", json!({ "key": "zone", "r": r }))
                }),
            "zone",
            zone(s),
        );
        col()
            .w_full()
            .h_full()
            .child(row().gap(16.0).child(tile).child(manual).child(plain))
            .child(zone_el)
            .on_layout(|s: &mut State, r: [f32; 4]| {
                log(&s.window, "layout", json!({ "w": r[2], "h": r[3] }))
            })
            .on_window_focus(|s: &mut State, focused: bool, _: &mut Tx| {
                log(&s.window, "focus", json!({ "focused": focused }));
            })
    }

    fn on_key(s: &mut State, ev: &KeyEvent, _: &mut Tx) -> bool {
        log(&s.window, "key", json!({ "key": format!("{:?}", ev.key) }));
        false
    }

    /// Each window's key and handle, for stdin's commands.
    type Handles = Arc<Mutex<Vec<(String, AppHandle<State>)>>>;

    static SHUT_DOWN: AtomicBool = AtomicBool::new(false);

    fn shutdown(window: &str, why: &str) {
        if SHUT_DOWN.swap(true, Ordering::SeqCst) {
            return;
        }
        let r = strata_platform::dnd::shutdown();
        log(
            window,
            "shutdown",
            json!({ "why": why, "ok": r.is_ok(), "error": r.err().map(|e| e.to_string()) }),
        );
    }

    fn command(s: &mut State, cmd: &Value, tx: &mut Tx) {
        let field = |k: &str| cmd.get(k).cloned().unwrap_or(Value::Null);
        let actions = |v: Value| -> Vec<DragAction> {
            serde_json::from_value::<Vec<String>>(v)
                .unwrap_or_default()
                .iter()
                .filter_map(|a| action(a))
                .collect()
        };
        match cmd["cmd"].as_str().unwrap_or_default() {
            "offer" => {
                s.offer = OfferSpec {
                    offers: serde_json::from_value(field("offers")).unwrap_or_default(),
                    actions: actions(field("actions")),
                    local: field("local").as_str().map(str::to_owned),
                    icon: field("icon").as_str().map(str::to_owned),
                };
                if s.offer.actions.is_empty() {
                    s.offer.actions = vec![DragAction::Copy];
                }
                log(&s.window, "offered", json!({}));
            }
            "zone" => {
                let z = ZoneSpec::default();
                s.zone = ZoneSpec {
                    mimes: serde_json::from_value(field("mimes")).unwrap_or(z.mimes),
                    read_all: field("read_all").as_bool().unwrap_or(true),
                    local: field("local").as_bool().unwrap_or(true),
                    action: match field("action").as_str() {
                        None => None,
                        Some("refuse") => Some(None),
                        Some(a) => Some(action(a)),
                    },
                    ask_answer: field("ask_answer").as_str().and_then(action),
                    peek: serde_json::from_value(field("peek")).unwrap_or_default(),
                };
                log(&s.window, "zoned", json!({}));
            }
            "cancel" => {
                s.dnd.cancel_drag();
                log(&s.window, "cancel-sent", json!({}));
            }
            "quit" => {
                shutdown(&s.window, "quit");
                tx.quit();
            }
            other => log(&s.window, "bad-command", json!({ "cmd": other })),
        }
    }

    fn on_close(s: &mut State, _: &mut Tx) {
        shutdown(&s.window, "close");
    }

    fn window_app(
        title: &'static str,
        key: &str,
        handles: &Handles,
        route: TestRoute,
    ) -> App<State> {
        let dnd = Dnd::<State>::new().test_route(route);
        let state_dnd = dnd.clone();
        let window = key.to_owned();
        let mut spec = AppSpec::new(title, (720.0, 480.0), || unreachable!("init is set"), view);
        spec.on_key = Some(on_key);
        let app = App::new(spec)
            .init(move || State {
                window,
                dnd: state_dnd,
                offer: OfferSpec::default(),
                zone: ZoneSpec::default(),
                hover: Hover::Left,
            })
            .on_close(on_close);
        let handle = app.handle();
        handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((key.to_owned(), handle.clone()));
        let name = key.to_owned();
        diagnostics(&name);
        app.on_window(move |w: &WindowHandle| match dnd.attach(w, &handle) {
            Ok(st) => log(
                &name,
                "attached",
                json!({
                    "key": w.key(),
                    "backend": st.backend,
                    "drag_in": st.drag_in,
                    "drag_out": st.drag_out,
                    "unsupported": st.unsupported,
                }),
            ),
            Err(e) => log(&name, "attach-error", json!({ "error": e.to_string() })),
        })
    }

    fn diagnostics(window: &str) {
        let w = window.to_owned();
        strata_platform::dnd::on_diagnostic(move |m| {
            log(&w, "diagnostic", json!({ "message": m }))
        });
    }

    pub(super) fn main() {
        let args = rune::tooling::extra_args();
        let flag = |f: &str| args.iter().any(|a| a == f);
        let title: &'static str = args
            .iter()
            .position(|a| a == "--title")
            .and_then(|i| args.get(i + 1))
            .map_or("dnd", |t| Box::leak(t.clone().into_boxed_str()));
        rune::tooling::set_app_id("strata-dnd-demo");
        let handles: Handles = Arc::default();
        let route = TestRoute {
            as_if_libwayland_below_1_23: flag("--as-if-libwayland-1-22"),
            as_if_no_data_control: flag("--as-if-no-data-control"),
        };
        let mut app = window_app(title, "main", &handles, route);
        if flag("--two-windows") {
            let second: &'static str = Box::leak(format!("{title} second").into_boxed_str());
            let spec =
                WindowSpec::from_app("second", window_app(second, "second", &handles, route));
            let _ = app.window(spec);
        }
        let stdin_handles = handles.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let Ok(cmd) = serde_json::from_str::<Value>(&line) else {
                    log("", "bad-command", json!({ "line": line }));
                    continue;
                };
                let only = cmd["window"].as_str().map(str::to_owned);
                let all = stdin_handles
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                for (key, h) in all {
                    if only.as_ref().is_some_and(|o| *o != key) {
                        continue;
                    }
                    let cmd = cmd.clone();
                    h.update(move |s, tx| command(s, &cmd, tx));
                }
            }
        });
        static DRAWN: AtomicBool = AtomicBool::new(false);
        let result = app
            .on_ready(|w: &WindowHandle| log("main", "ready", json!({ "key": w.key() })))
            .on_frame_drawn(|_| {
                if !DRAWN.swap(true, Ordering::Relaxed) {
                    log("main", "frame", json!({}));
                }
            })
            .try_run();
        log(
            "main",
            "exit",
            json!({ "shut_down": SHUT_DOWN.load(Ordering::SeqCst) }),
        );
        if let Err(error) = result {
            log("main", "exit-error", json!({ "error": error }));
            std::process::exit(1);
        }
    }
}

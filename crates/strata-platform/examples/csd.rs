//! Client-side decorations (`csd`): a borderless Rune window, without rune-kit, that moves from
//! its header, resizes from its edges where the platform allows, and has its own Minimise,
//! Maximise and Close buttons.
//!
//! `cargo run -p strata-platform --features csd --example csd` opens it.
//!
//! - `--frame auto|borderless|native` picks the [`Policy`] (default `auto`: borderless).
//! - `--title TITLE` sets the window title (default `csd`). Its app id is
//!   `strata-csd-example`.
//!
//! It reports what happens as JSON lines on stdout, for the session tests and hand tests:
//!
//! - `{"ev":"ready","handle":"wayland"}` (or `xlib`, `xcb`), then `{"ev":"frame"}` after its
//!   first frame;
//! - `{"ev":"csd","notice":"attached",…}` once the window is linked, and `"notice":"report"`
//!   each time the X11 watcher sees the window's state or capabilities change. Each line
//!   carries `backend`, `frame`, `bands` (whether the resize bands are mounted), `window_menu`,
//!   `state` and `capabilities`;
//! - `{"ev":"csd-error","error":"…"}` if its X11 connection fails;
//! - `{"ev":"outcome","request":"Minimize","result":"sent","route":"X11"}`, or `"unsupported"`
//!   with its `gap` and `draft`, or `"failed"` with its `error`, for each request;
//! - `{"ev":"layout","w":…,"h":…}` each time its size in points changes, and
//!   `{"ev":"rect","key":"minimize","x":…,"y":…,"w":…,"h":…}` for each button (`minimize`,
//!   `maximize`, `close`) each time it moves;
//! - `{"ev":"exit-error","error":"…"}` if the app can't run.

#[cfg(feature = "csd")]
mod demo {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};

    use rune::raw_window_handle::RawWindowHandle;
    use rune::{App, AppSpec, Cx, El, Tx, WindowHandle, WindowOptions, col, el, row, text};
    use serde_json::{Value, json};
    use strata_platform::csd::{self, Backend, Csd, Notice, Outcome, Policy};

    pub struct State {
        csd: Csd,
    }

    fn access(s: &mut State) -> &mut Csd {
        &mut s.csd
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

    fn arg(name: &str) -> Option<&'static str> {
        let args = rune::tooling::extra_args();
        let at = args.iter().position(|a| a == name)?;
        args.get(at + 1).map(String::as_str)
    }

    fn policy() -> Policy {
        match arg("--frame") {
            Some("borderless") => Policy::Borderless,
            Some("native") => Policy::Native,
            _ => Policy::Auto,
        }
    }

    fn observe(csd: &Csd, notice: Notice<'_>) {
        match notice {
            Notice::Attached | Notice::Report => {
                let (state, caps) = (csd.state(), csd.capabilities());
                log(
                    "csd",
                    json!({
                        "notice": if matches!(notice, Notice::Attached) { "attached" } else { "report" },
                        "backend": format!("{:?}", csd.backend()).to_lowercase(),
                        "frame": format!("{:?}", csd.frame()).to_lowercase(),
                        "bands": csd.shows_bands(),
                        "window_menu": csd.shows_window_menu(),
                        "state": { "maximized": state.maximized, "fullscreen": state.fullscreen },
                        "capabilities": {
                            "resize": caps.resize,
                            "minimize": caps.minimize,
                            "maximize": caps.maximize,
                            "window_menu": caps.window_menu,
                        },
                    }),
                );
            }
            Notice::Error(error) => log("csd-error", json!({ "error": error })),
            Notice::Outcome(outcome) => {
                let request = format!("{:?}", outcome.request());
                let fields = match outcome {
                    Outcome::Sent { route, .. } => {
                        json!({ "request": request, "result": "sent", "route": format!("{route:?}") })
                    }
                    Outcome::Unsupported { gap, .. } => json!({
                        "request": request,
                        "result": "unsupported",
                        "gap": format!("{gap:?}"),
                        "draft": gap.rune_draft(),
                    }),
                    Outcome::Failed { error, .. } => {
                        json!({ "request": request, "result": "failed", "error": error })
                    }
                };
                log("outcome", fields);
            }
        }
    }

    fn init() -> State {
        State {
            csd: Csd::new(policy().frame(), Backend::from_env()).observe(observe),
        }
    }

    fn view(s: &State, cx: &mut Cx<State>) -> El {
        let t = rune::tok::theme(cx.system_dark());
        let button = |key: &'static str, label: &'static str| {
            row()
                .key(key)
                .h(28.0)
                .px(10.0)
                .radius(rune::tok::radius::SM)
                .center()
                .fill(t.control_track)
                .role("button")
                .label(label)
                .child(text(label).color(t.text))
                .on_layout(move |_: &mut State, r: [f32; 4]| {
                    log(
                        "rect",
                        json!({ "key": key, "x": r[0], "y": r[1], "w": r[2], "h": r[3] }),
                    );
                })
        };
        let maximized = s.csd.state().maximized == Some(true);
        let max_label = if maximized { "Restore" } else { "Maximize" };
        let bar = row()
            .key("header")
            .w_full()
            .h(40.0)
            .px(12.0)
            .gap(8.0)
            .center_y()
            .fill(t.card)
            .child(text("csd").color(t.text))
            .child(el().grow(1.0))
            .child(
                button("minimize", "Minimize").on_press_tx(|s: &mut State, tx: &mut Tx| {
                    s.csd.minimize(tx);
                }),
            )
            .child(
                button("maximize", max_label).on_press_tx(|s: &mut State, tx: &mut Tx| {
                    s.csd.toggle_maximized(tx);
                }),
            )
            .child(
                button("close", "Close").on_press_tx(|s: &mut State, tx: &mut Tx| {
                    s.csd.close(tx);
                }),
            );
        let bar = csd::header(bar, &s.csd, access);
        let body = col()
            .key("body")
            .grow(1.0)
            .padding(16.0)
            .gap(8.0)
            .child(
                text(format!(
                    "{:?} frame on {:?}",
                    s.csd.frame(),
                    s.csd.backend()
                ))
                .color(t.text),
            )
            .child(text(format!("{:?}", s.csd.state())).color(t.text_secondary))
            .child(text(format!("{:?}", s.csd.capabilities())).color(t.text_secondary));
        let content = col()
            .key("content")
            .w_full()
            .h_full()
            .fill(t.bg)
            .on_layout(|_: &mut State, r: [f32; 4]| log("layout", json!({ "w": r[2], "h": r[3] })))
            .child(bar)
            .child(body);
        csd::decorate(content, &s.csd, access)
    }

    fn handle_kind(w: &WindowHandle) -> String {
        w.with_raw_handle(|h| match h.as_raw() {
            RawWindowHandle::Wayland(_) => "wayland".to_owned(),
            RawWindowHandle::Xlib(_) => "xlib".to_owned(),
            RawWindowHandle::Xcb(_) => "xcb".to_owned(),
            other => format!("{other:?}"),
        })
        .unwrap_or_else(|e| format!("error: {e}"))
    }

    fn xid(w: &WindowHandle) -> Option<u64> {
        w.with_raw_handle(|h| match h.as_raw() {
            RawWindowHandle::Xlib(x) => Some(x.window),
            RawWindowHandle::Xcb(x) => Some(u64::from(x.window.get())),
            _ => None,
        })
        .ok()
        .flatten()
    }

    pub fn main() {
        let title = arg("--title").unwrap_or("csd");
        rune::tooling::set_app_id("strata-csd-example");
        let options = csd::window_options(policy().frame(), WindowOptions::default());
        static DRAWN: AtomicBool = AtomicBool::new(false);
        let app = App::new(AppSpec::new(title, (640.0, 400.0), init, view)).window_options(options);
        let handle = app.handle();
        let result = app
            .on_window(move |w: &WindowHandle| csd::attach(w, &handle, access))
            .on_ready(|w: &WindowHandle| {
                log("ready", json!({ "handle": handle_kind(w), "xid": xid(w) }));
            })
            .on_frame_drawn(|_| {
                if !DRAWN.swap(true, Ordering::Relaxed) {
                    log("frame", json!({}));
                }
            })
            .try_run();
        if let Err(error) = result {
            log("exit-error", json!({ "error": error }));
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "csd")]
fn main() {
    demo::main();
}

#[cfg(not(feature = "csd"))]
fn main() {
    eprintln!("run with --features csd");
}

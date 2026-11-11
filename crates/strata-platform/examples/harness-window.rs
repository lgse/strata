//! A plain Rune window for the compositor test harness (W3): what strata-platform's session
//! tests open under headless sway and Xwayland. It uses no shim. The whole window is one
//! button, and it reports what it sees as JSON lines on stdout:
//!
//! - `{"ev":"ready","handle":"wayland"}` (or `xlib`, `xcb`) once the window exists;
//! - `{"ev":"frame"}` after its first frame is drawn;
//! - `{"ev":"layout","w":640.0,"h":400.0}` each time its size in points changes;
//! - `{"ev":"focus","focused":true}`, `{"ev":"press","n":1}`, `{"ev":"secondary-press"}` and
//!   `{"ev":"key","key":"Enter",…}`;
//! - `{"ev":"exit-error","error":"…"}` if the app can't run.
//!
//! `--title TITLE` sets the window title (default `harness-window`). Its app id is
//! `strata-harness-window`.
//!
//! `cargo run -p strata-platform --example harness-window` opens it on the desktop. The tests
//! run it in a private session only (`crates/strata-platform/tests/support/`).

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::raw_window_handle::RawWindowHandle;
use rune::{App, AppSpec, Cx, El, KeyEvent, PointerPress, Tx, WindowHandle, col, text};
use serde_json::{Value, json};

#[derive(Default)]
struct State {
    presses: u32,
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
        .on_secondary_press(|_: &mut State, _: PointerPress, _: &mut Tx| {
            log("secondary-press", json!({}));
        })
        .on_layout(|_: &mut State, r: [f32; 4]| {
            log("layout", json!({ "w": r[2], "h": r[3] }));
        })
        .on_window_focus(|_: &mut State, focused: bool, _: &mut Tx| {
            log("focus", json!({ "focused": focused }));
        })
}

fn on_key(_: &mut State, k: &KeyEvent, _: &mut Tx) -> bool {
    log(
        "key",
        json!({
            "key": format!("{:?}", k.key),
            "ctrl": k.mods.ctrl,
            "shift": k.mods.shift,
            "alt": k.mods.alt,
        }),
    );
    true
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

fn main() {
    let args = rune::tooling::extra_args();
    let title = args
        .iter()
        .position(|a| a == "--title")
        .and_then(|i| args.get(i + 1))
        .map_or("harness-window", |t| t.as_str());
    rune::tooling::set_app_id("strata-harness-window");
    let spec = AppSpec {
        on_key: Some(on_key),
        ..AppSpec::new(title, (640.0, 400.0), State::default, view)
    };
    static DRAWN: AtomicBool = AtomicBool::new(false);
    let result = App::new(spec)
        .on_ready(|w: &WindowHandle| log("ready", json!({ "handle": handle_kind(w) })))
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

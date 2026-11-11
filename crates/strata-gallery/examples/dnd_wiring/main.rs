//! The drag-and-drop wiring demo (X2): Strata's views and chrome on a real folder, with every
//! drop target and drag source joined to the system's drag and drop (P1) through
//! `adapter.rs`, which an app copies as it is.
//!
//! ```text
//! cargo run -p strata-gallery --example dnd_wiring -- --root DIR [--title T] [--two-windows] [--apply]
//! ```
//!
//! - **Drag out** from Columns, List and Icons (tabs 1 to 3): `text/uri-list`, GNOME's and
//!   KDE's file formats, and the typed payload for the demo's own windows.
//! - **Drop in** on folder rows and pane backgrounds in each view, crumbs, tabs, New tab and
//!   the strip (folders from this process open tabs), sidebar rows, the pin gaps and the USB
//!   device (another volume: a plain drop asks). The action follows Ctrl and Shift where the
//!   platform says, and is shown before the drop.
//! - Folder rows and sidebar rows spring open after 750 ms, crumbs after 750 ms and tabs after
//!   450 ms (their components'); Columns autoscrolls near its edges.
//! - Every transfer is logged. `--apply` does it, inside `--root` only.
//! - `--two-windows` opens a second window, `second`: drags between them take the fast path.
//!
//! Commands arrive as JSON lines on stdin (`"window"` picks one, else all):
//! `{"cmd":"locate","what":[{"name":…,"row":…,"depth":…}|{"crumb":i}|{"tab":id}|{"new-tab":true}|{"gap":i}|{"place":rel}|{"trash":true}|{"device":true}|{"pane":true}|{"column":d}]}`
//! (logs `rect` lines), `{"cmd":"tab","id":n}`, `{"cmd":"open","tab":n,"path":rel}`, `{"cmd":"group","tab":n}`, `{"cmd":"scroll","tab":n,"y":y}`, `{"cmd":"select","tab":n,"depth":d,"names":[…]}`,
//! `{"cmd":"state"}`, `{"cmd":"peek","on":bool}`, `{"cmd":"cancel"}`, `{"cmd":"quit"}`. What
//! happens goes to stdout as JSON lines (`{"ev":…,"window":…}`).

mod adapter;
mod fsys;
mod host;

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rune::{App, AppHandle, AppSpec, Tx, WindowHandle, WindowSpec};
use serde_json::{Value, json};
use strata_platform::dnd::Dnd;

use adapter::Wiring;
use host::Win;

/// One JSON line on stdout.
pub fn log(window: &str, event: &str, fields: Value) {
    let mut line = json!({ "ev": event, "window": window });
    if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
        obj.extend(fields);
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

type Handles = Arc<Mutex<Vec<(String, AppHandle<Win>)>>>;

/// Windows open: P1 shuts down in the last one's `on_close` (and in `on_quit`).
static OPEN: AtomicUsize = AtomicUsize::new(0);
static SHUT_DOWN: AtomicBool = AtomicBool::new(false);

fn shutdown(window: &str, why: &str) {
    if SHUT_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let r = strata_platform::dnd::shutdown();
    log(window, "shutdown", json!({ "why": why, "ok": r.is_ok() }));
}

fn on_close(w: &mut Win, _: &mut Tx) {
    if OPEN.fetch_sub(1, Ordering::SeqCst) == 1 {
        shutdown(&w.name, "last window closed");
    }
}

fn on_quit(w: &mut Win, _: &mut Tx) {
    shutdown(&w.name, "quit");
}

fn command(w: &mut Win, cmd: &Value, tx: &mut Tx) {
    if cmd["cmd"] == "quit" {
        shutdown(&w.name, "quit");
        tx.quit();
        return;
    }
    w.command(cmd, tx);
}

fn window_app(
    title: &'static str,
    key: &str,
    root: PathBuf,
    apply: bool,
    handles: &Handles,
) -> App<Win> {
    let dnd = Dnd::<Win>::new();
    let wiring = Wiring::new(dnd.clone());
    let name = key.to_owned();
    let spec = AppSpec::new(
        title,
        (1100.0, 700.0),
        || unreachable!("init is set"),
        host::view,
    );
    let mut spec = spec;
    spec.poll = Some(host::poll);
    let app = App::new(spec)
        .init(move || Win::new(name, root, apply, wiring))
        .on_close(on_close)
        .on_quit(on_quit);
    let handle = app.handle();
    handles
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((key.to_owned(), handle.clone()));
    let name = key.to_owned();
    let w = name.clone();
    strata_platform::dnd::on_diagnostic(move |m| log(&w, "diagnostic", json!({ "message": m })));
    OPEN.fetch_add(1, Ordering::SeqCst);
    app.on_window(move |win: &WindowHandle| match dnd.attach(win, &handle) {
        Ok(st) => {
            let st2 = st.clone();
            handle.update(move |w: &mut Win, _| w.status = Some(st2));
            log(
                &name,
                "attached",
                json!({
                    "backend": st.backend, "drag_in": st.drag_in, "drag_out": st.drag_out,
                    "unsupported": st.unsupported,
                }),
            );
        }
        Err(e) => log(&name, "attach-error", json!({ "error": e.to_string() })),
    })
}

fn main() {
    let args = rune::tooling::extra_args();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let Some(root) = value("--root").map(PathBuf::from) else {
        eprintln!("dnd_wiring: --root DIR is required");
        std::process::exit(2);
    };
    let Ok(root) = root.canonicalize() else {
        eprintln!("dnd_wiring: no folder {}", root.display());
        std::process::exit(2);
    };
    let title: &'static str = Box::leak(
        value("--title")
            .unwrap_or_else(|| "dnd wiring".into())
            .into_boxed_str(),
    );
    let apply = flag("--apply");
    rune::tooling::set_app_id("strata-dnd-wiring");
    let handles: Handles = Arc::default();
    let mut app = window_app(title, "main", root.clone(), apply, &handles);
    if flag("--two-windows") {
        let second: &'static str = Box::leak(format!("{title} second").into_boxed_str());
        let spec = WindowSpec::from_app(
            "second",
            window_app(second, "second", root, apply, &handles),
        );
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
                h.update(move |w, tx| command(w, &cmd, tx));
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

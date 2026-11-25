// SPDX-License-Identifier: MIT

mod adapter;
mod fsys;
mod host;

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::{App, AppHandle, AppSpec, Tx, WindowHandle};
use serde_json::{Value, json};
use strata_platform::clipboard::{self, TestRoute};

use adapter::FileClipboard;
use host::Win;

/// One JSON line on stdout (in the unit tests, kept for [`take_logged`] instead).
pub fn log(event: &str, fields: Value) {
    let mut line = json!({ "ev": event });
    if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
        obj.extend(fields);
    }
    #[cfg(test)]
    LOGGED.with_borrow_mut(|l| l.push(line));
    #[cfg(not(test))]
    {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

#[cfg(test)]
thread_local! {
    /// Every line logged on this thread, for the unit tests.
    static LOGGED: std::cell::RefCell<Vec<Value>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The lines logged on this thread since the last call.
#[cfg(test)]
pub fn take_logged() -> Vec<Value> {
    LOGGED.with_borrow_mut(std::mem::take)
}

static SHUT_DOWN: AtomicBool = AtomicBool::new(false);

fn shutdown(why: &str) {
    if SHUT_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let r = clipboard::shutdown();
    log(
        "shutdown",
        json!({ "why": why, "ok": r.is_ok(), "hand_off": r.ok().and_then(|c| c.hand_off).map(|h| format!("{h:?}")) }),
    );
}

fn command(w: &mut Win, cmd: &Value, tx: &mut Tx) {
    if cmd["cmd"] == "quit" {
        shutdown("quit");
        tx.quit();
        return;
    }
    w.command(cmd, tx);
}

fn parse_args() -> (PathBuf, &'static str, bool, bool) {
    let args = rune::tooling::extra_args();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let Some(root) = value("--root").map(PathBuf::from) else {
        eprintln!("clipboard_wiring: --root DIR is required");
        std::process::exit(2);
    };
    let Ok(root) = root.canonicalize() else {
        eprintln!("clipboard_wiring: no folder {}", root.display());
        std::process::exit(2);
    };
    let title: &'static str = Box::leak(
        value("--title")
            .unwrap_or_else(|| "clipboard wiring".into())
            .into_boxed_str(),
    );
    (
        root,
        title,
        flag("--apply"),
        flag("--as-if-no-data-control"),
    )
}

fn spawn_stdin(stdin_handle: AppHandle<Win>) {
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let Ok(cmd) = serde_json::from_str::<Value>(&line) else {
                log("bad-command", json!({ "line": line }));
                continue;
            };
            stdin_handle.update(move |w, tx| command(w, &cmd, tx));
        }
    });
}

fn main() {
    let (root, title, apply, as_if_no_data_control) = parse_args();
    rune::tooling::set_app_id("strata-clipboard-wiring");
    clipboard::on_diagnostic(|m| log("diagnostic", json!({ "message": m })));
    let clip = FileClipboard::new(|w: &mut Win| &mut w.clip).test_route(TestRoute {
        as_if_no_data_control,
        as_if_no_shared_display: false,
    });
    let attach = clip.handle();
    let mut spec = AppSpec::new(
        title,
        (900.0, 600.0),
        || unreachable!("init is set"),
        host::view,
    );
    spec.on_key = Some(host::on_key);
    spec.poll = Some(host::poll);
    let app = App::new(spec)
        .init(move || Win::new(root, apply, clip))
        .on_quit(|_, _| shutdown("quit"))
        .on_close(|_, _| shutdown("window closed"))
        .on_display_close(|| shutdown("display-close"));
    let handle: AppHandle<Win> = app.handle();
    spawn_stdin(handle.clone());
    rune::orderly_exit();
    static DRAWN: AtomicBool = AtomicBool::new(false);
    let result = app
        .on_window(move |w: &WindowHandle| match attach.attach(w, &handle) {
            Ok(st) => log(
                "attached",
                json!({ "backend": st.backend, "route": format!("{:?}", st.route), "xwayland": st.xwayland }),
            ),
            Err(e) => log("attach-error", json!({ "error": e.to_string() })),
        })
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

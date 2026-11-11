//! The clipboard wiring demo (X3): a C1e list over a real folder, whose Ctrl+C, Ctrl+X and
//! Ctrl+V go through strata-platform's clipboard (P2) in the file managers' formats, with
//! C19's Files on clipboard badge following the clipboard. `adapter.rs` is the glue an app
//! copies as it is.
//!
//! ```text
//! cargo run -p strata-gallery --example clipboard_wiring -- --root DIR [--apply] [--title T]
//! ```
//!
//! - **Ctrl+C, Ctrl+X** on the selection: `x-special/gnome-copied-files`, `text/uri-list` and
//!   `application/x-kde-cutselection`, no text, as Strata. Copied rows show the copy icon,
//!   cut rows scissors and the cut fade. Another app's copy clears every mark.
//! - **Ctrl+V** pastes into the one folder selected, else the folder shown: the files (moved
//!   for a GNOME `cut`, a KDE flag `1`, or exactly what we cut), else an image as
//!   `image.png`. A copy into its own folder takes `name (1).ext`; a cut there does nothing;
//!   a name taken is kept beside (Keep Both, logged `conflict`). A cut's moved entries leave
//!   the clipboard.
//! - **The badge** shows while the clipboard holds a non-empty file list, from any app.
//! - Enter or a double click opens a folder, a link to one shown as the folder it resolves to
//!   (one out of `--root` doesn't open); Alt+Up and Backspace go up (not above `--root`).
//! - Every paste is logged. `--apply` does it, inside `--root` only: a source or destination
//!   outside it refuses the whole paste. One that fails partway stops there (`failed` in its
//!   `paste` line) and keeps what it did: its moves still leave the cut.
//!
//! Test-only flag: `--as-if-no-data-control` makes P2 behave as on a compositor without
//! data control (mutter): the data device, whose writes take the key's serial. For the
//! session tests (`tests/x3.rs`); not for use.
//!
//! Commands arrive as JSON lines on stdin: `{"cmd":"open","path":rel}`,
//! `{"cmd":"select","names":[…]}`, `{"cmd":"state"}`, `{"cmd":"release"}` (let go of our
//! copy, Strata's unyank), `{"cmd":"quit"}`. What happens goes to stdout as JSON lines
//! (`{"ev":…}`).

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

/// P2's shutdown, once: in `on_quit` and in the last (only) window's `on_close`.
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
    let apply = flag("--apply");
    rune::tooling::set_app_id("strata-clipboard-wiring");
    clipboard::on_diagnostic(|m| log("diagnostic", json!({ "message": m })));
    let clip = FileClipboard::new(|w: &mut Win| &mut w.clip).test_route(TestRoute {
        as_if_no_data_control: flag("--as-if-no-data-control"),
        as_if_libwayland_below_1_23: false,
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
        .on_close(|_, _| shutdown("window closed"));
    let handle: AppHandle<Win> = app.handle();
    let stdin_handle = handle.clone();
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

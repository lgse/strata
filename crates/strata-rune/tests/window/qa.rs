// SPDX-License-Identifier: MIT

//! The QA mode on the session host (H4): `strata-rune --qa <directory>` with
//! `STRATA_QA_WINDOW=1` opens a real window and serves the runner's protocol from it, with
//! input through the compositor's virtual keyboard and pointer. The steps are the runner's
//! own; the replies are read as the runner reads them.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use strata_test_peer::protocol::{Offer, Selection, Via};

use crate::support::{Display, Proc, Session};

/// sway's IPC socket in the session's runtime directory, which the session host resizes and
/// finds its window through.
fn sway_socket(s: &Session) -> PathBuf {
    std::fs::read_dir(s.runtime_dir())
        .expect("the runtime directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            name.is_some_and(|name| name.starts_with("sway-ipc.") && name.ends_with(".sock"))
        })
        .expect("sway's IPC socket")
}

/// Starts the QA mode on `folder` and waits for its first reply, the ready one.
fn serve(s: &Session, folder: &Path) -> Proc {
    let mut command = s.command(env!("CARGO_BIN_EXE_strata-rune"), Display::Wayland);
    command
        .arg("--qa")
        .arg(folder)
        .env("STRATA_QA_WINDOW", "1")
        .env("SWAYSOCK", sway_socket(s));
    let app = s.spawn("strata-rune-qa", command);
    let ready = app.wait_json("the ready reply", |reply| reply.get("ok").is_some());
    assert_eq!(ready["ok"], true, "{ready}");
    assert_eq!(ready["result"], json!({ "ready": true }), "{ready}");
    app
}

/// Sends one step and returns its reply, which must pass.
fn passes(app: &Proc, step: &Value) -> Value {
    let before = app.line_count();
    app.send_line(&step.to_string());
    let (_, reply) = app.wait_json_from(before, &format!("the reply to {step}"), |reply| {
        reply.get("ok").is_some()
    });
    assert_eq!(reply["ok"], true, "{step}: {reply}");
    reply
}

/// A key and a click cross the compositor and reach the views, and Ctrl+V in the rename
/// field pastes what another process put on the system clipboard: the clipboard the
/// offscreen host has none of.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn the_qa_mode_drives_a_real_window_through_the_compositor() {
    let s = Session::start();
    let folder = s.runtime_dir().join("home").join("Projects");
    for name in ["alpha", "beta"] {
        std::fs::create_dir_all(folder.join(name)).expect("folder");
    }
    std::fs::write(folder.join("beta").join("inside.txt"), "inside").expect("file");
    std::fs::write(folder.join("notes.txt"), "notes").expect("file");
    let app = serve(&s, &folder);

    passes(&app, &json!({ "key": "Down" }));
    passes(&app, &json!({ "wait": { "focused": "beta" } }));
    passes(&app, &json!({ "wait": { "entry": "inside.txt" } }));

    passes(&app, &json!({ "click": { "entry": "notes.txt" } }));
    passes(&app, &json!({ "wait": { "focused": "notes.txt" } }));

    passes(&app, &json!({ "key": "F2" }));
    passes(
        &app,
        &json!({ "wait": { "field": { "name": "Rename", "focused": true } } }),
    );
    let source = s.peer(Display::Wayland);
    source.set_selection(
        Selection::Clipboard,
        Via::DataControl,
        &[Offer::text("text/plain;charset=utf-8", "pasted.txt")],
    );
    passes(&app, &json!({ "key": "ctrl+a" }));
    passes(&app, &json!({ "key": "ctrl+v" }));
    passes(
        &app,
        &json!({ "wait": { "field": { "text": "pasted.txt" } } }),
    );
}

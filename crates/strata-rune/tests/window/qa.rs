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

    passes(&app, &json!({ "key": "Escape" }));
    passes(&app, &json!({ "key": "ctrl+c" }));
    passes(
        &app,
        &json!({
            "clipboard": format!("copy\nfile://{}/notes.txt", folder.display()),
            "mime": "x-special/gnome-copied-files",
        }),
    );

    passes(
        &app,
        &json!({
            "clipboard_set": {
                "offers": [{
                    "mime": "x-special/gnome-copied-files",
                    "text": format!("cut\nfile://{}/beta/inside.txt", folder.display()),
                }],
            },
        }),
    );
    passes(
        &app,
        &json!({
            "expect": {
                "present": { "role": "status bar", "name": "Files on clipboard" },
            },
        }),
    );
    passes(&app, &json!({ "key": "ctrl+v" }));
    passes(&app, &json!({ "wait": { "entry": "inside.txt" } }));
    assert!(!folder.join("beta").join("inside.txt").exists());
}

/// How many of `pid`'s windows in sway's `tree` are titled `title`.
fn windows_titled(tree: &Value, pid: u32, title: &str) -> usize {
    let own = tree["pid"].as_u64() == Some(u64::from(pid)) && tree["name"] == title;
    let nested = ["nodes", "floating_nodes"]
        .iter()
        .filter_map(|key| tree[*key].as_array())
        .flatten()
        .map(|node| windows_titled(node, pid, title))
        .sum::<usize>();
    usize::from(own) + nested
}

/// Waits until the window `id` has the keyboard or asks for it: on Wayland Rune can only ask
/// (`WindowHandle::focus` with no activation token), which sway shows as urgent.
fn wait_asking_for_attention(s: &Session, id: i64) {
    let deadline = std::time::Instant::now() + crate::support::timeout();
    loop {
        let node = node_of(&s.tree(), id);
        let asks = node
            .as_ref()
            .is_some_and(|node| node["focused"] == true || node["urgent"] == true);
        if asks {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the chooser was neither focused nor urgent: {node:#?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// The node of sway's `tree` with the container id `id`.
fn node_of(tree: &Value, id: i64) -> Option<Value> {
    if tree["id"].as_i64() == Some(id) {
        return Some(tree.clone());
    }
    ["nodes", "floating_nodes"]
        .iter()
        .filter_map(|key| tree[*key].as_array())
        .flatten()
        .find_map(|node| node_of(node, id))
}

/// Opens the entry menu on `notes.txt` and chooses "Copy to…" in it.
fn ask_to_copy(app: &Proc) {
    passes(
        app,
        &json!({ "click": { "entry": "notes.txt" }, "button": 3 }),
    );
    passes(app, &json!({ "wait": { "menu_items": {} } }));
    passes(app, &json!({ "click": { "menu_item": "Copy to…" } }));
    passes(app, &json!({ "wait": { "menu_items": null } }));
}

/// The floating Copy to chooser is a window of its own, which Rune cannot make transient for
/// the browser (Rune packet entry 23). So the host does what GTK's transient chooser gets from
/// its parent: asking again brings it forward (`DestinationLease::acquire`), as far as Rune can
/// (on Wayland, an attention request), and closing the browser closes it (`bind_host_parent`),
/// and with it the process.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn the_floating_chooser_comes_forward_and_closes_with_its_browser() {
    let s = Session::start();
    let folder = s.runtime_dir().join("home").join("Projects");
    std::fs::create_dir_all(folder.join("alpha")).expect("folder");
    std::fs::write(folder.join("notes.txt"), "notes").expect("file");
    let mut app = serve(&s, &folder);
    let pid = app.pid();
    let browser = s.wait_window_where("the browser window", |w| {
        w.pid == pid && w.title == "Projects — Strata"
    });

    ask_to_copy(&app);
    let chooser = s.wait_window_where("the Copy to chooser, in front", |w| {
        w.pid == pid && w.title == "Copy to" && w.focused
    });

    s.focus(&browser);
    ask_to_copy(&app);
    wait_asking_for_attention(&s, chooser.id);
    let choosers = windows_titled(&s.tree(), pid, "Copy to");
    assert_eq!(choosers, 1, "asking again opens no second chooser");

    s.sway_command(&format!("[con_id={}] kill", browser.id));
    assert!(
        app.wait_exit().success(),
        "the chooser closes with its browser, and the process exits"
    );
}

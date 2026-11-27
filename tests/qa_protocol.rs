// SPDX-License-Identifier: MIT

#![cfg(feature = "qa")]

use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdout, Command, Stdio},
};

use serde_json::{Value, json};

/// The binary with a home of its own and no display, bus or desktop to reach.
fn binary(home: &Path) -> Command {
    let executable = std::env::var_os("CARGO_BIN_EXE_strata")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_strata").into());
    let mut command = Command::new(executable);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_RUNTIME_DIR", home.join("runtime"))
        .env("GIO_USE_VFS", "local")
        .env("GIO_USE_VOLUME_MONITOR", "unix")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/strata-test-no-bus",
        );
    command
}

fn reply(replies: &mut BufReader<ChildStdout>) -> Value {
    let mut line = String::new();
    replies.read_line(&mut line).expect("a reply");
    serde_json::from_str(&line).unwrap_or_else(|error| panic!("{line:?}: {error}"))
}

fn send(child: &mut Child, step: &Value) {
    let stdin = child.stdin.as_mut().expect("stdin");
    writeln!(stdin, "{step}").expect("the step is sent");
    stdin.flush().expect("the step is sent");
}

#[test]
fn the_binary_answers_ready_then_one_reply_for_each_step_and_ends_with_its_input() {
    let home = tempfile::tempdir().expect("a home");
    let folder = tempfile::tempdir().expect("a folder");
    for name in ["alpha.txt", "beta.txt"] {
        std::fs::write(folder.path().join(name), name).expect("a file");
    }
    let mut child = binary(home.path())
        .arg("--qa")
        .arg(folder.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the binary starts");
    let mut replies = BufReader::new(child.stdout.take().expect("stdout"));

    let ready = reply(&mut replies);
    assert_eq!(ready["ok"], true, "{ready}");
    assert_eq!(ready["result"], json!({ "ready": true }));
    assert_eq!(ready["app"]["view_mode"], "Columns");
    let keys: Vec<&String> = ready["app"]
        .as_object()
        .expect("the state")
        .keys()
        .collect();
    assert_eq!(
        keys,
        [
            "dialog",
            "directory",
            "field",
            "focused",
            "focused_pane",
            "matches",
            "menu",
            "panes",
            "preview",
            "toplevels",
            "view_mode",
            "window_focused"
        ],
        "the snapshot's twelve keys"
    );

    // A step that arrives while the host sleeps wakes it.
    std::thread::sleep(std::time::Duration::from_millis(300));
    send(
        &mut child,
        &json!({ "expect": { "entries": ["alpha.txt", "beta.txt"] } }),
    );
    let listed = reply(&mut replies);
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(listed["criticals"], json!([]));

    send(&mut child, &json!({ "select": { "entry": "beta.txt" } }));
    let selected = reply(&mut replies);
    assert_eq!(selected["ok"], true, "{selected}");
    assert_eq!(selected["app"]["focused"], "beta.txt");
    assert!(
        selected["changed"]
            .as_array()
            .is_some_and(|changed| changed.contains(&json!("focused"))),
        "{selected}"
    );

    send(&mut child, &json!({ "no_such_step": true }));
    let refused = reply(&mut replies);
    assert_eq!(refused["ok"], false, "{refused}");

    drop(child.stdin.take());
    let status = child.wait().expect("the binary ends");
    assert!(status.success(), "{status}");
}

#[test]
fn a_wrong_command_line_prints_the_usage_the_release_check_looks_for() {
    let home = tempfile::tempdir().expect("a home");
    let output = binary(home.path())
        .arg("--qa")
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(2));
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.starts_with("usage: strata --qa "), "{said}");
    assert!(output.stdout.is_empty(), "stdout is the replies' alone");
}

#[test]
fn opening_settings_updates_does_not_trip_the_ui_thread_guard() {
    let home = tempfile::tempdir().expect("a home");
    let folder = tempfile::tempdir().expect("a folder");
    let mut child = binary(home.path())
        .arg("--qa")
        .arg(folder.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the binary starts");
    let mut replies = BufReader::new(child.stdout.take().expect("stdout"));

    let ready = reply(&mut replies);
    assert_eq!(ready["ok"], true, "{ready}");

    send(&mut child, &json!({ "key": "ctrl+comma" }));
    let opened = reply(&mut replies);
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(opened["app"]["dialog"], "Settings", "{opened}");

    send(
        &mut child,
        &json!({ "wait": { "present": { "role": "button", "name": "Updates" } } }),
    );
    let shown = reply(&mut replies);
    assert_eq!(shown["ok"], true, "{shown}");

    send(
        &mut child,
        &json!({ "click": { "role": "button", "name": "Updates" } }),
    );
    let updates = reply(&mut replies);
    assert_eq!(updates["ok"], true, "{updates}");
    assert_eq!(updates["app"]["dialog"], "Settings", "{updates}");
    assert_eq!(updates["criticals"], json!([]), "{updates}");

    drop(child.stdin.take());
    let status = child.wait().expect("the binary ends");
    assert!(status.success(), "{status}");
}

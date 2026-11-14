// SPDX-License-Identifier: MIT

//! The backend in process: steps run through `strata-qa`'s engine against the QA harness's
//! canonical tree, on a host with no display and no runner.

use std::{
    future::Future,
    path::PathBuf,
    pin::pin,
    task::{Context, Poll, Waker},
};

use serde_json::{Value, json};
use strata_core::model::Location;
use strata_qa::{Backend as _, Outcome, Remembered, execute, parse_request};

use super::{SIZE, backend::Rune, input_log::InputLog};
use crate::{
    host::offscreen::Offscreen,
    startup::{self, UiThread},
    test_support::{fixture, in_own_process},
};

mod filter;
mod icon;
mod invariants;
mod ready;
mod rename;
mod replaced;
mod right_click;
mod scroll;

const INPUT_LOG: &str = "STRATA_INPUT_LOG";

/// Runs the test `name` of this module on the QA mode's own host, in a process of its own.
fn staged(name: &str, input_log: Option<PathBuf>, run: impl FnOnce(Rune)) {
    staged_after(name, input_log, || {}, run);
}

/// As [`staged`], with `prepare` run in the test's own process before the settings are read.
fn staged_after(
    name: &str,
    input_log: Option<PathBuf>,
    prepare: impl FnOnce(),
    run: impl FnOnce(Rune),
) {
    let log: Vec<_> = input_log
        .iter()
        .map(|path| (INPUT_LOG, path.as_os_str()))
        .collect();
    in_own_process(&format!("qa::tests::{name}"), &log, || {
        prepare();
        let settings = startup::load_settings();
        let ui = UiThread::claim().expect("the UI thread");
        let host = Offscreen::sized(ui, settings, SIZE).without_animations();
        let log = InputLog::from_environment().expect("the input log");
        run(Rune::new(host, log));
    });
}

/// A step waits by turning the host itself, so its future never parks.
fn now<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("a step waited for something other than the host"),
    }
}

fn run(backend: &Rune, kept: &Remembered<Rune>, step: &Value) -> Outcome {
    let command = parse_request(&step.to_string()).expect("a step the protocol knows");
    now(execute(backend, kept, command))
}

fn passes(backend: &Rune, kept: &Remembered<Rune>, step: &Value) -> Value {
    let outcome = run(backend, kept, step);
    assert!(
        outcome.error.is_none() && outcome.failures.is_empty(),
        "{step} came to {outcome:?}"
    );
    outcome.result
}

/// The fixture, open and listed.
fn launched(backend: &Rune) -> tempfile::TempDir {
    let folder = fixture();
    backend.open(folder.path());
    now(backend.ready()).expect("the first listing");
    folder
}

/// Gate G1's scenario, `arrow-keys-move-focus/columns-arrows`, step for step. The runner
/// sends `keymap` for the scenario's `layout` step.
#[test]
fn the_first_light_scenario_passes_step_for_step() {
    let scratch = tempfile::tempdir().expect("a folder");
    let log = scratch.path().join("input.log");
    staged(
        "the_first_light_scenario_passes_step_for_step",
        Some(log),
        |backend| {
            let folder = launched(&backend);
            let kept = Remembered::default();
            for step in [
                json!({ "expect": { "entries": ["archive", "documents", "pictures", "readme.md", "todo.txt"] } }),
                json!({ "select": { "entry": "readme.md" } }),
                json!({ "wait": { "focused": "readme.md" } }),
                json!({ "keymap": { "layouts": "us", "variants": "", "group": 0, "caps_lock": false } }),
                json!({ "key": "Down" }),
                json!({ "wait": { "focused": "todo.txt" } }),
                json!({ "wait": { "selected": ["todo.txt"] } }),
                json!({ "key": "Up" }),
                json!({ "wait": { "focused": "readme.md" } }),
            ] {
                passes(&backend, &kept, &step);
            }

            let state = passes(&backend, &kept, &json!({ "query": "state" }));
            let name = Location::local(folder.path()).display_name();
            assert_eq!(state["view_mode"], "Columns");
            assert_eq!(state["directory"], name.as_str());
            assert_eq!(state["focused"], "readme.md");
            assert_eq!(state["focused_pane"], name.as_str());
            assert_eq!(
                state["panes"],
                json!([{ "name": name, "selected": ["readme.md"], "focused": "readme.md" }])
            );
            for absent in ["dialog", "menu", "preview", "field"] {
                assert_eq!(state[absent], Value::Null, "{absent}");
            }

            let path = std::env::var_os(INPUT_LOG).expect("the log's path");
            let written = std::fs::read_to_string(path).expect("the input log");
            let lines: Vec<&str> = written.lines().collect();
            assert!(lines[0].starts_with("motion "), "{written}");
            assert!(
                lines.iter().any(|line| line.ends_with(" button 1 down")),
                "{written}"
            );
            let keys: Vec<&&str> = lines
                .iter()
                .filter(|line| line.starts_with("key "))
                .collect();
            assert_eq!(
                keys,
                [
                    &"key Down down",
                    &"key Down up",
                    &"key Up down",
                    &"key Up up"
                ]
            );
        },
    );
}

#[test]
fn a_wait_for_what_never_comes_fails_and_shows_the_tree() {
    staged(
        "a_wait_for_what_never_comes_fails_and_shows_the_tree",
        None,
        |backend| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            passes(
                &backend,
                &kept,
                &json!({ "select": { "entry": "readme.md" } }),
            );

            let step =
                json!({ "wait": { "focused": "todo.txt" }, "timeout_ms": 200, "message": "m" });
            let outcome = run(&backend, &kept, &step);

            assert_eq!(outcome.failures.len(), 1, "{outcome:?}");
            assert_eq!(outcome.failures[0].actual, "readme.md");
            let tree = outcome.result["tree"].as_str().expect("the tree");
            assert!(
                tree.starts_with("focus: list-box-option role=\"list item\" name=\"readme.md\"\n"),
                "{tree}"
            );
            let row = tree
                .lines()
                .find(|line| line.contains("name=\"readme.md\""));
            let row = row.and_then(|_| tree.lines().find(|line| line.contains("[focused]")));
            let row = row.expect("the focused row").trim_start();
            assert!(row.starts_with("list-box-option#row-"), "{row}");
            assert!(
                row.contains(" role=\"list item\" name=\"readme.md\" [selected] description=\"File\" [focused] ("),
                "{row}"
            );
        },
    );
}

#[test]
fn held_modifiers_reach_the_view_and_what_only_gtk_runs_says_so() {
    staged(
        "held_modifiers_reach_the_view_and_what_only_gtk_runs_says_so",
        None,
        |backend| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            passes(
                &backend,
                &kept,
                &json!({ "select": { "entry": "readme.md" } }),
            );
            // The adapter tells a Shift range from a move by the modifiers Rune sees held.
            passes(&backend, &kept, &json!({ "key": "shift+Down" }));
            passes(
                &backend,
                &kept,
                &json!({ "wait": { "selected": ["readme.md", "todo.txt"] } }),
            );
            passes(
                &backend,
                &kept,
                &json!({ "wait": { "focused": "todo.txt" } }),
            );

            let unsupported = run(&backend, &kept, &json!({ "clipboard_set": "text" }));
            assert_eq!(
                unsupported.error.as_deref(),
                Some("clipboard_set is not supported on this target")
            );
            let pixel = json!({ "expect": { "pixel": { "target": { "entry": "todo.txt" }, "rgb": [0, 0, 0] } } });
            assert_eq!(
                run(&backend, &kept, &pixel).error.as_deref(),
                Some("pixel is not supported on this target")
            );
            let unknown = json!({ "keymap": { "layouts": "no-such-layout" } });
            assert!(run(&backend, &kept, &unknown).error.is_some());
        },
    );
}

/// `selection/leftover-click-honors-modifiers`: the run aims at each entry's `part: label`,
/// two points inside its right edge, and lets the modifier go before the button.
#[test]
fn a_click_on_a_rows_name_label_lands_on_the_name_the_view_draws() {
    staged(
        "a_click_on_a_rows_name_label_lands_on_the_name_the_view_draws",
        None,
        |backend| {
            let folder = launched(&backend);
            let root = Location::local(folder.path()).display_name();
            let kept = Remembered::default();
            let click = |entry: &str, modifier: &str| {
                json!({
                    "click": { "entry": entry, "part": "label", "dir": root },
                    "mods": [modifier], "release_mods_first": true, "at": [-2, "50%"]
                })
            };
            let selected = |names: &[&str]| json!({ "wait": { "selected": names, "dir": root } });
            for step in [
                selected(&["archive"]),
                click("pictures", "shift"),
                selected(&["archive", "documents", "pictures"]),
                click("documents", "ctrl"),
                selected(&["archive", "pictures"]),
            ] {
                passes(&backend, &kept, &step);
            }

            // The label is the name the view draws, under its row.
            let window = backend.window().expect("the window");
            let mut below = vec![window];
            let mut labels = Vec::new();
            while let Some(node) = below.pop() {
                if backend.role(&node) == "label" && backend.name(&node) == "pictures" {
                    labels.push(node.clone());
                }
                below.extend(backend.children(&node));
            }
            let [label] = labels.as_slice() else {
                panic!("one label named pictures, not {labels:?}");
            };
            let row = backend.parent(label).expect("its row");
            assert_eq!(backend.role(&row), "list item");
            assert_eq!(backend.name(&row), "pictures");
            let (inner, outer) = (backend.frame(label), backend.frame(&row));
            assert!(inner.is_some() && outer.is_some(), "{inner:?} in {outer:?}");
        },
    );
}

/// `selection/right-click-selects-entry`, both variants, step for step, and the rows
/// `dialogs_and_menus/entry-menu-named-actions` and `pane-menu-directory-actions` read.
#[test]
fn a_right_click_opens_an_entry_menu_and_escape_gives_the_keyboard_back() {
    staged(
        "a_right_click_opens_an_entry_menu_and_escape_gives_the_keyboard_back",
        None,
        |backend| {
            let folder = launched(&backend);
            let root = Location::local(folder.path()).display_name();
            let kept = Remembered::default();
            let wait = |predicate: Value| json!({ "wait": predicate });
            for (target, previous) in [("todo.txt", "readme.md"), ("documents", "archive")] {
                for step in [
                    json!({ "select": { "entry": "readme.md", "dir": root } }),
                    wait(json!({ "focused": "readme.md" })),
                    json!({ "click": { "entry": target, "dir": root }, "button": 3 }),
                    wait(json!({ "menu_items": {} })),
                    wait(json!({ "selected": [target], "dir": root })),
                    json!({ "expect": { "description": {
                        "target": { "menu_item": "Copy" }, "text": "Ctrl+C" } } }),
                    json!({ "key": "Escape" }),
                    wait(json!({ "menu_items": null })),
                    wait(json!({ "focused": target })),
                    json!({ "expect": { "panes": [root] } }),
                    json!({ "key": "Up" }),
                    wait(json!({ "focused": previous })),
                    wait(json!({ "selected": [previous], "dir": root })),
                ] {
                    passes(&backend, &kept, &step);
                }
            }

            passes(
                &backend,
                &kept,
                &json!({ "click": { "entry": "todo.txt" }, "button": 3 }),
            );
            let file = json!({ "wait": { "menu_items": { "in_order": [
                "Open", "Open With…", "Quick preview", "Cut", "Copy", "Duplicate", "Rename",
                "Move to…", "Copy to…", "Compress…", "Customize…", "Copy path", "Copy name",
                "Properties", "Move to Trash", "Permanently delete"
            ] } } });
            passes(&backend, &kept, &file);
            passes(&backend, &kept, &json!({ "key": "Escape" }));
            passes(&backend, &kept, &wait(json!({ "menu_items": null })));

            passes(
                &backend,
                &kept,
                &json!({ "click": { "background": true }, "button": 3 }),
            );
            let pane = json!({ "wait": { "menu_items": { "in_order": [
                "New Folder", "New File", "Paste", "Open With…", "Open in Terminal",
                "Select All", "Refresh", "Customize…", "Properties"
            ] } } });
            passes(&backend, &kept, &pane);
            // A row that is wired runs: Show Hidden Files, by its name.
            passes(
                &backend,
                &kept,
                &json!({ "click": { "menu_item": "Show Hidden Files" } }),
            );
            passes(&backend, &kept, &wait(json!({ "menu_items": null })));
            passes(
                &backend,
                &kept,
                &wait(json!({ "entry": ".hidden.txt", "dir": root })),
            );
        },
    );
}

// SPDX-License-Identifier: MIT

//! The error dialog's offered action: Return, which the action's button has, runs it once and
//! closes the dialog; Escape and Close close it and run nothing.

use std::cell::Cell;

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_kit::icons::IconName;

use super::{Error, Offer, show};
use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted, send_key},
};

const MODULE: &str = "error_dialog::tests";

thread_local! {
    /// How often the offered action ran.
    static RAN: Cell<u32> = const { Cell::new(0) };
}

fn is_open(node: &Value) -> bool {
    node["role"] == "dialog"
        || node["children"]
            .as_array()
            .is_some_and(|children| children.iter().any(is_open))
}

fn open(host: &mut Offscreen) {
    host.runtime().update(|app: &mut App, _| {
        let offer = Offer::new("Act", IconName::Lock, |_: &mut App| {
            RAN.set(RAN.get() + 1);
        });
        show(app, Error::new("Failed", "Why it failed").offering(offer));
    });
    let shown = host.run_until(PATIENCE, |host| is_open(&host.tree()));
    assert!(shown, "no dialog");
}

fn closed(host: &mut Offscreen, after: &str) {
    let gone = host.run_until(PATIENCE, |host| !is_open(&host.tree()));
    assert!(gone, "the dialog stayed after {after}");
    while host.turn() {}
}

fn press(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
}

/// Clicks the dialog's element keyed `key`, as a pointer does.
fn click(host: &mut Offscreen, key: &str) {
    while host.turn() {
        host.runtime().settle_all();
    }
    let elements = host.runtime().elements();
    let button = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = button.expect("the button is drawn").visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
}

#[test]
fn the_offered_action_runs_once_on_return_and_never_on_escape_or_close() {
    hosted(
        MODULE,
        "the_offered_action_runs_once_on_return_and_never_on_escape_or_close",
        |mut host| {
            while host.turn() {}

            open(&mut host);
            press(&mut host, Key::Enter);
            closed(&mut host, "Return");
            assert_eq!(RAN.get(), 1, "Return runs the action once");

            open(&mut host);
            press(&mut host, Key::Escape);
            closed(&mut host, "Escape");
            assert_eq!(RAN.get(), 1, "Escape runs nothing");

            open(&mut host);
            click(&mut host, "error-secondary");
            closed(&mut host, "Close");
            assert_eq!(RAN.get(), 1, "Close runs nothing");
        },
    );
}

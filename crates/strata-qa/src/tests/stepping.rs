// SPDX-License-Identifier: MIT

use std::{future::Future, pin::pin, task};

use serde_json::json;

use super::{
    super::{Backend, Outcome, Remembered, execute, parse_command},
    fake::{Fake, Node, WINDOW, drawn},
};

/// The in-memory backend never waits, so one poll finishes any step.
fn finish<Output>(step: impl Future<Output = Output>) -> Output {
    let mut context = task::Context::from_waker(task::Waker::noop());
    match pin!(step).poll(&mut context) {
        task::Poll::Ready(output) => output,
        task::Poll::Pending => panic!("the in-memory backend should never wait"),
    }
}

fn run(fake: &Fake, kept: &Remembered<Fake>, step: serde_json::Value) -> Outcome {
    let command = parse_command(&step.to_string()).expect("the step should parse");
    finish(execute(fake, kept, command))
}

fn button(fake: &mut Fake) -> usize {
    fake.add(WINDOW, drawn("button", "Open", [100.0, 200.0, 80.0, 40.0]))
}

#[test]
fn a_click_approaches_its_target_and_holds_the_modifiers_around_the_press() {
    let mut fake = Fake::new();
    button(&mut fake);
    let kept = Remembered::default();
    let outcome = run(
        &fake,
        &kept,
        json!({ "click": { "role": "button", "name": "Open" }, "mods": ["ctrl"], "count": 2 }),
    );
    assert_eq!((outcome.error, outcome.failures.len()), (None, 0));
    assert_eq!(
        *fake.sent.borrow(),
        [
            "move 134 214",
            "move 140 220",
            "hold [Ctrl]",
            "button 1 down",
            "button 1 up",
            "button 1 down",
            "button 1 up",
            "release [Ctrl]",
        ]
    );
}

#[test]
fn a_wheel_turn_is_one_press_of_button_four_or_five_for_each_click() {
    let mut fake = Fake::new();
    button(&mut fake);
    let kept = Remembered::default();
    let outcome = run(
        &fake,
        &kept,
        json!({ "wheel": { "role": "button", "name": "Open" }, "clicks": 2, "direction": "down" }),
    );
    assert_eq!(outcome.error, None);
    assert_eq!(
        fake.sent.borrow()[2..],
        [
            "button 5 down",
            "button 5 up",
            "button 5 down",
            "button 5 up"
        ]
    );
}

#[test]
fn remember_keeps_a_node_with_its_box_and_keep_only_a_box() {
    let mut fake = Fake::new();
    let open = button(&mut fake);
    let kept = Remembered::default();
    let remembered = run(
        &fake,
        &kept,
        json!({ "remember": { "role": "button", "name": "Open" }, "as": "open" }),
    );
    assert_eq!(remembered.error, None);
    assert_eq!(kept.nodes.borrow().get("open"), Some(&fake.keep(&open)));
    assert_eq!(
        kept.boxes.borrow().get("open").map(|rect| rect.pixels()),
        Some([100, 200, 80, 40])
    );
    assert!(kept.fields.borrow().is_empty());

    assert_eq!(
        run(&fake, &kept, json!({ "keep": [1, 2, 3, 4], "as": "spot" })).error,
        None
    );
    assert_eq!(
        kept.boxes.borrow().get("spot").map(|rect| rect.pixels()),
        Some([1, 2, 3, 4])
    );
    let held = run(
        &fake,
        &kept,
        json!({ "holds": { "moved": {
            "target": { "role": "button", "name": "Open" }, "since": "spot", "direction": "right"
        } } }),
    );
    assert_eq!(
        held.result,
        json!(true),
        "the button is right of the kept box"
    );
}

#[test]
fn activation_and_typing_go_to_the_backend_and_an_unknown_step_says_so() {
    let mut fake = Fake::new();
    fake.add(
        WINDOW,
        Node {
            role: "button",
            name: "Hidden but presented",
            ..Node::default()
        },
    );
    let kept = Remembered::default();
    assert_eq!(
        run(
            &fake,
            &kept,
            json!({ "activate": { "role": "button", "name": "Hidden but presented" } })
        )
        .error,
        None,
        "a plain role and name is activated without being drawn"
    );
    assert_eq!(run(&fake, &kept, json!({ "type": "ab" })).error, None);
    assert_eq!(
        run(&fake, &kept, json!({ "key": "ctrl+Return" })).error,
        None
    );
    assert_eq!(
        *fake.sent.borrow(),
        [
            "activate Hidden but presented",
            "type a",
            "type b",
            "chord Return"
        ]
    );
    assert_eq!(
        run(
            &fake,
            &kept,
            json!({ "reveal": { "role": "button", "name": "Hidden but presented" } })
        )
        .error
        .as_deref(),
        Some("reveal is not supported on this target")
    );
}

// SPDX-License-Identifier: MIT

use std::{future::Future, pin::pin, task};

use serde_json::json;

use super::{
    super::{Backend, Mark, Outcome, Remembered, execute, parse_request},
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
    let command = parse_request(&step.to_string()).expect("the step should parse");
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
    assert_eq!(
        run(
            &fake,
            &kept,
            json!({ "keymap": { "layouts": "us,ru", "group": 1 } })
        )
        .error
        .as_deref(),
        Some("keymap is not supported on this target"),
        "a toolkit that types through a display server takes its layout from that server"
    );
}

#[derive(Clone, Copy)]
struct LateList {
    first: usize,
    target: usize,
    middle: usize,
    last: usize,
    reveal_at: usize,
}

fn down_in_late_list(fake: &Fake, rows: LateList) {
    let cur = fake.live_focus.get();
    if cur == Some(rows.reveal_at) {
        fake.shown_later.borrow_mut().push(rows.target);
    }
    let next = match cur {
        Some(node) if node == rows.first => {
            match fake.shown_later.borrow().contains(&rows.target) {
                true => rows.target,
                false => rows.middle,
            }
        }
        Some(node) if node == rows.middle => rows.last,
        _ => return,
    };
    fake.live_focus.set(Some(next));
}

fn build_late_list(fake: &mut Fake, before_end: bool) -> LateList {
    let pane = fake.add(
        WINDOW,
        Node {
            mark: Some(Mark::Pane("List")),
            ..drawn("grouping", "home", [0.0, 0.0, 400.0, 600.0])
        },
    );
    let list = fake.add(
        pane,
        Node {
            mark: Some(Mark::EntryContainer),
            ..drawn("list", "home", [0.0, 40.0, 400.0, 560.0])
        },
    );
    let add = |fake: &mut Fake, name, y, hidden| {
        fake.add(
            list,
            Node {
                mark: Some(Mark::Entry),
                hidden,
                ..drawn("list item", name, [0.0, y, 300.0, 26.0])
            },
        )
    };
    let first = add(fake, "alpha.txt", 40.0, false);
    let target = add(fake, "scrolling", 66.0, true);
    let middle = add(fake, "readme.md", 92.0, false);
    let last = add(fake, "todo.txt", 118.0, false);
    LateList {
        first,
        target,
        middle,
        last,
        reveal_at: if before_end { middle } else { last },
    }
}

#[test]
fn select_with_keyboard_rehomes_when_a_late_entry_appears_above_the_cursor() {
    for (before_end, expected) in [
        (
            false,
            vec![
                "chord Home",
                "chord Down",
                "chord Down",
                "chord Down",
                "chord Home",
                "chord Down",
            ],
        ),
        (
            true,
            vec![
                "chord Home",
                "chord Down",
                "chord Down",
                "chord Home",
                "chord Down",
            ],
        ),
    ] {
        let mut fake = Fake::new();
        let rows = build_late_list(&mut fake, before_end);
        fake.on_chord = Some(Box::new(move |fake, chord| match chord.key.as_str() {
            "Home" => fake.live_focus.set(Some(rows.first)),
            "Down" => down_in_late_list(fake, rows),
            _ => {}
        }));
        let kept = Remembered::default();
        let outcome = run(&fake, &kept, json!({ "select_with_keyboard": "scrolling" }));
        assert_eq!((outcome.error, outcome.failures.len()), (None, 0));
        assert_eq!(fake.live_focus.get(), Some(rows.target));
        assert_eq!(*fake.sent.borrow(), expected, "before_end={before_end}");
    }
}

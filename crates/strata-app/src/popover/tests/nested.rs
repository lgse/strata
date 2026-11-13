// SPDX-License-Identifier: MIT

//! A plain popover opened from another, as A8's "Customize columns…" is from its Appearance
//! popover, on made-up parts: the keyboard going into the child leaves the parent open, and a
//! press outside the child, on the parent's surface included, closes both.

use std::{any::TypeId, cell::Cell};

use rune::{El, Tx, col, el, row};
use rune_kit::Button;

use super::{centre, click_at, idle};
use crate::{
    App,
    commands::Holds,
    popover::{Parent, Plain, focus_left},
    slots::{Entry, Frame, Place},
    test_support::{PATIENCE, hosted_over, idle_view},
};

const MODULE: &str = "popover::tests::nested";

const ANCHOR: &str = "made-anchor";
const PARENT_STOPS: [&str; 2] = ["made-parent-a", "made-child-anchor"];
const CHILD_STOPS: [&str; 1] = ["made-child-a"];

thread_local! {
    static PARENT: Cell<bool> = const { Cell::new(false) };
    static CHILD: Cell<bool> = const { Cell::new(false) };
}

fn close_parent(_: &mut App) {
    PARENT.set(false);
}

fn close_child(_: &mut App) {
    CHILD.set(false);
}

fn button(app: &App, key: &'static str) -> El {
    Button::new(key.to_owned()).build(&app.look.kit).key(key)
}

fn made_view(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    Some(el().key("made-view").w(40.0).h(20.0).focusable(true))
}

/// The anchor, and the two popovers while they are open.
fn nest(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let look = &app.look;
    if PARENT.get() {
        let body = col()
            .padding(16.0)
            .child(button(app, PARENT_STOPS[0]))
            .child(button(app, PARENT_STOPS[1]));
        let parent = Plain {
            key: "made-parent",
            anchor: ANCHOR,
            label: "Parent".to_owned(),
            stops: PARENT_STOPS.to_vec(),
            own_arrows: &[],
            listing: None,
            close: close_parent,
            parent: None,
        };
        parent.build(look, body, frame);
    }
    if CHILD.get() {
        let body = col().padding(16.0).child(button(app, CHILD_STOPS[0]));
        let child = Plain {
            key: "made-child",
            anchor: PARENT_STOPS[1],
            label: "Child".to_owned(),
            stops: CHILD_STOPS.to_vec(),
            own_arrows: &[],
            listing: None,
            close: close_child,
            parent: Some(Parent {
                close: close_parent,
                anchor: ANCHOR,
            }),
        };
        child.build(look, body, frame);
    }
    Some(row().absolute(400.0, 200.0).child(button(app, ANCHOR)))
}

/// Each popover closes when the keyboard leaves it, the parent counting its open child's
/// controls as its own.
fn poll(_: &mut App, _: f64, tx: &mut Tx) -> bool {
    let child: &[&str] = if CHILD.get() { &CHILD_STOPS } else { &[] };
    let mut changed = false;
    if CHILD.get() && focus_left(tx, PARENT_STOPS[1], &CHILD_STOPS, &[]) {
        CHILD.set(false);
        changed = true;
    }
    if PARENT.get() && focus_left(tx, ANCHOR, &PARENT_STOPS, child) {
        PARENT.set(false);
        changed = true;
    }
    changed
}

fn parts() -> Vec<Entry<App>> {
    struct View;
    struct Nest;
    let entry = |name, kind, place, poll| Entry {
        name,
        kind,
        place,
        listen: |_| None,
        publish: |_| {},
        caught_up: |_| {},
        poll,
        switched: |_, _| {},
        commands: &[],
        keyboard: |_| (PARENT.get() || CHILD.get()).then_some(Holds::Popover),
    };
    let no_poll: fn(&mut App, f64, &mut Tx) -> bool = |_, _, _| false;
    vec![
        entry(
            "made-view",
            TypeId::of::<View>,
            Place::View(idle_view("made-view", made_view, |_| true)),
            no_poll,
        ),
        entry("made-nest", TypeId::of::<Nest>, Place::Overlays(nest), poll),
    ]
}

#[test]
fn the_keyboard_in_the_child_keeps_the_parent_and_a_press_outside_closes_both() {
    let name = "the_keyboard_in_the_child_keeps_the_parent_and_a_press_outside_closes_both";
    hosted_over(MODULE, name, parts, |mut host| {
        host.runtime().update(|_: &mut App, tx: &mut Tx| {
            PARENT.set(true);
            tx.focus(PARENT_STOPS[0], true);
        });
        idle(&mut host);
        // The child opens from its button, which has the keyboard as it is pressed.
        host.runtime()
            .update(|_: &mut App, tx: &mut Tx| tx.focus(PARENT_STOPS[1], true));
        idle(&mut host);
        host.runtime().update(|_: &mut App, tx: &mut Tx| {
            CHILD.set(true);
            tx.focus(CHILD_STOPS[0], true);
        });
        idle(&mut host);
        for _ in 0..10 {
            host.turn();
        }
        assert!(
            CHILD.get() && PARENT.get(),
            "the child's keyboard keeps the parent open"
        );

        let on_parent = centre(&mut host, PARENT_STOPS[0]);
        click_at(&mut host, on_parent);
        let closed = host.run_until(PATIENCE, |_| !CHILD.get() && !PARENT.get());
        assert!(closed, "a press on the parent's surface closes both");
        assert_eq!(
            super::focused(&mut host).as_deref(),
            Some(ANCHOR),
            "the keyboard goes to the parent's button"
        );
    });
}

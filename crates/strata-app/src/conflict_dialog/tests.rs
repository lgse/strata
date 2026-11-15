// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use rune::{Key, KeyEvent, Mods};
use strata_components::dialog::Focus;
use strata_fs::{adapters::TransferTargetCollision, services::TransferConflict};

use super::{
    ConflictActions, ConflictChoice, ConflictFocus, ConflictPrompt, covered_by_answer,
    initial_focus, is_open, prompt, toggle_apply_all,
};
use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted, send_key},
};

const MODULE: &str = "conflict_dialog::tests";

fn press(host: &mut Offscreen, key: Key, shift: bool) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods {
                shift,
                ..Mods::default()
            },
        },
    );
    host.turn();
    while host.turn() {}
}

#[test]
fn visibility_initial_focus_and_apply_to_all_coverage_follow_gtk_rules() {
    assert_eq!(ConflictPrompt::visibility(0, 0), (false, false));
    assert_eq!(ConflictPrompt::visibility(0, 1), (false, true));
    assert_eq!(ConflictPrompt::visibility(2, 0), (true, true));

    for (actions, expected) in [
        (
            ConflictActions {
                replace: true,
                keep_both: true,
                merge: false,
                focus: ConflictFocus::Replace,
            },
            Focus::Primary,
        ),
        (
            ConflictActions {
                replace: true,
                keep_both: true,
                merge: false,
                focus: ConflictFocus::KeepBoth,
            },
            Focus::Action("keep-both".to_owned()),
        ),
        (
            ConflictActions {
                replace: false,
                keep_both: true,
                merge: false,
                focus: ConflictFocus::Replace,
            },
            Focus::Action("keep-both".to_owned()),
        ),
        (
            ConflictActions {
                replace: false,
                keep_both: false,
                merge: false,
                focus: ConflictFocus::Replace,
            },
            Focus::Action("cancel".to_owned()),
        ),
    ] {
        assert_eq!(initial_focus(actions), expected, "{actions:?}");
    }

    let remaining = vec![
        ("same-kind", TransferTargetCollision::between(false, false)),
        ("mismatched", TransferTargetCollision::between(false, true)),
    ];
    let (covered, rest) =
        covered_by_answer(remaining, TransferConflict::ReplaceExisting, |(_, k)| *k);
    assert_eq!(covered.len(), 1);
    assert_eq!(covered[0].0, "same-kind");
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].0, "mismatched");
}

#[test]
fn keyboard_navigation_apply_to_all_and_escape_dismissal_work_across_chained_prompts() {
    hosted(
        MODULE,
        "keyboard_navigation_apply_to_all_and_escape_dismissal_work_across_chained_prompts",
        |mut host| {
            let chosen: Rc<RefCell<Vec<(ConflictChoice, bool)>>> = Rc::default();
            let chosen_esc = chosen.clone();
            host.runtime().update(move |app: &mut App, _| {
                prompt(
                    app,
                    ConflictPrompt {
                        name: "todo.txt".to_owned(),
                        explanation: "Already exists.".to_owned(),
                        apply_to_all_visible: true,
                        skip_visible: true,
                        actions: ConflictActions {
                            replace: true,
                            keep_both: true,
                            merge: false,
                            focus: ConflictFocus::Replace,
                        },
                    },
                    move |_, choice, apply| chosen_esc.borrow_mut().push((choice, apply)),
                );
            });
            while host.turn() {}
            assert!(is_open(host.app()));

            press(&mut host, Key::Escape, false);
            assert!(!is_open(host.app()));
            assert!(chosen.borrow().is_empty(), "Escape must abandon");

            let chosen_chain = chosen.clone();
            host.runtime().update(move |app: &mut App, _| {
                let inner = chosen_chain.clone();
                prompt(
                    app,
                    ConflictPrompt {
                        name: "first.txt".to_owned(),
                        explanation: "First.\n\nSecond paragraph.".to_owned(),
                        apply_to_all_visible: true,
                        skip_visible: true,
                        actions: ConflictActions {
                            replace: true,
                            keep_both: true,
                            merge: false,
                            focus: ConflictFocus::Replace,
                        },
                    },
                    move |app, choice, apply| {
                        inner.borrow_mut().push((choice, apply));
                        let inner2 = inner.clone();
                        prompt(
                            app,
                            ConflictPrompt {
                                name: "second.txt".to_owned(),
                                explanation: "Second.".to_owned(),
                                apply_to_all_visible: true,
                                skip_visible: false,
                                actions: ConflictActions {
                                    replace: true,
                                    keep_both: true,
                                    merge: false,
                                    focus: ConflictFocus::Replace,
                                },
                            },
                            move |_, choice2, apply2| {
                                inner2.borrow_mut().push((choice2, apply2));
                            },
                        );
                    },
                );
                toggle_apply_all(app);
            });
            while host.turn() {}

            // Shift+Tab from Replace lands on Keep Both; Return activates Keep Both.
            press(&mut host, Key::Tab, true);
            press(&mut host, Key::Enter, false);
            assert!(host.run_until(PATIENCE, |_| !chosen.borrow().is_empty()));
            while host.turn() {}
            assert_eq!(*chosen.borrow(), vec![(ConflictChoice::KeepBoth, true)]);
            assert!(is_open(host.app()), "chained prompt opens immediately");

            // The chained prompt resets Apply to All to false and starts on Replace.
            press(&mut host, Key::Enter, false);
            assert!(host.run_until(PATIENCE, |_| chosen.borrow().len() == 2));
            while host.turn() {}
            assert_eq!(
                *chosen.borrow(),
                vec![
                    (ConflictChoice::KeepBoth, true),
                    (ConflictChoice::Replace, false),
                ]
            );
            assert!(!is_open(host.app()));
        },
    );
}

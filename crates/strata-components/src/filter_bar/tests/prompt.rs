// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods};

use crate::filter_bar::prompt::{
    By, Candidate, Candidates, Polled, PromptIntent, PromptSpec, PromptState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum P {
    Find,
    Filter,
    Go,
    Rename,
}

fn press(key: Key) -> KeyEvent {
    KeyEvent {
        key,
        mods: Mods::default(),
    }
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        key: Key::Command(c),
        mods: Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        },
    }
}

fn query(p: &Polled<P>) -> u64 {
    p.intents
        .iter()
        .find_map(|i| match i {
            PromptIntent::Complete { query, .. } => Some(query.generation),
            _ => None,
        })
        .expect("a listing asked for")
}

fn folders(names: &[&str]) -> Vec<Candidate> {
    names
        .iter()
        .map(|n| Candidate::folder(*n, *n, "~", format!("~/{n}/")))
        .collect()
}

fn submit(p: &Polled<P>) -> Option<(String, Option<String>, bool, bool)> {
    p.intents.iter().find_map(|i| match i {
        PromptIntent::Submit {
            text,
            chosen,
            stepped,
            settled,
            ..
        } => Some((text.clone(), chosen.clone(), *stepped, *settled)),
        _ => None,
    })
}

#[test]
fn a_new_prompt_replaces_the_open_one_without_its_text() {
    let mut st = PromptState::new();
    let _ = st.open(P::Go, PromptSpec::go(), "", None);
    st.edit.text = "smb://user:secret@host".into();
    let out = st.open(P::Filter, PromptSpec::filter(), "", None);
    assert!(out.intents.contains(&PromptIntent::CancelCompletion));
    assert!(out.intents.contains(&PromptIntent::Dismissed {
        kind: P::Go,
        text: "smb://user:secret@host".into(),
        by: By::Replaced
    }));
    assert_eq!(st.kind(), Some(P::Filter));
    assert_eq!(st.text(), "");
}

#[test]
fn a_prefilled_prompt_keeps_its_text_unselected() {
    let mut st = PromptState::new();
    let _ = st.open(P::Filter, PromptSpec::filter(), "rep md", None);
    assert_eq!(st.edit.range(), (6, 6));
    assert!(st.focused());
}

// integration/10xer-mode/file-verbs: "`r` and F2 open `rename ›`… A file's stem… is selected."
#[test]
fn rename_opens_with_the_range_the_host_names_selected() {
    let mut st = PromptState::new();
    let _ = st.open(
        P::Rename,
        PromptSpec::rename(),
        "report.final.pdf",
        Some(0..12),
    );
    assert_eq!(st.edit.range(), (0, 12));
}

#[test]
fn up_and_down_step_the_listing_choose_a_candidate_or_do_nothing() {
    let mut st = PromptState::new();
    let _ = st.open(P::Find, PromptSpec::find(), "", None);
    let out = st.key(&press(Key::Down), false).expect("taken");
    assert_eq!(
        out.intents,
        vec![PromptIntent::Step {
            kind: P::Find,
            delta: 1
        }]
    );

    let out = st.open(P::Go, PromptSpec::go(), "", None);
    let g = query(&out);
    let _ = st.deliver(Candidates {
        generation: g,
        items: folders(&["a", "b", "c"]),
        done: true,
    });
    let _ = st.key(&press(Key::Up), false).expect("taken");
    assert_eq!(
        st.chosen_index(),
        Some(2),
        "Up from the first wraps to the last"
    );
    assert!(st.stepped());

    let _ = st.open(P::Rename, PromptSpec::rename(), "a", None);
    let out = st.key(&press(Key::Down), false).expect("taken");
    assert_eq!(out, Polled::default());
}

#[test]
fn a_chosen_candidate_survives_later_batches_and_a_stale_answer_is_dropped() {
    let mut st = PromptState::new();
    let g = query(&st.open(P::Go, PromptSpec::go(), "", None));
    let _ = st.deliver(Candidates {
        generation: g,
        items: folders(&["a", "b"]),
        done: false,
    });
    assert_eq!(st.chosen().map(|c| c.id.as_str()), Some("a"));
    let _ = st.key(&press(Key::Down), false);
    let _ = st.deliver(Candidates {
        generation: g,
        items: folders(&["c", "a", "b"]),
        done: false,
    });
    assert_eq!(st.chosen().map(|c| c.id.as_str()), Some("b"));
    assert!(st.stepped(), "the choice is still the user's");
    // A third batch: the choice holds again, at its new place.
    let _ = st.deliver(Candidates {
        generation: g,
        items: folders(&["b", "c", "a"]),
        done: true,
    });
    assert_eq!(st.chosen().map(|c| c.id.as_str()), Some("b"));
    assert_eq!(st.chosen_index(), Some(0));
    assert!(st.stepped());

    st.edit.text = "x".into();
    let next = query(&st.edited());
    assert!(next > g);
    assert_eq!(
        st.deliver(Candidates {
            generation: g,
            items: folders(&["stale"]),
            done: true,
        }),
        Polled::default()
    );
    assert_eq!(
        st.candidates().len(),
        3,
        "the previous list stays until the new one lands"
    );
}

#[test]
fn tab_writes_the_chosen_candidate_and_lists_again() {
    let mut st = PromptState::new();
    let g = query(&st.open(P::Go, PromptSpec::go(), "~/d", None));
    let _ = st.deliver(Candidates {
        generation: g,
        items: folders(&["dev", "docs"]),
        done: true,
    });
    let _ = st.key(&press(Key::Down), false);
    let out = st.key(&press(Key::Tab), false).expect("taken");
    assert_eq!(st.text(), "~/docs/");
    assert!(submit(&out).is_none());
    assert!(
        out.intents
            .iter()
            .any(|i| matches!(i, PromptIntent::Edited { .. }))
    );
    assert!(query(&out) > g);
}

#[test]
fn a_submit_can_wait_for_the_listing_to_finish() {
    let mut st = PromptState::new();
    let g = query(&st.open(P::Go, PromptSpec::go(), "~/d", None));
    let out = st.key(&press(Key::Enter), false).expect("taken");
    let (_, _, _, settled) = submit(&out).expect("a submit");
    assert!(!settled);
    assert!(st.wait_until_settled(st.serial()));
    let _ = st.deliver(Candidates {
        generation: g,
        items: folders(&["dev"]),
        done: false,
    });
    let out = st.deliver(Candidates {
        generation: g,
        items: folders(&["dev", "docs"]),
        done: true,
    });
    assert_eq!(
        submit(&out),
        Some(("~/d".into(), Some("dev".into()), false, true))
    );
    assert!(!st.wait_until_settled(st.serial()), "nothing pending now");

    let g = query(&st.open(P::Go, PromptSpec::go(), "~/d", None));
    assert!(st.wait_until_settled(st.serial()));
    st.edit.text = "~/do".into();
    let g2 = query(&st.edited());
    let _ = st.deliver(Candidates {
        generation: g2,
        items: folders(&["docs"]),
        done: true,
    });
    assert!(g2 > g);
    assert!(st.text() == "~/do");
}

#[test]
fn a_check_answer_after_an_edit_or_a_dismissal_is_dropped() {
    let mut st = PromptState::new();
    let _ = st.open(P::Rename, PromptSpec::rename(), "notes.txt", None);
    let asked = st.serial();
    st.edit.text = "notes2.txt".into();
    let _ = st.edited();
    assert!(!st.refuse(asked, "“notes.txt” already exists"));
    assert!(!st.invalid());
    let asked = st.serial();
    assert!(st.refuse(asked, "“notes2.txt” already exists"));
    assert!(st.invalid());
    assert_eq!(st.hint(), Some("“notes2.txt” already exists"));
    let _ = st.key(&press(Key::Escape), false);
    assert!(!st.is_open());
    assert!(!st.show_hint(asked, Some("late".into())));
}

// integration/10xer-mode/places: `go ›` "keeps no undo history".
#[test]
fn a_prompt_without_undo_swallows_the_undo_keys() {
    let mut st = PromptState::new();
    let _ = st.open(P::Go, PromptSpec::go(), "", None);
    assert_eq!(st.key(&ctrl('z'), false), Some(Polled::default()));
    assert_eq!(st.key(&ctrl('y'), false), Some(Polled::default()));
    let _ = st.open(P::Find, PromptSpec::find(), "", None);
    assert_eq!(st.key(&ctrl('z'), false), None, "the editor undoes");
}

#[test]
fn losing_the_keyboard_dismisses_the_prompt() {
    let mut st = PromptState::new();
    let _ = st.open(P::Rename, PromptSpec::rename(), "a", None);
    assert_eq!(st.tick(), Polled::default());
    st.edit.focused = false;
    let out = st.tick();
    assert_eq!(
        out.intents,
        vec![PromptIntent::Dismissed {
            kind: P::Rename,
            text: "a".into(),
            by: By::Blur
        }]
    );
    assert!(!st.is_open());
}

// A held Enter or Escape acts once.
#[test]
fn repeats_of_enter_and_escape_do_nothing() {
    let mut st = PromptState::new();
    let _ = st.open(P::Find, PromptSpec::find(), "x", None);
    assert_eq!(st.key(&press(Key::Enter), true), Some(Polled::default()));
    assert_eq!(st.key(&press(Key::Escape), true), Some(Polled::default()));
    assert!(st.is_open());
}

#[test]
fn a_stale_batch_never_lands_in_a_later_prompt() {
    let mut st = PromptState::new();
    let g = query(&st.open(P::Go, PromptSpec::go(), "", None));
    let _ = st.key(&press(Key::Escape), false);
    let _ = st.open(P::Rename, PromptSpec::rename(), "notes.txt", None);
    let out = st.deliver(Candidates {
        generation: g,
        items: folders(&["stale"]),
        done: true,
    });
    assert_eq!(out, Polled::default());
    assert!(st.candidates().is_empty());
    let out = st.key(&press(Key::Enter), false).expect("taken");
    assert_eq!(
        submit(&out).map(|s| s.1),
        Some(None),
        "no candidate is chosen"
    );

    // A prompt closed and opened again of the same kind asks afresh: the old answer is stale.
    let g = query(&st.open(P::Go, PromptSpec::go(), "", None));
    let _ = st.close();
    let g2 = query(&st.open(P::Go, PromptSpec::go(), "", None));
    assert_ne!(g, g2);
    assert_eq!(
        st.deliver(Candidates {
            generation: g,
            items: folders(&["stale"]),
            done: true,
        }),
        Polled::default()
    );
}

#[test]
fn an_edit_or_a_tab_forgets_the_choice() {
    let mut st = PromptState::new();
    let g = query(&st.open(P::Go, PromptSpec::go(), "~/d", None));
    let _ = st.deliver(Candidates {
        generation: g,
        items: folders(&["dev", "docs"]),
        done: true,
    });
    let _ = st.key(&press(Key::Down), false);
    assert!(st.stepped());
    st.edit.text = "/etc".into();
    let _ = st.edited();
    let out = st.key(&press(Key::Enter), false).expect("taken");
    let (text, _, stepped, settled) = submit(&out).expect("a submit");
    assert_eq!(text, "/etc");
    assert!(!stepped, "the choice was for other text");
    assert!(!settled, "the new listing hasn't come");

    let g = query(&st.open(P::Go, PromptSpec::go(), "~/d", None));
    let _ = st.deliver(Candidates {
        generation: g,
        items: folders(&["dev", "docs"]),
        done: true,
    });
    let _ = st.key(&press(Key::Down), false);
    let _ = st.key(&press(Key::Tab), false);
    assert_eq!(st.text(), "~/docs/");
    assert!(!st.stepped(), "Tab's write is a change too");
}

#[test]
fn shift_tab_completes_nothing() {
    let mut st = PromptState::new();
    let g = query(&st.open(P::Go, PromptSpec::go(), "~/d", None));
    let _ = st.deliver(Candidates {
        generation: g,
        items: folders(&["dev"]),
        done: true,
    });
    let shift_tab = KeyEvent {
        key: Key::Tab,
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
    };
    assert_eq!(st.key(&shift_tab, false), None, "the key goes on");
    assert_eq!(st.text(), "~/d");
}

// A hint for older text is dropped while the prompt is still open, as a late check's is.
#[test]
fn a_hint_for_older_text_is_dropped() {
    let mut st = PromptState::new();
    let _ = st.open(P::Go, PromptSpec::go(), "", None);
    let asked = st.serial();
    st.edit.text = "~/x".into();
    let _ = st.edited();
    assert!(!st.show_hint(asked, Some("No matching folders".into())));
    assert_eq!(st.hint(), None);
    assert!(st.show_hint(st.serial(), Some("Searching…".into())));
    assert_eq!(st.hint(), Some("Searching…"));
}

#[test]
fn an_edit_drops_a_waiting_submit() {
    let mut st = PromptState::new();
    let _ = st.open(P::Go, PromptSpec::go(), "~/d", None);
    assert!(st.wait_until_settled(st.serial()));
    st.edit.text = "~/do".into();
    let g = query(&st.edited());
    let out = st.deliver(Candidates {
        generation: g,
        items: folders(&["docs"]),
        done: true,
    });
    assert!(
        submit(&out).is_none(),
        "the edit dropped the waiting submit"
    );
}

#[test]
fn a_held_prompt_survives_losing_the_keyboard_until_released() {
    let mut st = PromptState::new();
    let _ = st.open(P::Filter, PromptSpec::filter(), "rep", None);
    let _ = st.tick();
    st.hold_open(true);
    st.edit.focused = false;
    assert_eq!(st.tick(), Polled::default());
    assert!(st.is_open(), "held while the reference has the keyboard");
    // The reference closes and gives the keyboard back: the prompt stays.
    st.edit.focused = true;
    st.hold_open(false);
    assert_eq!(st.tick(), Polled::default());
    assert!(st.is_open());
    // Held again, and released with the keyboard elsewhere: it closes.
    st.hold_open(true);
    st.edit.focused = false;
    let _ = st.tick();
    st.hold_open(false);
    let out = st.tick();
    assert!(out.intents.contains(&PromptIntent::Dismissed {
        kind: P::Filter,
        text: "rep".into(),
        by: By::Blur
    }));
    assert!(!st.is_open());
}

#[test]
fn closing_a_prompt_drops_its_hold() {
    let mut st = PromptState::new();
    let _ = st.open(P::Find, PromptSpec::find(), "rep", None);
    st.hold_open(true);
    let _ = st.close();
    let _ = st.open(P::Find, PromptSpec::find(), "", None);
    let _ = st.tick();
    st.edit.focused = false;
    let out = st.tick();
    assert!(
        out.intents
            .iter()
            .any(|i| matches!(i, PromptIntent::Dismissed { by: By::Blur, .. }))
    );
    assert!(!st.is_open());
}

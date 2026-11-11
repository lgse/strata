use rune::Tx;

use super::*;
use crate::contract::dnd::DropHover;

fn intents(p: Polled) -> Vec<LocationIntent> {
    p.intents
}

fn query(p: &Polled) -> CompletionQuery {
    p.intents
        .iter()
        .find_map(|i| match i {
            LocationIntent::Complete(q) => Some(q.clone()),
            _ => None,
        })
        .expect("a query")
}

fn two() -> Vec<Candidate> {
    vec![
        Candidate::new("Desktop/", "~/Desktop/"),
        Candidate::new("Documents/", "~/Documents/"),
    ]
}

// browser/navigation/location-bar: "Crumb labels longer than 32 characters are shortened in
// the middle."
#[test]
fn long_labels_shorten_in_the_middle() {
    assert_eq!(elide_middle("short", CRUMB_CHARS), "short");
    let exact = "a".repeat(CRUMB_CHARS);
    assert_eq!(elide_middle(&exact, CRUMB_CHARS), exact);
    let long = "a-rather-long-folder-name-to-shorten-in-the-middle";
    let cut = elide_middle(long, CRUMB_CHARS);
    assert_eq!(cut.chars().count(), CRUMB_CHARS);
    assert_eq!(cut, "a-rather-long-f…en-in-the-middle");
    // Characters, not bytes.
    let wide = "é".repeat(40);
    assert_eq!(
        elide_middle(&wide, CRUMB_CHARS).chars().count(),
        CRUMB_CHARS
    );
}

#[test]
fn begin_edit_selects_the_whole_path_and_asks_for_suggestions() {
    let mut s = LocationBarState::new();
    let p = s.begin_edit("~/Projects");
    assert!(s.editing());
    assert!(s.edit.focused);
    assert_eq!(s.edit.range(), (0, "~/Projects".len()));
    assert_eq!(query(&p).text, "~/Projects");
}

#[test]
fn escape_and_submit_end_the_edit_and_drop_the_listing() {
    let mut s = LocationBarState::new();
    let _ = s.begin_edit("~");
    assert_eq!(
        intents(s.cancel()),
        [LocationIntent::CancelCompletion, LocationIntent::Cancelled]
    );
    assert!(!s.editing());
    let _ = s.begin_edit("~");
    assert_eq!(
        intents(s.submit("/tmp".into())),
        [
            LocationIntent::CancelCompletion,
            LocationIntent::Submit {
                text: "/tmp".into()
            }
        ]
    );
}

#[test]
fn a_press_elsewhere_cancels_on_the_next_tick_only_after_the_entry_had_focus() {
    let mut s = LocationBarState::new();
    let mut tx = Tx::default();
    let _ = s.begin_edit("~");
    // Rune hasn't focused the field yet: nothing ends.
    s.edit.focused = false;
    assert!(intents(s.tick(0.0, &mut tx)).is_empty());
    s.edit.focused = true;
    let _ = s.tick(0.1, &mut tx);
    s.edit.focused = false;
    let out = intents(s.tick(0.2, &mut tx));
    assert!(out.contains(&LocationIntent::Cancelled), "{out:?}");
    assert!(!s.editing());
}

#[test]
fn a_late_or_stale_answer_is_dropped() {
    let mut s = LocationBarState::new();
    let first = query(&s.begin_edit("~"));
    s.edit.replace_text("~/D");
    let second = query(&s.edited());
    assert!(!s.deliver(Completions::new(first.generation, two())).changed);
    assert!(!s.suggestions_open());
    assert!(
        s.deliver(Completions::new(second.generation, two()))
            .changed
    );
    assert!(s.suggestions_open());
    // After the edit ends, an answer for it shows nothing.
    let third = query(&s.edited());
    let _ = s.cancel();
    let _ = s.deliver(Completions::new(third.generation, two()));
    assert!(!s.suggestions_open());
}

#[test]
fn navigating_keeps_an_entry_being_typed_in() {
    let mut s = LocationBarState::new();
    let _ = s.begin_edit("~");
    s.edit.replace_text("~/Mu");
    let _ = s.navigated();
    assert!(s.editing(), "the focused entry stays open");
    assert_eq!(s.edit.text, "~/Mu");
    s.edit.focused = false;
    let _ = s.navigated();
    assert!(
        !s.editing(),
        "an entry without the keyboard gives way to the crumbs"
    );
}

#[test]
fn a_resting_drag_springs_once_after_its_delay() {
    let mut s = LocationBarState::new();
    let mut tx = Tx::default();
    s.drag_over(
        1,
        "/home/demo",
        DropHover::Accepting,
        Some(SPRING_DELAY),
        1.0,
    );
    assert!(intents(s.tick(1.0, &mut tx)).is_empty());
    assert!(intents(s.tick(1.5, &mut tx)).is_empty());
    assert_eq!(
        intents(s.tick(1.0 + SPRING_DELAY, &mut tx)),
        [LocationIntent::Open {
            id: "/home/demo".into(),
            via: Via::Spring
        }]
    );
    assert!(intents(s.tick(3.0, &mut tx)).is_empty(), "once");
    // Leaving before the delay disarms it, and so does spring-loading being off.
    s.drag_over(
        1,
        "/home/demo",
        DropHover::Accepting,
        Some(SPRING_DELAY),
        4.0,
    );
    let _ = s.tick(4.0, &mut tx);
    s.drag_over(1, "/home/demo", DropHover::Left, Some(SPRING_DELAY), 4.5);
    assert!(intents(s.tick(4.0 + SPRING_DELAY, &mut tx)).is_empty());
    s.drag_over(1, "/home/demo", DropHover::Accepting, None, 6.0);
    let _ = s.tick(6.0, &mut tx);
    assert!(intents(s.tick(7.0, &mut tx)).is_empty());
}

#[test]
fn the_copy_feedback_lasts_two_seconds() {
    let mut s = LocationBarState::new();
    let mut tx = Tx::default();
    s.show_copied(10.0);
    assert!(s.copied());
    let _ = s.tick(10.0, &mut tx);
    let _ = s.tick(10.0 + COPIED_FOR - 0.1, &mut tx);
    assert!(s.copied());
    assert!(s.tick(10.0 + COPIED_FOR, &mut tx).changed);
    assert!(!s.copied());
}

#[test]
fn a_leave_from_another_crumb_keeps_the_hover() {
    let mut s = LocationBarState::new();
    s.set_drop_hover(2, DropHover::Accepting);
    s.set_drop_hover(1, DropHover::Left);
    assert_eq!(s.drop_hover(), Some((2, DropHover::Accepting)));
    s.set_drop_hover(2, DropHover::Left);
    assert_eq!(s.drop_hover(), None);
}

// Review round 1, SF3: a binder that reports the same accepting hover again (X2's may, on every
// motion) must not restart the spring, as Strata's `schedule_spring_load_navigation` keeps it.
#[test]
fn a_repeated_hover_on_the_same_crumb_keeps_the_spring_s_start() {
    let mut s = LocationBarState::new();
    let mut tx = Tx::default();
    s.drag_over(
        1,
        "/home/demo",
        DropHover::Accepting,
        Some(SPRING_DELAY),
        1.0,
    );
    let _ = s.tick(1.0, &mut tx);
    s.drag_over(
        1,
        "/home/demo",
        DropHover::Accepting,
        Some(SPRING_DELAY),
        1.5,
    );
    let _ = s.tick(1.5, &mut tx);
    assert_eq!(
        intents(s.tick(1.0 + SPRING_DELAY, &mut tx)),
        [LocationIntent::Open {
            id: "/home/demo".into(),
            via: Via::Spring
        }]
    );
    // Another crumb starts afresh.
    s.drag_over(
        1,
        "/home/demo",
        DropHover::Accepting,
        Some(SPRING_DELAY),
        5.0,
    );
    let _ = s.tick(5.0, &mut tx);
    s.drag_over(0, "/", DropHover::Accepting, Some(SPRING_DELAY), 5.5);
    assert!(intents(s.tick(5.0 + SPRING_DELAY, &mut tx)).is_empty());
}

// A second press restarts the two seconds.
#[test]
fn a_second_copy_restarts_the_feedback() {
    let mut s = LocationBarState::new();
    let mut tx = Tx::default();
    s.show_copied(10.0);
    let _ = s.tick(10.0, &mut tx);
    let _ = s.tick(11.5, &mut tx);
    s.show_copied(11.5);
    let _ = s.tick(11.5, &mut tx);
    let _ = s.tick(12.5, &mut tx);
    assert!(s.copied(), "still showing 1 s after the second press");
    let _ = s.tick(11.5 + COPIED_FOR, &mut tx);
    assert!(!s.copied());
}

// Review round 1, nit: Ctrl+L while editing keeps the typed text and selects it, as Strata's
// `begin_location_edit` does.
#[test]
fn ctrl_l_while_editing_keeps_the_typed_text() {
    let mut s = LocationBarState::new();
    let _ = s.begin_edit("~/Projects");
    s.edit.replace_text("~/Mu");
    let p = s.begin_edit("~/Projects");
    assert_eq!(s.edit.text, "~/Mu");
    assert_eq!(s.edit.range(), (0, "~/Mu".len()));
    assert_eq!(query(&p).text, "~/Mu");
}

// Review round 1, nit: a click on a shown suggestion while keys wait for a listing takes that
// suggestion at once, as Strata's `row_activated` does; the waiting keys and listing are dropped.
#[test]
fn a_click_takes_its_suggestion_even_while_keys_wait() {
    let mut s = LocationBarState::new();
    let q = query(&s.begin_edit("~/D"));
    let _ = s.deliver(Completions::new(q.generation, two()));
    s.edit.replace_text("~/Do");
    let _ = s.edited();
    let (_, _) = s.command(completion::Command::Complete);
    let out = intents(s.choose(1));
    assert!(
        out.contains(&LocationIntent::Submit {
            text: "~/Documents/".into()
        }),
        "{out:?}"
    );
    assert!(!s.editing());
}

mod perf;

// SPDX-License-Identifier: MIT

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

struct CrumbHost {
    look: strata_kit::look::Look,
    trail: Trail,
    state: LocationBarState,
    open_in_new: bool,
    intents: Vec<LocationIntent>,
}

fn crumb_view(h: &CrumbHost, cx: &mut rune::Cx<CrumbHost>) -> rune::El {
    let mut layer = Vec::new();
    let bar = LocationBar::new(
        "loc",
        &h.look,
        &h.trail,
        &h.state,
        |h: &mut CrumbHost| &mut h.state,
        |h: &mut CrumbHost, intent, _| h.intents.push(intent),
    )
    .open_in_new(h.open_in_new)
    .build(cx, &mut layer);
    rune_kit::root(bar, layer)
}

fn mount_bar_with_animations(open_in_new: bool, animations: bool) -> rune::Runtime<CrumbHost> {
    let mut look = strata_kit::look::Look::default();
    look.animations = animations;
    let host = CrumbHost {
        look,
        trail: Trail::new(
            vec![
                Crumb::new("0", "~"),
                Crumb::new("1", "Projects"),
                Crumb::new("2", "strata"),
            ],
            "~/Projects/strata",
        ),
        state: LocationBarState::new(),
        open_in_new,
        intents: Vec::new(),
    };
    let mut rt = rune::Runtime::new(host, crumb_view, (500.0, 48.0), rune::Env::default());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn mount_bar(open_in_new: bool) -> rune::Runtime<CrumbHost> {
    mount_bar_with_animations(open_in_new, true)
}

fn element_by_key(rt: &rune::Runtime<CrumbHost>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("element keyed {key:?}"))
}

fn element_center(rt: &rune::Runtime<CrumbHost>, key: &str) -> (f32, f32) {
    let el = element_by_key(rt, key);
    (
        el.visible[0] + el.visible[2] / 2.0,
        el.visible[1] + el.visible[3] / 2.0,
    )
}

fn settle_frames(rt: &mut rune::Runtime<CrumbHost>, seconds: f64) {
    let steps = (seconds * 60.0).ceil() as usize;
    for _ in 0..steps.max(1) {
        rt.frame(1.0 / 60.0);
    }
}

#[test]
fn middle_press_on_ancestor_crumb_opens_tab_or_window_when_enabled() {
    use rune::{Mods, PhysicalInput, PointerButton};

    let mut rt = mount_bar(true);
    let crumb = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("loc-crumb-1"))
        .expect("ancestor crumb");
    let (x, y) = (
        crumb.visible[0] + crumb.visible[2] / 2.0,
        crumb.visible[1] + crumb.visible[3] / 2.0,
    );
    rt.pointer_move(x, y);
    rt.button_down(x, y, PointerButton::Middle);
    rt.button_up(x, y, PointerButton::Middle);
    rt.frame(1.0 / 60.0);
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        ctrl: true,
        ..Mods::default()
    }));
    rt.button_down(x, y, PointerButton::Middle);
    rt.button_up(x, y, PointerButton::Middle);
    rt.frame(1.0 / 60.0);
    assert_eq!(
        rt.state.intents,
        [
            LocationIntent::OpenInNewTab { id: "1".into() },
            LocationIntent::OpenInNewWindow { id: "1".into() },
        ]
    );

    let mut disabled = mount_bar(false);
    disabled.pointer_move(x, y);
    disabled.button_down(x, y, PointerButton::Middle);
    disabled.button_up(x, y, PointerButton::Middle);
    disabled.frame(1.0 / 60.0);
    assert!(disabled.state.intents.is_empty());
}

// Packet entry 108: a `Tx::element_at` callback inside `Runtime::frame` sees `tx.clock() == 0.0`
// even when the window's clock is seconds ahead; the spring stamps its start on the next tick.
#[test]
fn a_zero_clock_pick_hover_waits_the_full_delay_from_next_tick() {
    let mut s = LocationBarState::new();
    let mut tx = Tx::default();
    s.drag_over(
        1,
        "/home/demo",
        DropHover::Accepting,
        Some(SPRING_DELAY),
        0.0,
    );
    assert!(intents(s.tick(2.5, &mut tx)).is_empty());
    assert!(intents(s.tick(3.0, &mut tx)).is_empty());
    assert_eq!(
        intents(s.tick(2.5 + SPRING_DELAY, &mut tx)),
        [LocationIntent::Open {
            id: "/home/demo".into(),
            via: Via::Spring
        }]
    );
}

#[test]
fn copy_path_hides_at_rest_and_reveals_on_hover_without_flicker() {
    let mut rt = mount_bar(true);
    let copy = element_by_key(&rt, "loc-copy");
    assert_eq!(copy.opacity, 0.0, "hidden visually at rest");
    assert!(copy.focusable, "stays keyboard-focusable at rest");
    assert_eq!(copy.label.as_deref(), Some("Copy path"));
    assert!(
        rt.accessibility_json().to_string().contains("Copy path"),
        "stays present in the accessibility tree at rest"
    );

    // Hovering the current breadcrumb label reveals the Copy path button.
    let (cx, cy) = element_center(&rt, "loc-current");
    rt.pointer_move(cx, cy);
    settle_frames(&mut rt, 0.25);
    assert!(
        (element_by_key(&rt, "loc-copy").opacity - 1.0).abs() < 1e-3,
        "hovering current crumb reveals copy button"
    );

    // Moving directly from the current crumb onto the Copy path button keeps it at full opacity.
    let (bx, by) = element_center(&rt, "loc-copy");
    rt.pointer_move(bx, by);
    for step in 0..10 {
        rt.frame(1.0 / 60.0);
        let opacity = element_by_key(&rt, "loc-copy").opacity;
        assert!(
            (opacity - 1.0).abs() < 1e-3,
            "frame {step}: moving from current crumb to copy button must not flicker (opacity={opacity})"
        );
    }

    // Leaving the current breadcrumb hides the button again.
    let (ax, ay) = element_center(&rt, "loc-crumb-0");
    rt.pointer_move(ax, ay);
    settle_frames(&mut rt, 0.25);
    assert!(
        element_by_key(&rt, "loc-copy").opacity < 1e-3,
        "leaving current crumb hides copy button again"
    );
}

#[test]
fn copy_path_reveals_on_keyboard_focus_and_while_copied() {
    use rune::{Key, KeyEvent, Mods};

    let mut rt = mount_bar_with_animations(true, false);
    assert_eq!(element_by_key(&rt, "loc-copy").opacity, 0.0);

    // Tab until the Copy path button holds keyboard focus.
    let tab = KeyEvent {
        key: Key::Tab,
        mods: Mods::default(),
    };
    for _ in 0..8 {
        rt.key(tab.clone());
        rt.frame(1.0 / 60.0);
        if element_by_key(&rt, "loc-copy").focused {
            break;
        }
    }
    assert!(
        element_by_key(&rt, "loc-copy").focused,
        "Tab reaches Copy path"
    );
    assert_eq!(
        element_by_key(&rt, "loc-copy").opacity,
        1.0,
        "keyboard focus reveals Copy path"
    );

    // Moving focus off the button hides it again.
    rt.key(tab);
    rt.frame(1.0 / 60.0);
    assert!(!element_by_key(&rt, "loc-copy").focused);
    assert_eq!(
        element_by_key(&rt, "loc-copy").opacity,
        0.0,
        "blurring Copy path hides it again"
    );

    // Clicking Copy path keeps the confirmation checkmark visible even after the pointer leaves,
    // until COPIED_FOR elapses.
    let (bx, by) = element_center(&rt, "loc-copy");
    rt.pointer_move(bx, by);
    rt.frame(1.0 / 60.0);
    assert_eq!(
        element_by_key(&rt, "loc-copy").opacity,
        1.0,
        "hovering Copy path snaps open when animations are off"
    );
    rt.pointer_down(bx, by);
    rt.pointer_up(bx, by);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.intents, [LocationIntent::CopyPath]);

    let (ax, ay) = element_center(&rt, "loc-crumb-0");
    rt.pointer_move(ax, ay);
    rt.frame(1.0 / 60.0);
    let copied = element_by_key(&rt, "loc-copy");
    assert_eq!(copied.label.as_deref(), Some("Path copied"));
    assert_eq!(
        copied.opacity, 1.0,
        "Copy path stays visible while copied confirmation is active"
    );

    // Expire the 2-second feedback timer.
    rt.update(|h, tx| {
        let _ = h.state.tick(tx.clock() + COPIED_FOR + 0.1, tx);
    });
    rt.frame(1.0 / 60.0);
    let reset = element_by_key(&rt, "loc-copy");
    assert_eq!(reset.label.as_deref(), Some("Copy path"));
    assert_eq!(
        reset.opacity, 0.0,
        "Copy path hides again once copied feedback expires"
    );
}

mod perf;

use rune::{Cx, El, Env, Runtime, col, row};
use strata_kit::look::Look;
use strata_kit::shape::LookParams;

use super::range::Range;
use super::state::{SliderChange, SliderState};
use super::view::{Slider, Snap};

/// Strata's Frame density: 0–12 px, step 1, page 4, Compact 0 and Airy 4.
fn frame() -> Range {
    Range::new(0.0, 12.0, 1.0).page(4.0)
}

const KNOB: f32 = 16.0;
const WITHIN: f32 = 12.0;

/// A slider laid out 216 pt wide: 200 pt of travel, so 1 px of value is 16.7 pt.
fn laid_out(value: f64, range: Range) -> SliderState {
    let mut st = SliderState::new(value, range);
    st.set_width(200.0 + KNOB);
    st
}

#[test]
fn a_step_from_between_grid_points_lands_on_the_next_one_that_way() {
    let r = Range::new(32.0, 256.0, 16.0);
    assert_eq!(r.stepped(70.0, 1), 80.0);
    assert_eq!(r.stepped(70.0, -1), 64.0);
    assert_eq!(r.stepped(64.0, 1), 80.0, "on the grid, one whole step");
    assert_eq!(r.stepped(64.0, -2), 32.0);
}

#[test]
fn steps_stop_at_the_ends_and_reach_a_max_off_the_grid() {
    let r = Range::new(0.0, 10.0, 3.0);
    assert_eq!(r.stepped(9.0, 1), 10.0, "the last step reaches the max");
    assert_eq!(r.stepped(10.0, -1), 9.0, "and back down to the grid");
    assert_eq!(r.stepped(10.0, 5), 10.0);
    assert_eq!(r.stepped(0.0, -1), 0.0);
}

#[test]
fn tenths_step_without_float_noise() {
    let r = Range::new(0.0, 1.0, 0.1);
    let v = (0..3).fold(0.0, |v, _| r.stepped(v, 1));
    assert_eq!(v, 0.3);
    assert_eq!(r.format(v), "0.3");
}

#[test]
fn a_page_is_whole_steps() {
    assert_eq!(frame().page_steps(), 4);
    assert_eq!(
        Range::new(0.0, 200.0, 10.0).page_steps(),
        10,
        "ten steps by default"
    );
    assert_eq!(Range::new(0.0, 1.0, 0.1).page(0.25).page_steps(), 3);
}

#[test]
fn a_drag_near_a_snap_point_lands_on_it() {
    let mut st = laid_out(0.0, frame());
    let snaps = [0.0, 4.0];
    // Grab the knob at its centre (x 8), then drag to 3.6 px: 8 pt short of Airy's tick.
    assert_eq!(
        st.press(8.0, KNOB, &snaps, WITHIN, false),
        None,
        "grabbing the knob moves nothing"
    );
    let near = st.x_of(3.6, KNOB);
    assert_eq!(
        st.drag(near, KNOB, &snaps, WITHIN, false),
        Some(SliderChange::Live)
    );
    assert_eq!(st.value(), 4.0);
    // 2.6 px is 23 pt from Airy and 43 pt from Compact: the grid's nearest point.
    st.drag(st.x_of(2.6, KNOB), KNOB, &snaps, WITHIN, false);
    assert_eq!(st.value(), 3.0);
    assert_eq!(st.release(false), Some(SliderChange::Commit));
    assert!(!st.dragging());
}

#[test]
fn a_press_on_the_track_jumps_and_one_on_the_knob_keeps_its_grab() {
    let mut st = laid_out(6.0, frame());
    let centre = st.x_of(6.0, KNOB);
    // Pressed 7 pt right of the knob's centre, then moved 40 pt: the knob follows the
    // pointer from where it was grabbed, so it moves 40 pt (2.4 px, to 8), not 47 (to 9).
    assert_eq!(st.press(centre + 7.0, KNOB, &[], WITHIN, false), None);
    st.drag(centre + 47.0, KNOB, &[], WITHIN, false);
    assert_eq!(st.value(), 8.0);
    st.release(false);
    // Off the knob, the press itself moves it there.
    assert_eq!(
        st.press(st.x_of(2.0, KNOB), KNOB, &[], WITHIN, false),
        Some(SliderChange::Live)
    );
    assert_eq!(st.value(), 2.0);
}

#[test]
fn a_cancelled_drag_puts_the_value_back() {
    let mut st = laid_out(5.0, frame());
    st.press(st.x_of(10.0, KNOB), KNOB, &[], WITHIN, false);
    assert_eq!(st.value(), 10.0);
    assert_eq!(st.release(true), Some(SliderChange::Cancel));
    assert_eq!(st.value(), 5.0);
    // A drag that never moved the value reports nothing either way.
    let c = st.x_of(5.0, KNOB);
    st.press(c, KNOB, &[], WITHIN, false);
    assert_eq!(st.release(true), None);
    st.press(c, KNOB, &[], WITHIN, false);
    assert_eq!(st.release(false), None);
}

#[test]
fn a_free_drag_keeps_values_between_grid_points() {
    let mut st = laid_out(0.0, Range::new(0.0, 1.0, 0.1));
    st.press(st.x_of(0.43, KNOB), KNOB, &[], WITHIN, true);
    assert_eq!(st.value(), 0.4, "kept to the step's decimals");
    let mut st = laid_out(0.0, Range::new(0.0, 1.0, 0.1).decimals(3));
    st.press(st.x_of(0.43, KNOB), KNOB, &[], WITHIN, true);
    assert_eq!(st.value(), 0.43);
    let mut st = laid_out(0.0, Range::new(0.0, 1.0, 0.1).decimals(3));
    st.press(st.x_of(0.43, KNOB), KNOB, &[], WITHIN, false);
    assert_eq!(st.value(), 0.4, "a snapped drag rounds to the grid");
}

#[test]
fn precise_wheel_travel_adds_up_to_whole_notches() {
    let mut st = SliderState::new(64.0, Range::new(32.0, 256.0, 16.0));
    assert!(!st.wheel(30.0, true, 1.0), "30 pt is short of a notch");
    assert!(st.wheel(30.0, true, 1.0));
    assert_eq!(
        st.value(),
        65.0,
        "a notch moves the wheel's amount, off the grid"
    );
    assert!(
        st.wheel(-1.0, false, 1.0),
        "a mouse notch is one whatever its size"
    );
    assert_eq!(st.value(), 64.0);
}

#[test]
fn a_sideways_wheel_moves_nothing() {
    let mut st = SliderState::new(64.0, Range::new(32.0, 256.0, 16.0));
    assert!(!st.wheel(0.0, false, 1.0), "a notch with no vertical part");
    assert!(!st.wheel(0.0, true, 1.0));
    assert_eq!(st.value(), 64.0);
}

#[test]
fn home_end_and_set_report_only_a_move() {
    let mut st = SliderState::new(0.0, frame());
    assert!(!st.jump_to_end(false));
    assert!(st.jump_to_end(true));
    assert_eq!(st.value(), 12.0);
    assert!(st.set(4.0));
    assert!(!st.set(4.0));
}

struct Host {
    slider: SliderState,
    look: Look,
}

/// Long snap labels, as Russian's are, in a column sized to its content, as Strata's
/// Appearance menu is.
fn shrink_wrapped(s: &Host, _: &mut Cx<Host>) -> El {
    let slider = Slider::new("s", |h: &Host| &h.slider, |h: &mut Host| &mut h.slider)
        .snaps(vec![
            Snap::labelled(0.0, "Компактная плотность"),
            Snap::labelled(4.0, "Просторная плотность"),
        ])
        .build(s, &s.look);
    row().items_start().child(col().child(slider))
}

#[test]
fn a_content_sized_menu_reserves_the_snap_labels_width() {
    let mut look = Look::fallback(false);
    look.apply_params(&LookParams {
        text_size: 24,
        ..LookParams::default()
    });
    let gap = look.space.md;
    let host = Host {
        slider: SliderState::new(0.0, frame()),
        look,
    };
    // Rune's own font: a text system built here would move the fonts the crate's other tests
    // measure with (docs/testing.md, R-K6-1).
    // The label's width comes from the same shared measure, so the claim holds in any font.
    let mut rt = Runtime::new(host, shrink_wrapped, (900.0, 400.0), Env::new(false, 1.0));
    rt.frame(0.0);
    for _ in 0..5 {
        rt.frame(1.0 / 60.0);
    }
    let rect = |key: &str| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no {key}"))
            .rect
    };
    let (slider, a, b) = (rect("s"), rect("s-snap-0"), rect("s-snap-1"));
    let label = rune::text::advance("Просторная плотность", look_small(&rt));
    assert!(b[2] >= label, "the label keeps its width: {b:?}, {label}");
    assert!(a[0] >= slider[0], "{a:?} inside {slider:?}");
    assert!(
        a[0] + a[2] + gap <= b[0] + 0.5,
        "{a:?} and {b:?} {gap} apart"
    );
    assert!(
        b[0] + b[2] <= slider[0] + slider[2] + 0.5,
        "{b:?} inside {slider:?}"
    );
}

fn look_small(rt: &Runtime<Host>) -> rune::TextStyle {
    rt.state.look.kit.ty.small
}

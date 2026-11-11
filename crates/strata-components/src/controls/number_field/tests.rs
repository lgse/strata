use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, Tx, col, el};
use strata_kit::look::Look;

use super::state::NumberFieldState;
use super::view::{NumberField, key_step};
use crate::controls::slider::Range;

/// Strata's Corner roundness: 0–200 % in steps of 10.
fn roundness(v: f64) -> NumberFieldState {
    NumberFieldState::new(v, Range::new(0.0, 200.0, 10.0)).suffix("%")
}

/// Strata's Thumbnail workers: 1–16.
fn workers(v: f64) -> NumberFieldState {
    NumberFieldState::new(v, Range::new(1.0, 16.0, 1.0))
}

fn typed(st: &mut NumberFieldState, text: &str) {
    st.edit_mut().replace_text(text);
    st.edited();
}

fn press(st: &mut NumberFieldState, key: Key, mods: Mods) -> Option<Option<f64>> {
    key_step(st, &KeyEvent { key, mods })
}

#[test]
fn a_number_reads_with_or_without_its_unit() {
    let mut st = roundness(60.0);
    for (text, value) in [
        ("80", 80.0),
        (" 80 % ", 80.0),
        ("80%", 80.0),
        ("+70", 70.0),
        ("7", 7.0),
    ] {
        typed(&mut st, text);
        assert!(!st.invalid(), "{text:?} is a value");
        assert_eq!(st.commit_text(), Some(value), "{text:?}");
        st.set_value(60.0);
    }
}

#[test]
fn a_spaced_unit_reads_typed_with_or_without_its_space() {
    let mut st = NumberFieldState::new(13.0, Range::new(8.0, 48.0, 1.0)).suffix(" px");
    for text in ["14px", "14 px", "14"] {
        typed(&mut st, text);
        assert_eq!(st.parse(), Some(14.0), "{text:?}");
    }
    assert_eq!(st.value_text(), "13 px");
}

#[test]
fn text_out_of_range_or_not_a_number_reverts() {
    let mut st = workers(4.0);
    for text in ["40", "0", "-3", "abc", "", "1e1", "4.5", "4.", "inf", "%"] {
        typed(&mut st, text);
        assert!(st.invalid(), "{text:?} isn't a value");
        assert_eq!(st.commit_text(), None, "{text:?}");
        assert_eq!(st.edit().text, "4", "{text:?} reverts to the value");
        assert!(!st.invalid());
    }
    assert_eq!(st.value(), 4.0);
}

#[test]
fn signs_and_decimal_points_follow_the_range() {
    let mut st = NumberFieldState::new(0.0, Range::new(-5.0, 5.0, 0.5));
    typed(&mut st, "-2.5");
    assert_eq!(st.commit_text(), Some(-2.5));
    typed(&mut st, "1.25");
    assert_eq!(
        st.commit_text(),
        Some(1.3),
        "rounded to the step's decimals"
    );
    assert_eq!(st.edit().text, "1.3");
    typed(&mut st, "1.2.5");
    assert_eq!(st.commit_text(), None);
}

#[test]
fn a_step_commits_the_typed_text_first() {
    let mut st = workers(4.0);
    typed(&mut st, "9");
    assert_eq!(st.step(1), Some(10.0));
    typed(&mut st, "junk");
    assert_eq!(st.step(-1), Some(9.0), "junk reverts, then the step");
    assert_eq!(st.edit().text, "9");
}

#[test]
fn steps_hold_at_the_ends() {
    let mut st = workers(16.0);
    assert_eq!(st.step(1), None, "At 16, + leaves 16");
    assert_eq!(st.value(), 16.0);
    let mut st = workers(1.0);
    assert_eq!(st.step(-1), None);
}

#[test]
fn the_keys_step_page_and_reach_the_limits() {
    let none = Mods::default();
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    };
    let mut st = roundness(60.0);
    assert_eq!(press(&mut st, Key::Up, none), Some(Some(70.0)));
    assert_eq!(press(&mut st, Key::Down, none), Some(Some(60.0)));
    assert_eq!(
        press(&mut st, Key::PageUp, none),
        Some(Some(160.0)),
        "a page is ten steps"
    );
    assert_eq!(press(&mut st, Key::PageDown, none), Some(Some(60.0)));
    assert_eq!(press(&mut st, Key::End, ctrl), Some(Some(200.0)));
    assert_eq!(press(&mut st, Key::Home, ctrl), Some(Some(0.0)));
    assert_eq!(press(&mut st, Key::PageUp, ctrl), Some(Some(200.0)));
    assert_eq!(press(&mut st, Key::PageDown, ctrl), Some(Some(0.0)));
    assert_eq!(
        press(&mut st, Key::Home, none),
        None,
        "plain Home moves the caret"
    );
    assert_eq!(press(&mut st, Key::Left, none), None);
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    assert_eq!(press(&mut st, Key::Up, shift), None, "Shift+Up selects");
    typed(&mut st, "90");
    assert_eq!(
        press(&mut st, Key::Enter, none),
        Some(Some(90.0)),
        "Return commits"
    );
    typed(&mut st, "900");
    assert_eq!(
        press(&mut st, Key::Enter, none),
        Some(None),
        "and takes Return when it reverts"
    );
    assert_eq!(st.edit().text, "90");
}

#[test]
fn losing_the_keyboard_commits_but_not_to_the_menu() {
    let mut st = workers(4.0);
    assert_eq!(st.blurred(), None, "never focused: nothing to commit");
    st.edit_mut().focused = true;
    typed(&mut st, "6");
    assert_eq!(st.blurred(), None, "still focused");
    st.hold(true);
    st.edit_mut().focused = false;
    assert_eq!(st.blurred(), None, "the field's menu took the keyboard");
    st.hold(false);
    assert_eq!(st.blurred(), Some(Some(6.0)));
    assert_eq!(st.blurred(), None, "once");
    st.edit_mut().focused = true;
    st.blurred();
    st.edit_mut().focused = false;
    assert_eq!(
        st.blurred(),
        None,
        "a blur with nothing typed changes nothing"
    );
    st.edit_mut().focused = true;
    st.blurred();
    typed(&mut st, "99");
    st.edit_mut().focused = false;
    assert_eq!(
        st.blurred(),
        Some(None),
        "reverted: the text changed, the value didn't"
    );
    assert_eq!(st.edit().text, "6");
}

#[test]
fn the_wheel_steps_a_notch_or_forty_points() {
    let mut st = roundness(60.0);
    assert_eq!(
        st.wheel(1.0, false),
        Some(70.0),
        "the wheel rolled up raises it"
    );
    assert_eq!(st.wheel(-3.0, false), Some(60.0));
    assert_eq!(st.wheel(25.0, true), None);
    assert_eq!(st.wheel(25.0, true), Some(70.0));
}

#[test]
fn a_sideways_wheel_neither_steps_nor_commits_the_text() {
    let mut st = workers(4.0);
    typed(&mut st, "9");
    assert_eq!(st.wheel(0.0, false), None, "a notch with no vertical part");
    assert_eq!(st.wheel(0.0, true), None);
    assert_eq!((st.value(), st.edit().text.as_str()), (4.0, "9"));
}

#[test]
fn the_value_text_has_its_unit() {
    let mut st = roundness(60.0);
    assert_eq!(st.value_text(), "60%");
    st.set_value(400.0);
    assert_eq!((st.value(), st.edit().text.as_str()), (200.0, "200"));
}

struct Host {
    field: NumberFieldState,
    look: Look,
    heard: Vec<f64>,
}

/// A field whose host opens no menu for it (it has none to show now), and a button to move
/// the keyboard to.
fn field() -> NumberField<Host> {
    NumberField::new("n", |h: &Host| &h.field, |h: &mut Host| &mut h.field)
        .on_change(|h: &mut Host, v, _| h.heard.push(v))
        .on_menu(|_: &mut Host, _, _: &mut Tx| false)
}

fn view(h: &Host, _: &mut Cx<Host>) -> El {
    col().child(field().build(h, &h.look)).child(
        el().key("other")
            .size(40.0, 20.0)
            .focusable(true)
            .on_press(|_: &mut Host| {}),
    )
}

fn poll(h: &mut Host, _: f64, tx: &mut Tx) -> bool {
    field().poll(h, tx)
}

#[test]
fn a_menu_press_that_opens_no_menu_keeps_the_blur_commit() {
    let host = Host {
        field: workers(4.0),
        look: Look::fallback(false),
        heard: Vec::new(),
    };
    // Rune's own font: a text system built here would move the fonts the crate's other tests
    // measure with (docs/testing.md, R-K6-1).
    let mut rt = Runtime::new(host, view, (400.0, 200.0), Env::new(false, 1.0));
    rt.poll = Some(poll);
    let frames = |rt: &mut Runtime<Host>| {
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
    };
    rt.frame(0.0);
    frames(&mut rt);
    let r = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("n-input"))
        .expect("the field")
        .rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt);
    assert!(rt.state.field.edit().focused);
    for key in [Key::End, Key::Backspace, Key::Text("9".into())] {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        frames(&mut rt);
    }
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    frames(&mut rt);
    rt.key(KeyEvent {
        key: Key::Tab,
        mods: Mods::default(),
    });
    frames(&mut rt);
    assert!(!rt.state.field.edit().focused, "the keyboard went on");
    assert_eq!(rt.state.heard, [9.0], "and the typed count committed");
}

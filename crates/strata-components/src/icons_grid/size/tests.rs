//! The size keys and the wheel's steps.

use rune::{Chord, ChordKey, ChordMod, GesturePhase, Key, KeyEvent, Mods, WheelEvent};

use super::*;

/// Ctrl and Alt, as a window reports them off macOS (the primary modifier set with Ctrl).
const CTRL_ALT: Mods = Mods {
    shift: false,
    cmd: true,
    alt: true,
    ctrl: true,
    logo: false,
};

fn listing(keys: &SizeKeys, key: Key, mods: Mods) -> Option<SizeCommand> {
    let event = KeyEvent { key, mods };
    keys.listing_command(&KeyInput::new(&event))
}

#[test]
fn ctrl_alt_plus_equal_and_minus_step_and_ctrl_alt_0_resets() {
    let keys = SizeKeys::default_map();
    for key in [Key::Command('='), Key::Text("=".into())] {
        assert_eq!(listing(&keys, key, CTRL_ALT), Some(SizeCommand::Step(1)));
    }
    let shifted = Mods {
        shift: true,
        ..CTRL_ALT
    };
    assert_eq!(
        listing(&keys, Key::Text("+".into()), shifted),
        Some(SizeCommand::Step(1)),
        "+ with the Shift that types it"
    );
    assert_eq!(
        listing(&keys, Key::Command('-'), CTRL_ALT),
        Some(SizeCommand::Step(-1))
    );
    assert_eq!(
        listing(&keys, Key::Command('0'), CTRL_ALT),
        Some(SizeCommand::Reset)
    );
    let ctrl = Mods {
        alt: false,
        ..CTRL_ALT
    };
    assert_eq!(
        listing(&keys, Key::Command('='), ctrl),
        None,
        "Ctrl+= is the text size, not the thumbnails"
    );
}

#[test]
fn plain_plus_equal_and_minus_step_only_in_tenxer() {
    let off = SizeKeys::default_map();
    let on = SizeKeys::default_map().with_tenxer(true);
    for (key, step) in [("+", 1), ("=", 1), ("-", -1)] {
        let mods = Mods {
            shift: key == "+",
            ..Mods::default()
        };
        assert_eq!(listing(&off, Key::Text(key.into()), mods), None);
        assert_eq!(
            listing(&on, Key::Text(key.into()), mods),
            Some(SizeCommand::Step(step))
        );
    }
    assert_eq!(
        listing(&on, Key::Command('-'), CTRL_ALT),
        Some(SizeCommand::Step(-1)),
        "the Ctrl+Alt keys still work in 10xer"
    );
}

#[test]
fn a_rebound_default_chord_applies_in_tenxer_too() {
    let mut keys = SizeKeys::default_map().with_tenxer(true);
    let ctrl_shift_up = Chord::new(ChordKey::Up)
        .with(ChordMod::Ctrl)
        .with(ChordMod::Shift);
    keys.bind(SizeCommand::Step(1), vec![ctrl_shift_up]);
    let mods = Mods {
        shift: true,
        cmd: true,
        ctrl: true,
        ..Mods::default()
    };
    assert_eq!(listing(&keys, Key::Up, mods), Some(SizeCommand::Step(1)));
    assert_eq!(
        listing(&keys, Key::Command('='), CTRL_ALT),
        None,
        "the old chord is gone"
    );
    assert_eq!(
        listing(
            &keys,
            Key::Text("+".into()),
            Mods {
                shift: true,
                ..Mods::default()
            }
        ),
        Some(SizeCommand::Step(1)),
        "10xer's plain keys stay"
    );
}

#[test]
fn an_intent_carries_its_command_back() {
    let intent = Intent::Other(Rc::new(SizeCommand::Reset));
    assert_eq!(size_command(&intent), Some(SizeCommand::Reset));
    assert_eq!(
        size_command(&Intent::Edge(crate::contract::Direction::Up)),
        None
    );
}

fn wheel(dy: f32, precise: bool) -> WheelEvent {
    WheelEvent::new(0.0, dy)
        .precise(precise)
        .phase(GesturePhase::Moved)
        .mods(Mods {
            shift: true,
            cmd: true,
            ctrl: true,
            ..Mods::default()
        })
}

#[test]
fn only_ctrl_shift_wheels_are_size_wheels() {
    assert!(WheelSteps::claims(&wheel(1.0, false)));
    let mut alt = wheel(1.0, false);
    alt.mods.alt = true;
    assert!(!WheelSteps::claims(&alt));
    let mut ctrl = wheel(1.0, false);
    ctrl.mods.shift = false;
    assert!(!WheelSteps::claims(&ctrl), "Ctrl+wheel is the text size");
}

#[test]
fn a_notch_is_one_step_and_precise_deltas_add_up_to_steps() {
    let mut w = WheelSteps::default();
    assert_eq!(w.steps(&wheel(40.0, false)), 1, "away is larger");
    assert_eq!(w.steps(&wheel(-120.0, false)), -1, "one notch, however far");
    assert_eq!(w.steps(&wheel(4.0, true)), 0);
    assert_eq!(w.steps(&wheel(4.0, true)), 0);
    assert_eq!(
        w.steps(&wheel(4.0, true)),
        1,
        "12 points: a step, 2 carried"
    );
    assert_eq!(
        w.steps(&wheel(-9.0, true)),
        0,
        "turning back drops what was carried"
    );
    assert_eq!(w.steps(&wheel(-25.0, true)), -3);
    let sideways = WheelEvent::new(30.0, 0.0)
        .precise(true)
        .mods(wheel(0.0, true).mods);
    assert_eq!(
        w.steps(&sideways),
        3,
        "Shift+wheel turned sideways counts its dx"
    );
}

use rune::{Chord, Key, KeyEvent, Modifier, Mods, Tx};

use crate::filter_bar::chord::{ChordMap, ChordOption, ChordOutcome, ChordState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Go {
    First,
    Last,
    Home,
}

fn go() -> ChordMap<Go> {
    let c = |s: &str| Chord::parse(s).expect("a chord");
    ChordMap {
        mark: "g-".into(),
        name: "Keys after g".into(),
        options: vec![
            ChordOption::new("g", "First item", vec![(c("g"), Go::First)]),
            ChordOption::new("G", "Last item", vec![(c("shift+g"), Go::Last)]),
            ChordOption::new("h", "Home", vec![(c("h"), Go::Home)]),
            ChordOption::mention("Shift", "Reverse"),
        ],
    }
}

fn typed(c: &str, shift: bool) -> KeyEvent {
    KeyEvent {
        key: Key::Text(c.into()),
        mods: Mods {
            shift,
            ..Mods::default()
        },
    }
}

// integration/10xer-mode: "The panel lists each valid second key… and closes when the chord
// completes or Esc cancels it"; places: "any other unknown second key cancels it with
// `Unknown chord`". A modifier alone keeps it armed, and the case of a letter counts.
#[test]
fn the_next_key_completes_cancels_or_passes() {
    let tx = Tx::default();
    let mut st = ChordState::new();
    assert_eq!(st.key(&typed("g", false), &tx), None, "nothing armed");

    st.arm(go());
    let shift = KeyEvent {
        key: Key::Modifier {
            modifier: Modifier::ShiftLeft,
            pressed: true,
        },
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
    };
    assert_eq!(st.key(&shift, &tx), Some(ChordOutcome::Pass));
    assert!(st.is_armed());
    assert_eq!(
        st.key(&typed("G", true), &tx),
        Some(ChordOutcome::Completed(Go::Last))
    );
    assert!(!st.is_armed());

    st.arm(go());
    assert_eq!(
        st.key(&typed("g", false), &tx),
        Some(ChordOutcome::Completed(Go::First))
    );

    st.arm(go());
    let esc = KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    };
    assert_eq!(st.key(&esc, &tx), Some(ChordOutcome::Cancelled));

    st.arm(go());
    assert_eq!(st.key(&typed("q", false), &tx), Some(ChordOutcome::Unknown));
    assert!(!st.is_armed());
}

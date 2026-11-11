// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::{
    super::{Failure, Rect, Remembered, Scope, States},
    fake::{Fake, Node, WINDOW, drawn},
    predicate,
};

fn check(fake: &Fake, kept: &Remembered<Fake>, json: Value) -> Result<Result<(), Value>, String> {
    let scope = Scope {
        backend: fake,
        window: WINDOW,
        kept,
    };
    scope
        .check(&predicate(json))
        .map(|held| held.map_err(|Failure { actual, .. }| actual))
}

fn toolbar(fake: &mut Fake) -> usize {
    let bar = fake.add(
        WINDOW,
        drawn("tool bar", "Actions", [0.0, 0.0, 800.0, 40.0]),
    );
    fake.add(
        bar,
        Node {
            states: States {
                sensitive: true,
                pressed: true,
                ..States::default()
            },
            ..drawn("toggle button", "Show hidden files", [0.0, 0.0, 40.0, 40.0])
        },
    )
}

#[test]
fn live_predicates_read_states_presence_and_names_through_the_backend() {
    let mut fake = Fake::new();
    toolbar(&mut fake);
    let kept = Remembered::default();
    let button = json!({ "role": "toggle button", "name": "Show hidden files" });

    for holding in [
        json!({ "state": { "target": button, "pressed": true, "sensitive": true } }),
        json!({ "present": button }),
        json!({ "absent": { "role": "button", "name": "Close" } }),
        json!({ "not": { "present": { "dialog": true } } }),
        json!({ "any": [{ "present": { "dialog": true } }, { "present": button }] }),
        json!({ "inside": { "target": button, "of": { "role": "tool bar", "name": "Actions" } } }),
        json!({ "windows": { "title": "Strata", "count": 1 } }),
    ] {
        assert_eq!(
            check(&fake, &kept, holding.clone()),
            Ok(Ok(())),
            "{holding}"
        );
    }
    assert_eq!(
        check(&fake, &kept, json!({ "absent": button })),
        Ok(Err(json!([0, 0, 40, 40]))),
        "a present target reports where it is"
    );
    assert_eq!(
        check(
            &fake,
            &kept,
            json!({ "state": { "target": { "role": "button", "name": "Close" }, "pressed": true } })
        ),
        Ok(Err(json!("not on screen")))
    );
}

#[test]
fn a_moved_check_compares_against_the_box_an_earlier_step_kept() {
    let mut fake = Fake::new();
    toolbar(&mut fake);
    let kept = Remembered::default();
    let moved = json!({ "moved": {
        "target": { "role": "toggle button", "name": "Show hidden files" },
        "since": "before",
        "direction": "left"
    } });

    assert_eq!(
        check(&fake, &kept, moved.clone()),
        Err("moved needs an earlier settle or remember on before".to_owned()),
        "nothing kept is an error a wait cannot fix"
    );
    kept.boxes.borrow_mut().insert(
        "before".to_owned(),
        Rect {
            x: 120.0,
            y: 0.0,
            width: 40.0,
            height: 40.0,
        },
    );
    assert_eq!(check(&fake, &kept, moved), Ok(Ok(())));
}

#[test]
fn a_predicate_the_toolkit_cannot_answer_says_so() {
    let mut fake = Fake::new();
    toolbar(&mut fake);
    let kept = Remembered::default();
    let pixel = json!({ "pixel": {
        "target": { "role": "toggle button", "name": "Show hidden files" },
        "rgb": [0, 0, 0]
    } });
    assert_eq!(
        check(&fake, &kept, pixel.clone()),
        Err("pixel is not supported on this target".to_owned())
    );
    assert_eq!(
        check(&fake, &kept, json!({ "not": pixel })),
        Err("pixel is not supported on this target".to_owned()),
        "a negation does not turn an unsupported predicate into one that holds"
    );
}

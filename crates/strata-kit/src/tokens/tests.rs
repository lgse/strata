use std::collections::BTreeSet;

use rune::Color;

use super::*;

fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color { r, g, b, a: 1.0 }
}

/// A value of the token's kind that differs from `current`, inside its range.
fn another(token: &Token, current: Value) -> Value {
    match current {
        Value::Color(c) => Value::Color(if c == rgb(0.25, 0.5, 0.75) {
            rgb(0.75, 0.5, 0.25)
        } else {
            rgb(0.25, 0.5, 0.75)
        }),
        Value::Length(x) => Value::Length(x + 3.0),
        Value::Number(x) => {
            let (min, max) = token.range().expect("numbers have a range");
            Value::Number(if x + 1.0 <= max { x + 1.0 } else { min })
        }
        Value::Flag(b) => Value::Flag(!b),
    }
}

#[test]
fn every_registered_name_round_trips() {
    let mut look = Look::fallback(true);
    for token in all() {
        let name = token.name();
        let was = get(&look, name).expect("a registered name reads");
        assert_eq!(was.kind(), token.kind(), "{name} reads as its own kind");
        set(&mut look, name, was).unwrap_or_else(|e| panic!("{name} takes its own value: {e}"));
        assert_eq!(get(&look, name), Ok(was), "{name} keeps its value");

        let next = another(token, was);
        set(&mut look, name, next).unwrap_or_else(|e| panic!("{name} takes {next:?}: {e}"));
        assert_eq!(get(&look, name), Ok(next), "{name} reads back what was set");
    }
}

fn is_slot(name: &str) -> bool {
    slots::TOKENS
        .iter()
        .any(|(_, tokens)| tokens.iter().any(|t| t.name() == name))
}

/// Setting a core token may move slot tokens, which inherit from it, but never another core
/// token. Setting a slot token moves nothing else.
#[test]
fn setting_one_token_moves_only_what_inherits_from_it() {
    let fresh = Look::fallback(false);
    for token in all() {
        let mut look = Look::fallback(false);
        let next = another(token, token.get(&look));
        token.set(&mut look, next).expect("in range");
        let moved: Vec<_> = all()
            .filter(|t| t.name() != token.name() && t.get(&look) != t.get(&fresh))
            .map(|t| t.name())
            .collect();
        let allowed = |n: &str| !is_slot(token.name()) && is_slot(n);
        assert!(
            moved.iter().all(|n| allowed(n)),
            "setting {} also moved {moved:?}",
            token.name()
        );
    }
}

#[test]
fn an_unknown_name_is_an_error() {
    let mut look = Look::fallback(false);
    for name in ["color.nope", "color", "list.row", "", "Color.bg_base"] {
        assert_eq!(get(&look, name), Err(TokenError::Unknown(name.to_owned())));
        assert_eq!(
            set(&mut look, name, Value::Length(1.0)),
            Err(TokenError::Unknown(name.to_owned()))
        );
    }
}

#[test]
fn a_rejected_value_leaves_the_look_unchanged() {
    let mut look = Look::fallback(false);
    let before = get(&look, "row.list");
    let rejected = [
        ("row.list", Value::Color(rgb(1.0, 0.0, 0.0))),
        ("row.list", Value::Number(30.0)),
        ("row.list", Value::Length(-1.0)),
        ("row.list", Value::Length(f32::NAN)),
        ("row.list", Value::Length(f32::INFINITY)),
    ];
    for (name, value) in rejected {
        assert!(
            set(&mut look, name, value).is_err(),
            "{name} took {value:?}"
        );
    }
    assert_eq!(get(&look, "row.list"), before);

    assert_eq!(
        set(&mut look, "type.ui.weight", Value::Number(1200.0)),
        Err(TokenError::Range {
            name: "type.ui.weight",
            min: 1.0,
            max: 1000.0
        })
    );
    assert_eq!(
        set(&mut look, "glow.on", Value::Number(1.0)),
        Err(TokenError::Kind {
            name: "glow.on",
            expected: Kind::Flag,
            got: Kind::Number
        })
    );
}

#[test]
fn names_are_unique_and_dotted_snake_case() {
    let mut seen = BTreeSet::new();
    for token in all() {
        let name = token.name();
        assert!(seen.insert(name), "{name} is registered twice");
        let segments: Vec<_> = name.split('.').collect();
        assert!(segments.len() >= 2, "{name} has fewer than two segments");
        for s in segments {
            assert!(
                !s.is_empty()
                    && s.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{name}: segment {s:?} isn't snake_case"
            );
        }
    }
}

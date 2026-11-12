// SPDX-License-Identifier: MIT

use crate::keys::{
    Binding, Command, KeyPress, KeyPressParts, Map, Matcher, Modifiers, SHORTCUT_MODIFIERS, Table,
    Typed,
    bindings::{COPY, HISTORY_BACK, REFRESH, RENAME},
    install,
};

const SHIFT: Modifiers = Modifiers::SHIFT_MASK;

fn every_binding_fires(_: Map, _: Command, _: Binding, _: bool) -> bool {
    false
}

/// A press of `binding`'s key with `held`.
fn pressed(binding: Binding, held: Modifiers) -> KeyPress {
    KeyPress::from_parts(KeyPressParts {
        key: binding.key(),
        modifiers: held,
        held,
        typed: Typed::Key(binding.key()),
        digit: None,
        fallback: false,
    })
}

/// What a matcher says it reads is what it matches: its command's binding in its map, with
/// the modifiers it ignores held beside the binding's own, and no other.
#[test]
fn a_matcher_matches_its_commands_binding_in_its_map_with_what_it_ignores_held() {
    // The first install decides for the process; another test's is the same backend.
    install(&Table, every_binding_fires);
    let cases = [
        ("copy, as the active map binds it", Matcher::from(COPY)),
        ("rename in the 10xer table", RENAME.in_map(Map::Tenxer)),
        (
            "history back with Shift let through",
            HISTORY_BACK.in_map(Map::Default).ignoring(SHIFT),
        ),
        (
            "refresh with anything held",
            REFRESH.ignoring(SHORTCUT_MODIFIERS),
        ),
    ];
    for (case, matcher) in cases {
        let map = matcher.map().unwrap_or_else(Map::active);
        let binding = matcher.command().defaults(map)[0];
        let own = binding.modifiers();

        let with_ignored = pressed(binding, own | matcher.ignored());
        assert!(with_ignored.is(matcher), "{case}: {with_ignored:?}");

        let beyond = SHORTCUT_MODIFIERS.difference(own | matcher.ignored());
        if !beyond.is_empty() {
            let with_more = pressed(binding, own | matcher.ignored() | beyond);
            assert!(!with_more.is(matcher), "{case}: {with_more:?}");
        }
    }
    // A matcher of one map does not read the other map's key: **r** renames in 10xer only.
    let r = RENAME.defaults(Map::Tenxer)[0];
    assert!(pressed(r, r.modifiers()).is(RENAME.in_map(Map::Tenxer)));
    assert!(!pressed(r, r.modifiers()).is(RENAME.in_map(Map::Default)));
}

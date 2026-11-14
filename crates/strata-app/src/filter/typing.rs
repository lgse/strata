// SPDX-License-Identifier: MIT

//! Type-to-search: what a key typed in the listing opens the filter with,
//! `type_to_search_query` in `src/ui/window/key_routing.rs`.

use strata_desktop::keys::{
    KeyPress, Modifiers,
    bindings::{FILTER_SLASH, SPACE},
};

/// What a typed key opens the filter on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Query {
    /// `/`: the filter, empty.
    Empty,
    /// A character: the filter, with it typed.
    Character(char),
}

/// What `press` opens the filter on, if anything. Space belongs to the quick preview, and a
/// letter that reads as `/` only through the layout fallback is typed.
pub(super) fn query(press: &KeyPress) -> Option<Query> {
    if press.is(SPACE.ignoring(Modifiers::SHIFT_MASK)) {
        return None;
    }
    if press.is(FILTER_SLASH) && !press.used_fallback() {
        return Some(Query::Empty);
    }
    press.text().map(Query::Character)
}

/// Whether `press` opens the filter: what the command's stage reads.
pub(super) fn reads(press: &KeyPress) -> bool {
    query(press).is_some()
}

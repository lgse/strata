// SPDX-License-Identifier: MIT

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
pub(crate) fn reads(press: &KeyPress) -> bool {
    query(press).is_some()
}

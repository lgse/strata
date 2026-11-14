// SPDX-License-Identifier: MIT

//! What the backend reads of the filter (A10): which entry carries its Enter mark, GTK's
//! `ENTER_TARGET_CLASS` on a row (`src/qa/targets/states.rs`), read as the `marked` state.

use strata_components::contract::EntryId;
use strata_qa::Mark;

use super::tree::Seen;
use crate::App;

/// Whether `seen` is the entry the filter's Enter would open, outlined as its target.
pub(super) fn marked(app: &App, seen: &Seen) -> bool {
    if seen.mark != Some(Mark::Entry) {
        return false;
    }
    let digits = seen
        .key
        .as_deref()
        .map(|key| key.trim_start_matches(|c: char| !c.is_ascii_digit()));
    let id = digits
        .and_then(|digits| digits.parse().ok())
        .map(EntryId::new);
    id.is_some_and(|id| crate::filter::outlined(app) == Some(id))
}

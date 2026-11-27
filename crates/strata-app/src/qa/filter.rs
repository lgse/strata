// SPDX-License-Identifier: MIT

use strata_components::contract::EntryId;
use strata_qa::Mark;

use super::tree::Seen;

pub(super) fn marked(outlined: Option<EntryId>, seen: &Seen) -> bool {
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
    id.is_some_and(|id| outlined == Some(id))
}

// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use strata_core::model::Location;

use super::Ids;

#[test]
fn retain_ids_drops_unreferenced_entries_without_reusing_pruned_ids() {
    let ids = Ids::default();
    let keep_loc = Location::local("/tmp/keep.jpg");
    let drop_loc = Location::local("/tmp/drop.jpg");

    let keep_id = ids.of(&keep_loc);
    let drop_id = ids.of(&drop_loc);
    assert_eq!(ids.len(), 2);

    let keep = HashSet::from([keep_id]);
    let dropped = ids.retain_ids(&keep);
    assert_eq!(dropped, 1);
    assert_eq!(ids.len(), 1);
    assert_eq!(
        ids.of(&keep_loc),
        keep_id,
        "retained locations must keep their EntryId"
    );

    let reissued = ids.of(&drop_loc);
    assert_ne!(
        reissued, drop_id,
        "pruned EntryIds must never be reused on a later lookup"
    );
    assert_ne!(reissued, keep_id);
}

// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn thumbnail_counters_are_monotonic() {
    let before = thumbnail_counts();
    mark_thumbnail_eligible(2);
    mark_thumbnail_requested();
    mark_thumbnail_started();
    mark_thumbnail_completed();
    mark_thumbnail_applied();
    mark_thumbnail_cancelled();
    mark_thumbnail_stale();
    let after = thumbnail_counts();
    assert_eq!(after.eligible, before.eligible + 2);
    assert_eq!(after.requested, before.requested + 1);
    assert_eq!(after.started, before.started + 1);
    assert_eq!(after.completed, before.completed + 1);
    assert_eq!(after.applied, before.applied + 1);
    assert_eq!(after.cancelled, before.cancelled + 1);
    assert_eq!(after.stale, before.stale + 1);
}

#[test]
fn stage_probes_do_not_panic() {
    record_stage("test-enumeration", 3);
    mark_first_themed_frame();
    mark_first_visible_row(1);
    // Second calls are idempotent one-shots.
    mark_first_themed_frame();
    mark_first_visible_row(1);
}

// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn stage_probes_do_not_panic() {
    record_stage("test-enumeration", 3);
    mark_first_themed_frame();
    mark_first_visible_row(1);
    // Second calls are idempotent one-shots.
    mark_first_themed_frame();
    mark_first_visible_row(1);
}

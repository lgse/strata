// SPDX-License-Identifier: MIT

//! What this crate's tests share.

use std::path::PathBuf;

/// Where a test run in a process of its own says that it ran.
const RAN: &str = "STRATA_TEST_RAN";

/// Runs `run` as the test `test` in a process of its own, with a home and XDG folders it can
/// discard, and fails when no test ran there.
///
/// The child is this executable started for the one test whose path is `test` exactly
/// (`strata_test_support::isolated_test`). A path that is no test's starts nothing, and the
/// child still exits well, so without the marker a test moved to another module, or renamed,
/// passes without running (finding lgse/strata#2193). Every test of this crate that runs in a
/// process of its own goes through here.
pub(crate) fn in_own_process(test: &str, run: impl FnOnce()) {
    let own = tempfile::tempdir().expect("a folder for the marker");
    // The parent picks the place, and the child is told it.
    let marker = std::env::var_os(RAN).map_or_else(|| own.path().join("ran"), PathBuf::from);
    strata_test_support::isolated_test(
        test,
        [(RAN, marker.as_os_str())],
        || true,
        || {
            std::fs::write(&marker, test).expect("the marker");
            run();
        },
    );
    assert!(
        marker.exists(),
        "`{test}` ran nowhere: no test of this executable has that path, so the child ran none \
         and exited well. Give the test's whole path, with every module it is in."
    );
}

#[cfg(test)]
mod tests;

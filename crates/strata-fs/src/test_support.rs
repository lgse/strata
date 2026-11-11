// SPDX-License-Identifier: MIT

//! Test helpers for code that drives the file operations and the browser's state.

pub mod operations;

pub use strata_test_support::*;

/// Runs the test `name` in a process of its own with disposable preferences, the mount
/// watcher started and its thread marked as the UI thread.
pub fn isolated_test(name: &str, run: impl FnOnce()) {
    strata_test_support::isolated_test(
        name,
        std::iter::empty::<(&str, &str)>(),
        || {
            crate::adapters::mount_watch::start();
            crate::util::ui_thread::mark();
            true
        },
        run,
    );
}

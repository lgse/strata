// SPDX-License-Identifier: GPL-3.0-or-later

#![cfg(test)]

use std::sync::Mutex;

/// Serializes tests that drive `glib::MainContext::default()` directly, since it is a
/// process-wide singleton and concurrent access from the test harness's per-test threads panics
/// with a GLib thread-affinity error. A single shared lock, not one static per module: two
/// separate locks each covering only their own module's tests do not prevent a test in one
/// module from racing a test in another, since neither knows about the other's lock.
pub(crate) static ASYNC_MAIN_CONTEXT_DEFAULT: Mutex<()> = Mutex::new(());

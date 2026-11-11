// SPDX-License-Identifier: MIT

//! What a process does before its interface thread asks for a first listing.

use crate::{adapters::mount_watch, util::ui_thread};

/// Makes the calling thread the UI thread, which file calls are refused on from here.
///
/// The mount watcher starts first: every listing reads it, and its own first read of the
/// mount table is a file call.
pub fn claim_ui_thread() {
    mount_watch::start();
    ui_thread::mark();
}

/// [`claim_ui_thread`], then the watchdog that reports a stalled UI thread
/// (`STRATA_UI_STALL_MS`). For a thread about to run the default GLib context.
pub fn claim_watched_ui_thread() {
    claim_ui_thread();
    ui_thread::stalls::start();
}

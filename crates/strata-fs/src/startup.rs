// SPDX-License-Identifier: MIT

//! What a process does before its interface thread asks for a first listing.

use crate::{adapters::mount_watch, util::ui_thread};

pub fn claim_ui_thread() {
    mount_watch::start();
    ui_thread::mark();
}

pub fn claim_watched_ui_thread() {
    claim_ui_thread();
    ui_thread::stalls::start();
}

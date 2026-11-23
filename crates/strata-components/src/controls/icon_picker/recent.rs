// SPDX-License-Identifier: MIT

//! Recently used choices: a list the host keeps and stores, newest first.

use super::data::Choice;

/// The most recent choices kept: native `MAX_RECENT`.
pub const RECENT_MAX: usize = 21;

/// Puts `choice` first in `recent`, dropping an earlier copy of it and anything past `RECENT_MAX`.
pub fn remember(recent: &mut Vec<Choice>, choice: Choice) {
    recent.retain(|c| *c != choice);
    recent.insert(0, choice);
    recent.truncate(RECENT_MAX);
}

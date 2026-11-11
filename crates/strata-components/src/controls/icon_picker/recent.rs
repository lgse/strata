//! Recently used choices: a list the host keeps and stores, newest first.

use super::data::Choice;

/// The most recent choices kept: GTK's `MAX_RECENT` (7 × 3, `gtk/gtkemojichooser.c`).
pub const RECENT_MAX: usize = 21;

/// Puts `choice` first in `recent`, dropping an earlier copy of it and anything past
/// [`RECENT_MAX`], as GTK's `add_recent_item` does. The host calls it from its `on_choose` and
/// stores the list; the picker reads it with `IconPickerState::set_recent`.
///
/// Used by: hosts.
pub fn remember(recent: &mut Vec<Choice>, choice: Choice) {
    recent.retain(|c| *c != choice);
    recent.insert(0, choice);
    recent.truncate(RECENT_MAX);
}

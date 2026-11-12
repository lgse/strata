// SPDX-License-Identifier: MIT

//! What a [`Matcher`] reads, for an interface that lists the keys its commands answer as
//! well as matching them: a palette's keys, a label, a conflict check. A matcher is built
//! with `Command::in_map` and `Command::ignoring`; these say what was built.
//!
//! Not said: whether a structural matcher also reads its command's other keys as pressed
//! (`Command::alternates`, `Command::or_alternates`). That choice is a private type of
//! `command.rs`, so a reader that must tell **h** from ← for `left` cannot see it from here,
//! and exposing it is a further change. Nothing reads it yet.
//!
//! New code beside `command.rs`, which is a moved file and takes none
//! (`docs/rune/shared-state.md`).

use super::{Command, Map, Matcher};
use crate::keys::Modifiers;

impl Matcher {
    /// The command whose bindings it matches.
    pub const fn command(&self) -> Command {
        self.command
    }

    /// The map whose table it reads whichever map is active, or `None` when it follows the
    /// active one.
    pub const fn map(&self) -> Option<Map> {
        self.map
    }

    /// The modifiers that may be held beside each binding's own.
    pub const fn ignored(&self) -> Modifiers {
        self.ignored
    }
}

#[cfg(test)]
mod tests;

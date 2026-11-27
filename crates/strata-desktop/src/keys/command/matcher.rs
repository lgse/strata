// SPDX-License-Identifier: MIT

use super::{Command, Map, Matcher};
use crate::keys::Modifiers;

impl Matcher {
    /// The command whose bindings it matches.
    pub const fn command(&self) -> Command {
        self.command
    }

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

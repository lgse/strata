// SPDX-License-Identifier: MIT

use std::ffi::OsString;

use xkbcommon::xkb::{self, compose};

/// What a press does to a dead-key sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Composed {
    /// The key is no part of a sequence, and types what it types.
    Passed,
    /// A sequence goes on: a window hears nothing yet.
    Pending,
    /// The sequence is complete, and types this.
    Text(String),
    /// The key broke a sequence off, which then types nothing.
    Cancelled,
}

pub(super) struct Compose(compose::State);

fn locale() -> OsString {
    let set = ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find(|value| !value.is_empty());
    match set {
        Some(locale) if locale != "C" && locale != "POSIX" => locale,
        _ => OsString::from("C.UTF-8"),
    }
}

impl Compose {
    pub(super) fn of_session() -> Option<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let table =
            compose::Table::new_from_locale(&context, &locale(), compose::COMPILE_NO_FLAGS).ok()?;
        Some(Self(compose::State::new(&table, compose::STATE_NO_FLAGS)))
    }

    pub(super) fn feed(&mut self, keysym: xkb::Keysym) -> Composed {
        if self.0.feed(keysym) == compose::FeedResult::Ignored {
            return Composed::Passed;
        }
        match self.0.status() {
            compose::Status::Nothing => Composed::Passed,
            compose::Status::Composing => Composed::Pending,
            compose::Status::Composed => self.0.utf8().map_or(Composed::Passed, Composed::Text),
            compose::Status::Cancelled => Composed::Cancelled,
        }
    }
}

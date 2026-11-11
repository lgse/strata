// SPDX-License-Identifier: MIT

//! Dead keys, as a Rune window hears them. Rune's window host takes its keys from winit,
//! which runs every press through xkbcommon's compose state, built from the Compose table of
//! the process's locale. The key that starts a sequence reaches no handler, the key that
//! completes it arrives as the composed text, and a key that breaks it off is still
//! delivered when it is a named key: Escape after a dead key is an Escape press. The GTK
//! application composes in its input method, where that Escape only ends the sequence.

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

/// The locale whose Compose table a window composes with: winit's order of the environment.
/// A process with none set runs in the C locale, whose table composes Latin-1 only; no
/// desktop session starts a window so, and the QA runner sets none, so that case reads the
/// UTF-8 table every UTF-8 locale shares.
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
    /// `None` where the system has no Compose table for the locale: keys then type as they
    /// do without one.
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

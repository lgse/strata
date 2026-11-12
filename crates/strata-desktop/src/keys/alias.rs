// SPDX-License-Identifier: MIT

//! Escape, Enter, Tab and the arrows are structural commands: a press of one
//! of their other bindings is read as the canonical key before any handler
//! sees it, so every surface that matches the command follows it. See
//! `docs/keyboard-shortcuts.md`.
//!
//! An interface says what owns typing where a press lands, as a [`Context`], and
//! [`rewrite`] does the rest.

use super::{
    Binding, Command, Key, KeyPress, Map, Match, Modifiers, bindings,
    command::{active_keymap, registry},
};

/// Escape always matches `escape`, even where a table leaves it out.
pub const FALLBACK: Command = bindings::ESCAPE;

/// What owns typing where a press lands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// A focused text field or popover, or type-to-search, reads plain
    /// printable keys as its own. In 10xer a read-only text view, such as a
    /// text preview, types nothing; the default map keeps treating it as a
    /// field.
    pub typing: bool,
}

impl Context {
    fn aliasing(self) -> Aliasing {
        if self.typing {
            Aliasing::Untyped
        } else {
            Aliasing::All
        }
    }
}

/// Which alternates a press's context lets through. `Off` is a press that
/// did not come through `on_press`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Aliasing {
    Off,
    Untyped,
    All,
}

impl Aliasing {
    pub(super) fn allows(self, typed: bool) -> bool {
        match self {
            Self::Off => false,
            Self::Untyped => !typed,
            Self::All => true,
        }
    }
}

/// The key a rewritten press came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Alias {
    pub(super) key: Key,
    pub(super) modifiers: Modifiers,
    /// A plain printable alternate, such as **j** for ↓.
    pub(super) typed: bool,
}

fn structural() -> impl Iterator<Item = (Command, Binding)> {
    registry().filter_map(|command| command.canonical().map(|canonical| (command, canonical)))
}

/// Reads `press` as the canonical key of the structural command whose
/// alternate it matches exactly in the active map, where `context` lets that
/// alternate through. A canonical key, with any modifiers, is never rewritten.
pub fn rewrite(press: KeyPress, context: Context) -> KeyPress {
    let aliasing = context.aliasing();
    let press = KeyPress { aliasing, ..press };
    if press.is_modifier() || press.aliased() {
        return press;
    }
    if structural().any(|(_, canonical)| canonical.key() == press.key) {
        return press;
    }
    let keymap = active_keymap();
    let map = Map::active();
    for (command, canonical) in structural() {
        let alternate = keymap.bindings(map, command).iter().find(|binding| {
            **binding != canonical && aliasing.allows(binding.is_typed()) && binding.matches(&press)
        });
        if let Some(alternate) = alternate {
            return press.read_as(canonical, alternate.is_typed());
        }
    }
    press
}

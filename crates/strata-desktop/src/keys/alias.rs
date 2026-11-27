// SPDX-License-Identifier: MIT

use super::{
    Binding, Command, Key, KeyPress, Map, Match, Modifiers, bindings,
    command::{active_keymap, registry},
};

/// Escape always matches `escape`, even where a table leaves it out.
pub const FALLBACK: Command = bindings::ESCAPE;

/// What owns typing where a press lands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
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

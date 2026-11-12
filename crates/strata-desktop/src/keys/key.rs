// SPDX-License-Identifier: MIT

//! A key and the held modifiers, without a toolkit type.

use glib::bitflags::bitflags;

use super::backend::backend;

/// A key, as an XKB keysym: what a saved keymap names and what GDK's key type wraps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(u32);

impl Key {
    pub const fn from_keysym(keysym: u32) -> Self {
        Self(keysym)
    }

    pub const fn keysym(self) -> u32 {
        self.0
    }

    /// The key's name, as `gdk_keyval_name` gives it. `None` for a value that is not a key.
    pub fn name(self) -> Option<String> {
        backend().key_name(self)
    }

    /// The key a name stands for.
    pub fn from_name(name: &str) -> Option<Self> {
        backend().key_from_name(name)
    }

    /// The character the key types, if it types one.
    pub fn to_unicode(self) -> Option<char> {
        backend().key_to_unicode(self)
    }

    /// The key that types `character`.
    pub fn from_char(character: char) -> Self {
        backend().key_from_unicode(character)
    }

    pub fn to_lower(self) -> Self {
        backend().key_to_lower(self)
    }

    pub fn to_upper(self) -> Self {
        backend().key_to_upper(self)
    }
}

macro_rules! declare {
    ($($name:ident = $keysym:literal,)*) => {
        #[expect(
            non_upper_case_globals,
            reason = "the constants keep the keys' own names, which saved keymaps and the catalog use"
        )]
        impl Key {
            $(pub const $name: Self = Self($keysym);)*
        }
    };
}
crate::key_constants!(declare);

bitflags! {
    /// Held modifiers, with GDK's names and bits so that the GTK application converts
    /// without a table. Shortcuts compare the first four; see `SHORTCUT_MODIFIERS`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Modifiers: u32 {
        const SHIFT_MASK = 1;
        const LOCK_MASK = 1 << 1;
        const CONTROL_MASK = 1 << 2;
        const ALT_MASK = 1 << 3;
        const SUPER_MASK = 1 << 26;
        const HYPER_MASK = 1 << 27;
        const META_MASK = 1 << 28;
    }
}

// SPDX-License-Identifier: MIT

//! The keyboard the QA mode types on: which modifiers are down, and the layout.

use rune::{KeyEvent, Mods};
use strata_qa::{Keymap, Modifier};

use super::{
    keymap::{self, Layout},
    keys::{self, Named},
};

#[derive(Default)]
pub(super) struct Keyboard {
    /// The modifiers a `hold` or a `key_down` step pressed and has not released.
    held: Vec<Modifier>,
    /// `None` on the US layout the application starts with.
    layout: Option<Layout>,
}

impl Keyboard {
    pub(super) fn set_layout(&mut self, keymap: &Keymap) -> Result<(), String> {
        self.layout = if keymap::is_plain_us(keymap) {
            None
        } else {
            Some(Layout::compile(keymap)?)
        };
        Ok(())
    }

    pub(super) fn hold(&mut self, modifier: Modifier) {
        if !self.held.contains(&modifier) {
            self.held.push(modifier);
        }
    }

    pub(super) fn release(&mut self, modifier: Modifier) {
        self.held.retain(|held| *held != modifier);
    }

    /// Whether the right Alt key shifts to a layout's third level instead of being Alt.
    fn altgr_shifts_level(&self) -> bool {
        self.layout.as_ref().is_some_and(Layout::has_altgr)
    }

    /// The modifiers Rune sees held between key presses.
    pub(super) fn mods(&self) -> Mods {
        keys::mods(&self.held, self.altgr_shifts_level())
    }

    /// The press of the key a scenario names, with `with` held beside what is down already.
    pub(super) fn press(&self, name: &str, with: &[Modifier]) -> Result<KeyEvent, String> {
        let mut held = self.held.clone();
        held.extend(with);
        let shift = held.contains(&Modifier::Shift);
        let (key, shift, us) = match &self.layout {
            None => {
                let key = keys::named(name)?;
                let implied = matches!(key, Named::Character(typed) if keys::needs_shift(typed));
                (key, shift || implied, None)
            }
            Some(layout) => {
                let altgr = held.contains(&Modifier::AltGr) && layout.has_altgr();
                let typed = layout
                    .typed(name, shift, altgr)
                    .ok_or_else(|| format!("no key of a US keyboard is named {name:?}"))?;
                (typed.key, typed.shift, typed.us)
            }
        };
        let mods = Mods {
            shift,
            ..keys::mods(&held, self.altgr_shifts_level())
        };
        Ok(keys::event(key, us, mods))
    }

    /// The press that types `character`. A character no US key types is sent as it is, where
    /// the GTK harness lends it a keycode.
    pub(super) fn typed(&self, character: char) -> KeyEvent {
        self.press(&character.to_string(), &[])
            .unwrap_or_else(|_| keys::event(Named::Character(character), None, self.mods()))
    }
}

#[cfg(test)]
mod tests;

// SPDX-License-Identifier: MIT

//! The keyboard the QA mode types on: which modifiers are down, and the layout.

use std::cell::Cell;

use rune::{Key, KeyEvent, Mods};
use strata_qa::{Keymap, Modifier};

use super::{
    compose::{Compose, Composed},
    keymap::{self, Layout},
    keys::{self, Named},
};
use crate::commands::{Origin, XKey};

#[derive(Default)]
pub(super) struct Keyboard {
    /// The modifiers a `hold` or a `key_down` step pressed and has not released.
    held: Vec<Modifier>,
    /// `None` on the US layout the application starts with.
    layout: Option<Layout>,
    /// The dead-key sequence in progress under a layout that has dead keys.
    compose: Option<Compose>,
    /// The physical origin of the last non-US press, for `commands::read` (packet entry 97).
    origin: Cell<Option<Origin>>,
}

impl Keyboard {
    pub(super) fn set_layout(&mut self, keymap: &Keymap) -> Result<(), String> {
        self.layout = if keymap::is_plain_us(keymap) {
            None
        } else {
            Some(Layout::compile(keymap)?)
        };
        // A new layout starts no sequence where the last one left off.
        self.compose = self.layout.as_ref().and_then(|_| Compose::of_session());
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
        self.origin.set(None);
        let mut held = self.held.clone();
        held.extend(with);
        let shift = held.contains(&Modifier::Shift);
        let (key, shift, us, origin) = match &self.layout {
            None => {
                let key = keys::named(name)?;
                let implied = matches!(key, Named::Character(typed) if keys::needs_shift(typed));
                (key, shift || implied, None, None)
            }
            Some(layout) => {
                let altgr = held.contains(&Modifier::AltGr) && layout.has_altgr();
                let typed = layout
                    .typed(name, shift, altgr)
                    .ok_or_else(|| format!("no key of a US keyboard is named {name:?}"))?;
                let origin = Origin::from_x(XKey {
                    keycode: typed.keycode,
                    base: typed.base,
                });
                (typed.key, typed.shift, typed.us, Some(origin))
            }
        };
        self.origin.set(origin);
        let mods = Mods {
            shift,
            ..keys::mods(&held, self.altgr_shifts_level())
        };
        Ok(keys::event(key, us, mods))
    }

    pub(super) fn origin(&self) -> Option<Origin> {
        self.origin.get()
    }

    /// What the key's press does to a dead-key sequence, under a layout that has one.
    fn compose(&mut self, name: &str, with: &[Modifier]) -> Composed {
        let (Some(layout), Some(compose)) = (&self.layout, &mut self.compose) else {
            return Composed::Passed;
        };
        let holds = |modifier| self.held.contains(&modifier) || with.contains(&modifier);
        let altgr = holds(Modifier::AltGr) && layout.has_altgr();
        let keysym = layout.keysym(name, holds(Modifier::Shift), altgr);
        keysym.map_or(Composed::Passed, |keysym| compose.feed(keysym))
    }

    pub(super) fn strike(
        &mut self,
        name: &str,
        with: &[Modifier],
    ) -> Result<Option<KeyEvent>, String> {
        match self.compose(name, with) {
            Composed::Pending => {
                self.origin.set(None);
                Ok(None)
            }
            Composed::Text(text) => {
                self.origin.set(None);
                Ok(Some(KeyEvent {
                    key: Key::Text(text),
                    mods: self.mods(),
                }))
            }
            Composed::Cancelled => {
                let pressed = self.press(name, with)?;
                let types = matches!(pressed.key, Key::Text(_) | Key::Command(_));
                if types {
                    self.origin.set(None);
                }
                Ok((!types).then_some(pressed))
            }
            Composed::Passed => self.press(name, with).map(Some),
        }
    }

    pub(super) fn typed(&mut self, character: char) -> Option<KeyEvent> {
        let unlisted = keys::event(Named::Character(character), None, self.mods());
        self.strike(&character.to_string(), &[])
            .unwrap_or_else(|_| {
                self.origin.set(None);
                Some(unlisted)
            })
    }
}

#[cfg(test)]
mod tests;

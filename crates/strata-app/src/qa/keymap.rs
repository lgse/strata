// SPDX-License-Identifier: MIT

//! A keyboard layout other than the US one, for a host with no display server to ask.
//!
//! The runner's `layout` step swaps the keymap of the X display it started, which the GTK
//! application types through. Rune's offscreen host has no display, so the runner also sends
//! the layout here as a `keymap` step, and this module answers what a key at a US position
//! types under it, from the same xkeyboard-config data the X server compiles.

use std::collections::HashMap;

use strata_qa::{Keymap, Modifier};
use xkbcommon::xkb;

use super::keys::{self, Named};

/// What the key at a US position types under a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Typed {
    pub(super) key: Named,
    /// Whether Shift is down for it: held by the step, or needed to reach the key's US name.
    pub(super) shift: bool,
    /// The character the key types unshifted on a US layout.
    pub(super) us: Option<char>,
}

pub(super) struct Layout {
    keymap: xkb::Keymap,
    us: xkb::Keymap,
    /// Each keysym's key on a US keyboard, and whether it needs Shift there: the same
    /// first-match order as the X keymap, keycodes upward, unshifted before shifted.
    positions: HashMap<u32, (xkb::Keycode, bool)>,
    group: u32,
    caps_lock: bool,
}

/// Whether `keymap` is the layout the application starts with, which needs no compiling.
pub(super) fn is_plain_us(keymap: &Keymap) -> bool {
    keymap.layouts == "us" && keymap.variants.is_empty() && keymap.group == 0 && !keymap.caps_lock
}

fn compile(context: &xkb::Context, layouts: &str, variants: &str) -> Result<xkb::Keymap, String> {
    xkb::Keymap::new_from_names(
        context,
        "evdev",
        "pc105",
        layouts,
        variants,
        Some(String::new()),
        xkb::COMPILE_NO_FLAGS,
    )
    .ok_or_else(|| format!("cannot compile the keyboard layout {layouts:?} {variants:?}"))
}

fn us_positions(us: &xkb::Keymap) -> HashMap<u32, (xkb::Keycode, bool)> {
    let mut positions = HashMap::new();
    for raw in us.min_keycode().raw()..=us.max_keycode().raw() {
        let keycode = xkb::Keycode::new(raw);
        for level in 0..2 {
            if let Some(keysym) = us.key_get_syms_by_level(keycode, 0, level).first() {
                positions
                    .entry(keysym.raw())
                    .or_insert((keycode, level == 1));
            }
        }
    }
    positions
}

/// The keysym a key name means: GDK's names are X's, and a character is its own keysym.
fn keysym(name: &str) -> Option<u32> {
    let mut characters = name.chars();
    if let (Some(character), None) = (characters.next(), characters.next()) {
        let code = u32::from(character);
        return Some(if code < 0x100 {
            code
        } else {
            0x0100_0000 | code
        });
    }
    let keysym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS).raw();
    (keysym != xkb::keysyms::KEY_NoSymbol).then_some(keysym)
}

fn character(code: u32) -> Option<char> {
    char::from_u32(code).filter(|character| *character != '\0' && !character.is_control())
}

impl Layout {
    pub(super) fn compile(keymap: &Keymap) -> Result<Self, String> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let us = compile(&context, "us", "")?;
        let layout = compile(&context, &keymap.layouts, &keymap.variants)?;
        if u32::from(keymap.group) >= layout.num_layouts() {
            return Err(format!(
                "the keyboard layout {:?} has no group {}",
                keymap.layouts, keymap.group
            ));
        }
        Ok(Self {
            positions: us_positions(&us),
            keymap: layout,
            us,
            group: u32::from(keymap.group),
            caps_lock: keymap.caps_lock,
        })
    }

    fn modifier(&self, name: &str) -> xkb::ModMask {
        let index = self.keymap.mod_get_index(name);
        if index == xkb::MOD_INVALID {
            0
        } else {
            1 << index
        }
    }

    /// Whether the right Alt key is this layout's level shift, as on a German keyboard, and
    /// not Alt, as on a US one.
    pub(super) fn has_altgr(&self) -> bool {
        let right_alt = keysym(Modifier::AltGr.keysym_name());
        let position = right_alt.and_then(|keysym| self.positions.get(&keysym));
        position.is_some_and(|(keycode, _)| {
            let here = self.keymap.key_get_syms_by_level(*keycode, self.group, 0);
            here.iter()
                .any(|keysym| keysym.raw() == xkb::keysyms::KEY_ISO_Level3_Shift)
        })
    }

    /// What the key named `name` on a US keyboard types here, with Shift or AltGr held.
    /// `None` for a name no US key has: a character such a keyboard cannot type.
    pub(super) fn typed(&self, name: &str, shift: bool, altgr: bool) -> Option<Typed> {
        let (keycode, needs_shift) = *self.positions.get(&keysym(name)?)?;
        let shift = shift || needs_shift;
        let mut held = 0;
        if shift {
            held |= self.modifier(xkb::MOD_NAME_SHIFT);
        }
        if altgr {
            // ISO_Level3_Shift, which xkeyboard-config maps to Mod5.
            held |= self.modifier("Mod5");
        }
        let locked = if self.caps_lock {
            self.modifier(xkb::MOD_NAME_CAPS)
        } else {
            0
        };
        let mut state = xkb::State::new(&self.keymap);
        state.update_mask(held, 0, locked, 0, 0, self.group);
        let named = keys::named(&xkb::keysym_get_name(state.key_get_one_sym(keycode))).ok();
        let key = match named {
            Some(key) if !matches!(key, Named::Character(_)) => key,
            _ => Named::Character(character(state.key_get_utf32(keycode))?),
        };
        let us = self.us.key_get_syms_by_level(keycode, 0, 0).first();
        Some(Typed {
            key,
            shift,
            us: us.and_then(|keysym| character(xkb::keysym_to_utf32(*keysym))),
        })
    }
}

#[cfg(test)]
mod tests;

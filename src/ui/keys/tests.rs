// SPDX-License-Identifier: MIT

use super::{Binding, Digits, Event, Key, KeyPress, Modifiers, normalize};

mod catalog;
mod composition;
mod constants;
mod digits;
mod labels;
mod letters;
mod modifiers;
mod parts;
mod table;

const NONE: Modifiers = Modifiers::empty();
const SHIFT: Modifiers = Modifiers::SHIFT_MASK;
const CTRL: Modifiers = Modifiers::CONTROL_MASK;
const CTRL_SHIFT: Modifiers = CTRL.union(SHIFT);
const CAPS: Modifiers = Modifiers::LOCK_MASK;

/// A layout's level 0 and 1 characters by GDK keycode, as `Display::map_keycode`
/// reports them (rows recorded from GDK 4.22 on Xvfb with xkeyboard-config).
type Layout = &'static [(u32, [Key; 2])];

const US: Layout = &[
    (10, [Key::_1, Key::exclam]),
    (11, [Key::_2, Key::at]),
    (38, [Key::a, Key::A]),
    (44, [Key::j, Key::J]),
    (54, [Key::c, Key::C]),
    (60, [Key::period, Key::greater]),
    (61, [Key::slash, Key::question]),
];
const RU: Layout = &[
    (11, [Key::_2, Key::quotedbl]),
    (29, [Key::Cyrillic_en, Key::Cyrillic_EN]),
    (38, [Key::Cyrillic_ef, Key::Cyrillic_EF]),
    (44, [Key::Cyrillic_o, Key::Cyrillic_O]),
    (45, [Key::Cyrillic_el, Key::Cyrillic_EL]),
    (54, [Key::Cyrillic_es, Key::Cyrillic_ES]),
    (60, [Key::Cyrillic_yu, Key::Cyrillic_YU]),
    (61, [Key::period, Key::comma]),
];
const UK: Layout = &[
    (39, [Key::Ukrainian_i, Key::Ukrainian_I]),
    (54, [Key::Cyrillic_es, Key::Cyrillic_ES]),
];
const EL: Layout = &[
    (24, [Key::semicolon, Key::colon]),
    (25, [Key::Greek_finalsmallsigma, Key::Greek_SIGMA]),
    (54, [Key::Greek_psi, Key::Greek_PSI]),
];
const HE: Layout = &[(24, [Key::slash, Key::Q]), (54, [Key::hebrew_bet, Key::C])];
const FR: Layout = &[
    (10, [Key::ampersand, Key::_1]),
    (11, [Key::eacute, Key::_2]),
    (15, [Key::minus, Key::_6]),
    (19, [Key::agrave, Key::_0]),
    (24, [Key::a, Key::A]),
    (38, [Key::q, Key::Q]),
    (59, [Key::semicolon, Key::period]),
];
const DE: Layout = &[
    (11, [Key::_2, Key::quotedbl]),
    (20, [Key::ssharp, Key::question]),
    (29, [Key::z, Key::Z]),
    (52, [Key::y, Key::Y]),
];
const DVORAK: Layout = &[(54, [Key::j, Key::J])];
const LT: Layout = &[(10, [Key::aogonek, Key::Aogonek])];
const CZ: Layout = &[(10, [Key::plus, Key::_1])];
const HU: Layout = &[
    (19, [Key::odiaeresis, Key::Odiaeresis]),
    (49, [Key::_0, Key::section]),
];

/// Where a press lands: `keycode` at `level` of `layout` among the configured `layouts`.
struct Slot {
    layouts: &'static [Layout],
    layout: u32,
    keycode: u32,
    level: u32,
}

impl Slot {
    fn keys_at(&self, keycode: u32) -> Vec<(u32, u32, Key)> {
        self.layouts
            .iter()
            .enumerate()
            .flat_map(|(layout, rows)| {
                rows.iter()
                    .filter(move |(code, _)| *code == keycode)
                    .flat_map(move |(_, levels)| {
                        levels
                            .iter()
                            .enumerate()
                            .map(move |(level, key)| (layout as u32, level as u32, *key))
                    })
            })
            .collect()
    }
}

fn slot(layouts: &'static [Layout], layout: u32, keycode: u32, level: u32) -> Slot {
    Slot {
        layouts,
        layout,
        keycode,
        level,
    }
}

/// What GDK delivered: the character `typed`, the key and modifiers it
/// `matched` after consuming any that typed the character, and the full `state`.
struct Typed {
    typed: Key,
    matched: (Key, Modifiers),
    state: Modifiers,
}

/// A press that consumed no modifier: it matches the key it typed under `modifiers`.
fn typed(key: Key, modifiers: Modifiers) -> Typed {
    Typed {
        typed: key,
        matched: (key, modifiers),
        state: modifiers,
    }
}

fn press(slot: Slot, keys: Typed) -> KeyPress {
    let event = Event {
        matched: keys.matched,
        typed: keys.typed,
        state: keys.state,
        keycode: slot.keycode,
        layout: slot.layout,
        level: slot.level,
        modifier: false,
    };
    normalize(&event, |keycode| slot.keys_at(keycode))
}

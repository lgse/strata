// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods};
use strata_qa::Modifier;

/// What a key name stands for before a layout is asked what it types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Named {
    Backspace,
    Delete,
    Down,
    End,
    Enter,
    Escape,
    Home,
    Insert,
    Left,
    Menu,
    PageDown,
    PageUp,
    Right,
    Tab,
    Up,
    Function(u8),
    /// A key that types this character on a US layout.
    Character(char),
}

pub(super) const NAMES: [(&str, Named); 63] = [
    ("BackSpace", Named::Backspace),
    ("Delete", Named::Delete),
    ("Down", Named::Down),
    ("End", Named::End),
    ("Escape", Named::Escape),
    ("Home", Named::Home),
    ("Insert", Named::Insert),
    ("KP_Delete", Named::Delete),
    ("KP_Down", Named::Down),
    ("KP_End", Named::End),
    ("KP_Enter", Named::Enter),
    ("KP_Home", Named::Home),
    ("KP_Insert", Named::Insert),
    ("KP_Left", Named::Left),
    ("KP_Page_Down", Named::PageDown),
    ("KP_Page_Up", Named::PageUp),
    ("KP_Right", Named::Right),
    ("KP_Up", Named::Up),
    ("Left", Named::Left),
    ("Menu", Named::Menu),
    ("Page_Down", Named::PageDown),
    ("Page_Up", Named::PageUp),
    ("Return", Named::Enter),
    ("Right", Named::Right),
    ("Tab", Named::Tab),
    ("Up", Named::Up),
    ("ampersand", Named::Character('&')),
    ("apostrophe", Named::Character('\'')),
    ("asciicircum", Named::Character('^')),
    ("asciitilde", Named::Character('~')),
    ("asterisk", Named::Character('*')),
    ("at", Named::Character('@')),
    ("backslash", Named::Character('\\')),
    ("bar", Named::Character('|')),
    ("braceleft", Named::Character('{')),
    ("braceright", Named::Character('}')),
    ("bracketleft", Named::Character('[')),
    ("bracketright", Named::Character(']')),
    ("colon", Named::Character(':')),
    ("comma", Named::Character(',')),
    ("dollar", Named::Character('$')),
    ("equal", Named::Character('=')),
    ("exclam", Named::Character('!')),
    ("grave", Named::Character('`')),
    ("greater", Named::Character('>')),
    ("less", Named::Character('<')),
    ("minus", Named::Character('-')),
    ("numbersign", Named::Character('#')),
    ("parenleft", Named::Character('(')),
    ("parenright", Named::Character(')')),
    ("percent", Named::Character('%')),
    ("period", Named::Character('.')),
    ("plus", Named::Character('+')),
    ("question", Named::Character('?')),
    ("quotedbl", Named::Character('"')),
    ("semicolon", Named::Character(';')),
    ("slash", Named::Character('/')),
    ("space", Named::Character(' ')),
    ("underscore", Named::Character('_')),
    // X's own names for the page keys, which a compiled layout reports.
    ("KP_Next", Named::PageDown),
    ("KP_Prior", Named::PageUp),
    ("Next", Named::PageDown),
    ("Prior", Named::PageUp),
];

/// The characters a US keyboard types with Shift held, beside the capital letters.
const SHIFTED: &str = "~!@#$%^&*()_+{}|:\"<>?";

/// The key a scenario's key name means. A single character is the key that types it.
pub(super) fn named(name: &str) -> Result<Named, String> {
    let mut characters = name.chars();
    if let (Some(character), None) = (characters.next(), characters.next()) {
        return Ok(Named::Character(character));
    }
    let listed = NAMES
        .iter()
        .find_map(|(known, key)| (*known == name).then_some(*key));
    let function = name
        .strip_prefix('F')
        .and_then(|number| number.parse::<u8>().ok())
        .filter(|number| (1..=24).contains(number))
        .map(Named::Function);
    listed
        .or(function)
        .ok_or_else(|| format!("no key named {name:?}"))
}

/// Whether a US keyboard needs Shift held to type the character.
pub(super) fn needs_shift(character: char) -> bool {
    character.is_uppercase() || SHIFTED.contains(character)
}

impl Named {
    pub(super) fn key(self, mods: Mods) -> Key {
        match self {
            Self::Backspace => Key::Backspace,
            Self::Delete => Key::Delete,
            Self::Down => Key::Down,
            Self::End => Key::End,
            Self::Enter => Key::Enter,
            Self::Escape => Key::Escape,
            Self::Home => Key::Home,
            Self::Insert => Key::Insert,
            Self::Left => Key::Left,
            Self::Menu => Key::ContextMenu,
            Self::PageDown => Key::PageDown,
            Self::PageUp => Key::PageUp,
            Self::Right => Key::Right,
            Self::Tab => Key::Tab,
            Self::Up => Key::Up,
            Self::Function(number) => Key::Function(number),
            Self::Character(' ') => Key::Text(" ".to_owned()),
            Self::Character(character) if mods.cmd => Key::Command(character.to_ascii_lowercase()),
            Self::Character(character) => Key::Text(character.to_string()),
        }
    }
}

pub(super) fn mods(held: &[Modifier], altgr_shifts_level: bool) -> Mods {
    let holds = |modifier: Modifier| held.contains(&modifier);
    Mods {
        shift: holds(Modifier::Shift),
        cmd: holds(Modifier::Ctrl),
        alt: holds(Modifier::Alt) || (holds(Modifier::AltGr) && !altgr_shifts_level),
        ctrl: holds(Modifier::Ctrl),
        logo: false,
    }
}

pub(super) fn event(key: Named, us: Option<char>, mods: Mods) -> KeyEvent {
    let key = match (key, us) {
        (Named::Character(typed), Some(us)) if mods.cmd && !typed.is_ascii() => {
            Named::Character(us)
        }
        (key, _) => key,
    };
    KeyEvent {
        key: key.key(mods),
        mods,
    }
}

#[cfg(test)]
mod tests;

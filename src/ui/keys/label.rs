// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType as Modifiers};

use super::Binding;

/// What a shortcut row or hint shows. Never free text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keys {
    /// One binding: **Ctrl+C**.
    One(Binding),
    /// Alternatives: **F2 / Ctrl+R**.
    Any(&'static [Binding]),
    /// A sequence: **g h**.
    Chord(&'static [Binding]),
    /// A digit range: **Ctrl+Shift+1–9 / 0**, **t 1–9 / t 0**.
    Digits(Digits),
    /// Alternatives that mix the shapes above: **g g / Home**.
    Mixed(&'static [Keys]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Digits {
    prefix: Option<Binding>,
    then: Option<Binding>,
    modifiers: Modifiers,
    ignored: Modifiers,
    last: u8,
    zero: bool,
}

impl Digits {
    pub(crate) const fn new(modifiers: Modifiers, last: u8, zero: bool) -> Self {
        Self {
            prefix: None,
            then: None,
            modifiers,
            ignored: Modifiers::empty(),
            last,
            zero,
        }
    }

    /// A chord's digit step. It also accepts Shift, which AZERTY needs to type
    /// digits and which no chord step uses.
    pub(crate) const fn after(prefix: Binding, last: u8, zero: bool) -> Self {
        Self {
            prefix: Some(prefix),
            then: None,
            modifiers: Modifiers::empty(),
            ignored: Modifiers::SHIFT_MASK,
            last,
            zero,
        }
    }

    pub(crate) const fn after_chord(prefix: Binding, then: Binding, last: u8, zero: bool) -> Self {
        Self {
            then: Some(then),
            ..Self::after(prefix, last, zero)
        }
    }

    pub(crate) const fn modifiers(self) -> Modifiers {
        self.modifiers
    }

    pub(crate) const fn ignored(self) -> Modifiers {
        self.ignored
    }

    /// The digits alone, as a panel or badge lists them while the prefix or
    /// modifiers are held. Handlers match the full range.
    pub(crate) const fn step(self) -> Self {
        Self {
            prefix: None,
            then: None,
            modifiers: Modifiers::empty(),
            ..self
        }
    }

    pub(crate) fn contains(self, digit: u8) -> bool {
        (1..=self.last).contains(&digit) || (digit == 0 && self.zero)
    }
}

pub(crate) fn label(keys: &Keys) -> String {
    label_in(&rust_i18n::locale(), keys)
}

/// Searchable text: the label in the interface language and in English, so
/// either spelling finds a shortcut, with shared modifiers also written out
/// so that **Ctrl+0** finds **Ctrl++ / − / 0**.
pub(crate) fn search_text(keys: &Keys) -> String {
    let local = rust_i18n::locale();
    let mut texts = Vec::new();
    for locale in [&*local, "en"] {
        for text in [label_in(locale, keys), written_out(locale, keys)] {
            if !texts.contains(&text) {
                texts.push(text);
            }
        }
    }
    texts.join(" ")
}

/// The label with each alternative's modifiers written out.
fn written_out(locale: &str, keys: &Keys) -> String {
    let alternatives = match keys {
        Keys::Any(bindings) => bindings
            .iter()
            .map(|binding| binding_label(locale, *binding))
            .collect::<Vec<_>>(),
        Keys::Mixed(keys) => keys.iter().map(|keys| written_out(locale, keys)).collect(),
        Keys::One(_) | Keys::Chord(_) | Keys::Digits(_) => return label_in(locale, keys),
    };
    alternatives.join(" / ")
}

/// Held modifiers alone, such as **Ctrl+Shift** in "hold Ctrl+Shift for numbers".
pub(crate) fn modifiers_label(modifiers: Modifiers) -> String {
    modifiers_label_in(&rust_i18n::locale(), modifiers)
}

pub(crate) fn modifiers_label_in(locale: &str, modifiers: Modifiers) -> String {
    modifier_names(locale, modifiers).join("+")
}

/// One digit of `digits`, for a hint that names a single member: **g 3**,
/// **Ctrl+Shift+3**, or **3** from [`Digits::step`].
pub(crate) fn digit_label(digits: Digits, digit: u8) -> String {
    debug_assert!(digits.contains(digit), "{digit} is outside {digits:?}");
    let locale = rust_i18n::locale();
    let digit = digit.to_string();
    match prefix_label(&locale, &digits) {
        Some(prefix) => format!("{prefix} {digit}"),
        None => with_modifiers(&locale, digits.modifiers, &digit),
    }
}

pub(crate) fn label_in(locale: &str, keys: &Keys) -> String {
    match keys {
        Keys::One(binding) => binding_label(locale, *binding),
        Keys::Any(bindings) => alternatives(locale, bindings),
        Keys::Chord(bindings) => bindings
            .iter()
            .map(|binding| binding_label(locale, *binding))
            .collect::<Vec<_>>()
            .join(" "),
        Keys::Digits(digits) => digits_label(locale, digits),
        Keys::Mixed(keys) => keys
            .iter()
            .map(|keys| label_in(locale, keys))
            .collect::<Vec<_>>()
            .join(" / "),
    }
}

/// Alternatives that share their modifiers write them once (**Ctrl+Alt+← / →**),
/// unless a key is a letter, which alone would read as a plain key.
fn alternatives(locale: &str, bindings: &[Binding]) -> String {
    let Some(first) = bindings.first() else {
        return String::new();
    };
    let shared = !first.modifiers().is_empty()
        && bindings.iter().all(|binding| {
            binding.modifiers() == first.modifiers() && letter(binding.key()).is_none()
        });
    if shared {
        let keys = bindings
            .iter()
            .map(|binding| key_name(locale, binding.key()))
            .collect::<Vec<_>>()
            .join(" / ");
        return with_modifiers(locale, first.modifiers(), &keys);
    }
    bindings
        .iter()
        .map(|binding| binding_label(locale, *binding))
        .collect::<Vec<_>>()
        .join(" / ")
}

fn digits_label(locale: &str, digits: &Digits) -> String {
    let range = format!("1\u{2013}{}", digits.last);
    match prefix_label(locale, digits) {
        Some(prefix) => {
            if digits.zero {
                format!("{prefix} {range} / {prefix} 0")
            } else {
                format!("{prefix} {range}")
            }
        }
        None => {
            let keys = if digits.zero {
                format!("{range} / 0")
            } else {
                range
            };
            with_modifiers(locale, digits.modifiers, &keys)
        }
    }
}

fn prefix_label(locale: &str, digits: &Digits) -> Option<String> {
    let prefix = binding_label(locale, digits.prefix?);
    Some(match digits.then {
        Some(then) => format!("{prefix} {}", binding_label(locale, then)),
        None => prefix,
    })
}

fn binding_label(locale: &str, binding: Binding) -> String {
    let modifiers = binding.modifiers();
    if let Some(letter) = letter(binding.key()) {
        // Without other modifiers a letter appears as typed: j, and J for Shift+j.
        if modifiers.is_empty() {
            return letter.to_string();
        }
        if modifiers == Modifiers::SHIFT_MASK {
            return letter.to_ascii_uppercase().to_string();
        }
        return with_modifiers(locale, modifiers, &letter.to_ascii_uppercase().to_string());
    }
    with_modifiers(locale, modifiers, &key_name(locale, binding.key()))
}

fn with_modifiers(locale: &str, modifiers: Modifiers, keys: &str) -> String {
    let mut parts = modifier_names(locale, modifiers);
    parts.push(keys.to_owned());
    parts.join("+")
}

fn modifier_names(locale: &str, modifiers: Modifiers) -> Vec<String> {
    [
        (Modifiers::CONTROL_MASK, "keys.name.ctrl"),
        (Modifiers::ALT_MASK, "keys.name.alt"),
        (Modifiers::SHIFT_MASK, "keys.name.shift"),
        (Modifiers::SUPER_MASK, "keys.name.super"),
    ]
    .into_iter()
    .filter(|(modifier, _)| modifiers.contains(*modifier))
    .map(|(_, name)| rust_i18n::t!(name, locale = locale).into_owned())
    .collect()
}

fn letter(key: Key) -> Option<char> {
    key.to_unicode()
        .filter(|character| character.is_ascii_lowercase())
}

fn key_name(locale: &str, key: Key) -> String {
    let name = match key {
        Key::Up => return "↑".to_owned(),
        Key::Down => return "↓".to_owned(),
        Key::Left => return "←".to_owned(),
        Key::Right => return "→".to_owned(),
        Key::minus => return "\u{2212}".to_owned(),
        Key::Return => "keys.name.enter",
        Key::Escape => "keys.name.escape",
        Key::Delete => "keys.name.delete",
        Key::BackSpace => "keys.name.backspace",
        Key::Tab => "keys.name.tab",
        Key::space => "keys.name.space",
        Key::Page_Up => "keys.name.page_up",
        Key::Page_Down => "keys.name.page_down",
        Key::Home => "keys.name.home",
        Key::End => "keys.name.end",
        Key::Insert => "keys.name.insert",
        Key::Menu => "keys.name.menu",
        _ => {
            return key
                .to_unicode()
                .filter(|character| character.is_ascii_graphic())
                .map(|character| character.to_ascii_uppercase().to_string())
                .or_else(|| key.name().map(Into::into))
                .unwrap_or_default();
        }
    };
    rust_i18n::t!(name, locale = locale).into_owned()
}

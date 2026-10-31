// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType as Modifiers};

use super::{Binding, Command, Map, command::Alternate};

/// A key a label shows: a command, which shows its bindings in the active
/// map, one alternate of a command, or a fixed binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    Command(Command),
    Alternate(Alternate),
    Binding(Binding),
}

impl Shown {
    /// Every alternate in `map` that can fire, primary first.
    pub(crate) fn bindings(self, map: Map) -> Vec<Binding> {
        match self {
            Self::Command(command) => firing(command, map, command.bindings_in(map)),
            Self::Alternate(alternate) => {
                firing(alternate.command(), map, alternate.bindings_in(map))
            }
            Self::Binding(binding) => vec![binding],
        }
    }

    fn command(self) -> Option<Command> {
        match self {
            Self::Command(command) => Some(command),
            Self::Alternate(alternate) => Some(alternate.command()),
            Self::Binding(_) => None,
        }
    }

    /// The binding a menu shows, from the active map.
    pub(crate) fn primary(self) -> Option<Binding> {
        self.primary_in(Map::active())
    }

    fn primary_in(self, map: Map) -> Option<Binding> {
        self.bindings(map).first().copied()
    }
}

/// Leaves out keys that never fire, so a label never shows one: a key no
/// press reaches, and a plain printable key a user added to the default map
/// while type-to-search types it.
fn firing(command: Command, map: Map, bindings: Vec<Binding>) -> Vec<Binding> {
    let typing = map == Map::Default
        && crate::ui::preferences::PreferenceManager::existing()
            .is_some_and(|preferences| preferences.type_to_search_active());
    bindings
        .into_iter()
        .filter(|binding| !super::conflicts::never_fires(map, command, *binding, typing))
        .collect()
}

/// The [`Shown`] keys of catalog constants, commands and fixed bindings alike,
/// for [`Keys::Any`] and [`Keys::Chord`].
macro_rules! shown {
    ($($key:expr),* $(,)?) => { &const { [$($key.shown()),*] } };
}
pub(crate) use shown;

/// What a shortcut row or hint shows. Never free text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keys {
    /// One key, with every alternate a command has: **Ctrl+C**.
    One(Shown),
    /// Alternatives: **F2 / Ctrl+R**.
    Any(&'static [Shown]),
    /// A sequence of primary bindings: **g h**.
    Chord(&'static [Shown]),
    /// A digit range: **Ctrl+Shift+1–9 / 0**, **t 1–9 / t 0**.
    Digits(Digits),
    /// Alternatives that mix the shapes above: **g g / Home**.
    Mixed(&'static [Keys]),
}

impl Keys {
    /// Whether the keys resolve to the same bindings in both maps.
    pub(crate) fn same_in_both_maps(&self) -> bool {
        self.resolved(Map::Default) == self.resolved(Map::Tenxer)
    }

    /// Whether the active map binds what the label needs: any alternative, and
    /// every step of a chord. An unbound row is left out rather than shown blank.
    pub(crate) fn is_bound(&self) -> bool {
        !label_in_map("en", self, Map::active()).is_empty()
    }

    /// The commands the keys show, in order, chord triggers and digit
    /// prefixes included.
    pub(crate) fn commands(&self) -> Vec<Command> {
        match self {
            Self::One(shown) => shown.command().into_iter().collect(),
            Self::Any(shown) | Self::Chord(shown) => {
                shown.iter().filter_map(|shown| shown.command()).collect()
            }
            Self::Digits(digits) => digits.prefix.and_then(Shown::command).into_iter().collect(),
            Self::Mixed(keys) => keys.iter().flat_map(Self::commands).collect(),
        }
    }

    fn resolved(&self, map: Map) -> Vec<Binding> {
        match self {
            Self::One(shown) => shown.bindings(map),
            Self::Any(shown) | Self::Chord(shown) => expanded(shown, map),
            Self::Digits(digits) => digits
                .prefix
                .map(|prefix| prefix.bindings(map))
                .unwrap_or_default(),
            Self::Mixed(keys) => keys.iter().flat_map(|keys| keys.resolved(map)).collect(),
        }
    }
}

/// Digit-row keys 1 to `last`, and 0 when `zero`, with shared modifiers or
/// after a chord prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Digits {
    prefix: Option<Shown>,
    modifiers: Modifiers,
    ignored: Modifiers,
    last: u8,
    zero: bool,
}

impl Digits {
    pub(crate) const fn new(modifiers: Modifiers, last: u8, zero: bool) -> Self {
        Self {
            prefix: None,
            modifiers,
            ignored: Modifiers::empty(),
            last,
            zero,
        }
    }

    /// A chord's digit step. It also accepts Shift, which AZERTY needs to type
    /// digits and which no chord step uses.
    pub(crate) const fn after(prefix: Shown, last: u8, zero: bool) -> Self {
        Self {
            prefix: Some(prefix),
            modifiers: Modifiers::empty(),
            ignored: Modifiers::SHIFT_MASK,
            last,
            zero,
        }
    }

    #[cfg(test)]
    pub(crate) const fn prefix(self) -> Option<Shown> {
        self.prefix
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
    let map = Map::active();
    let mut texts = Vec::new();
    for locale in [&*local, "en"] {
        for text in [
            label_in_map(locale, keys, map),
            written_out(locale, keys, map),
        ] {
            if !texts.contains(&text) {
                texts.push(text);
            }
        }
    }
    texts.join(" ")
}

/// The label with each alternative's modifiers written out.
fn written_out(locale: &str, keys: &Keys, map: Map) -> String {
    let alternatives = match keys {
        Keys::One(shown) => labels(locale, &shown.bindings(map)),
        Keys::Any(shown) => labels(locale, &expanded(shown, map)),
        Keys::Mixed(keys) => keys
            .iter()
            .map(|keys| written_out(locale, keys, map))
            .collect(),
        Keys::Chord(_) | Keys::Digits(_) => return label_in_map(locale, keys, map),
    };
    alternatives.join(" / ")
}

fn labels(locale: &str, bindings: &[Binding]) -> Vec<String> {
    bindings
        .iter()
        .map(|binding| binding_label(locale, *binding))
        .collect()
}

fn expanded(shown: &[Shown], map: Map) -> Vec<Binding> {
    shown.iter().flat_map(|shown| shown.bindings(map)).collect()
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
    match digits.prefix {
        Some(prefix) => primary_label(&locale, prefix, Map::active())
            .map(|prefix| format!("{prefix} {digit}"))
            .unwrap_or_default(),
        None => with_modifiers(&locale, digits.modifiers, &digit),
    }
}

pub(crate) fn label_in(locale: &str, keys: &Keys) -> String {
    label_in_map(locale, keys, Map::active())
}

/// Empty when nothing it shows is bound in `map`, as for a chord with an
/// unbound step.
pub(crate) fn label_in_map(locale: &str, keys: &Keys, map: Map) -> String {
    match keys {
        Keys::One(shown) => alternatives(locale, &shown.bindings(map)),
        Keys::Any(shown) => alternatives(locale, &expanded(shown, map)),
        Keys::Chord(steps) => steps
            .iter()
            .map(|step| primary_label(locale, *step, map))
            .collect::<Option<Vec<_>>>()
            .map(|steps| steps.join(" "))
            .unwrap_or_default(),
        Keys::Digits(digits) => digits_label(locale, digits, map).unwrap_or_default(),
        Keys::Mixed(keys) => keys
            .iter()
            .map(|keys| label_in_map(locale, keys, map))
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>()
            .join(" / "),
    }
}

fn primary_label(locale: &str, shown: Shown, map: Map) -> Option<String> {
    shown
        .primary_in(map)
        .map(|binding| binding_label(locale, binding))
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

fn digits_label(locale: &str, digits: &Digits, map: Map) -> Option<String> {
    let range = format!("1\u{2013}{}", digits.last);
    Some(match digits.prefix {
        Some(prefix) => {
            let prefix = primary_label(locale, prefix, map)?;
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
            return uppercase(letter);
        }
        return with_modifiers(locale, modifiers, &uppercase(letter));
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
        .filter(|character| character.is_lowercase())
}

fn uppercase(letter: char) -> String {
    let mut upper = letter.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single.to_string(),
        _ => letter.to_string(),
    }
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
        Key::Multi_key => "keys.name.compose",
        _ => {
            return key
                .to_unicode()
                .or_else(|| dead_key_mark(key))
                .filter(|character| !character.is_control() && !character.is_whitespace())
                .map(|character| character.to_string())
                .or_else(|| key.name().map(Into::into))
                .unwrap_or_default();
        }
    };
    rust_i18n::t!(name, locale = locale).into_owned()
}

/// The accent a dead key adds, as its keycap shows it.
fn dead_key_mark(key: Key) -> Option<char> {
    Some(match key {
        Key::dead_grave => '`',
        Key::dead_acute => '´',
        Key::dead_circumflex => '^',
        Key::dead_tilde => '~',
        Key::dead_macron => '¯',
        Key::dead_breve => '˘',
        Key::dead_abovedot => '˙',
        Key::dead_diaeresis => '¨',
        Key::dead_abovering => '˚',
        Key::dead_doubleacute => '˝',
        Key::dead_caron => 'ˇ',
        Key::dead_cedilla => '¸',
        Key::dead_ogonek => '˛',
        _ => return None,
    })
}

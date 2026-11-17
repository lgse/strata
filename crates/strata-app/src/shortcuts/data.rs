// SPDX-License-Identifier: MIT

//! The reference's rows as C19 takes them, from the port in `contents.rs`, and the keys its
//! panel answers, from the catalog: GTK's `Shortcut::key_text`, `action_text` and `matches`
//! (`src/ui/shortcut_reference.rs`) and `reference_note` (`src/ui/shortcut_footer/catalog.rs`).

use rune::{Chord, ChordKey, ChordMod};
use strata_components::footer::reference::{
    self as reference, Context, ReferenceKeys, ReferenceLabels, ShortcutData,
};
use strata_core::i18n::{integer, tr};
use strata_desktop::keys::{
    self, Binding, Command, Key, Keys, Modifiers, Shown,
    bindings::{
        DOWN, ESCAPE, FOCUS_NEXT, REFERENCE, REFERENCE_CATEGORIES, REFERENCE_LIST,
        REFERENCE_SEARCH, UP,
    },
    shown,
};

use super::contents::{self, Mention, Mode, Shortcut};

/// What the reference shows: the sections of `mode`, labelled through the keymap in force.
pub(super) fn rows(mode: Mode, tenxer: bool, revision: u64) -> ShortcutData {
    let sections = contents::sections(mode, tenxer)
        .into_iter()
        .map(|section| reference::Section {
            title: tr(section.title),
            rows: section.rows.iter().map(row).collect(),
        })
        .collect();
    ShortcutData { sections, revision }
}

fn row(shortcut: &Shortcut) -> reference::Shortcut {
    let keys = caps(&shortcut.keys);
    let mut row = reference::Shortcut::new(action_in(shortcut, &rust_i18n::locale()), keys);
    if let Some(template) = shortcut.context {
        let (before, after) = around_keys(template);
        row.context = Some(Context { before, after });
    }
    // GTK's `matches` also finds a row by its keys written out and by its English action.
    row.search(
        [keys::search_text(&shortcut.keys), action_in(shortcut, "en")]
            .into_iter()
            .filter(|term| !term.is_empty()),
    )
}

/// `Shortcut::action_in`.
fn action_in(shortcut: &Shortcut, locale: &str) -> String {
    match shortcut.mention {
        Some(mention) => rust_i18n::t!(
            shortcut.action,
            locale = locale,
            keys = mentioned(mention, locale)
        )
        .into_owned(),
        None => match shortcut.number {
            Some(number) => {
                rust_i18n::t!(shortcut.action, locale = locale, number = number).into_owned()
            }
            None => rust_i18n::t!(shortcut.action, locale = locale).into_owned(),
        },
    }
}

/// `Mention::label_in`.
fn mentioned(mention: Mention, locale: &str) -> String {
    match mention {
        Mention::Keys(keys) => keys::label_in(locale, &keys),
        Mention::Held(modifiers) => keys::modifiers_label_in(locale, modifiers),
    }
}

/// The words a context template puts before and after its keys: "%{keys} at first pane"
/// reads "", " at first pane". GTK fills the template whole (`key_text`), so the words are
/// those of the filled template on either side of the keys.
fn around_keys(template: &str) -> (String, String) {
    const MARK: &str = "\u{1}";
    let filled = rust_i18n::t!(template, keys = MARK).into_owned();
    match filled.split_once(MARK) {
        Some((before, after)) => (before.to_owned(), after.to_owned()),
        None => (String::new(), filled),
    }
}

/// The keys as keycaps: one per alternative, and one per step of a chord. Their text, the
/// alternatives a " / " apart and the steps a space apart, is GTK's label of the keys as it
/// is: "Ctrl+Alt+← / →" draws as "Ctrl+Alt+←" and "→".
fn caps(keys: &Keys) -> reference::Keys {
    let alternatives = match keys {
        Keys::Mixed(each) => each.iter().flat_map(|keys| caps(keys).0).collect(),
        Keys::Chord(_) => split(&keys::label(keys), " / ")
            .map(|alternative| alternative.split(' ').map(str::to_owned).collect())
            .collect(),
        _ => split(&keys::label(keys), " / ")
            .map(|alternative| vec![alternative.to_owned()])
            .collect(),
    };
    reference::Keys(alternatives)
}

fn split<'a>(label: &'a str, separator: &'a str) -> impl Iterator<Item = &'a str> {
    label.split(separator).filter(|part| !part.is_empty())
}

/// The panel's words. GTK names neither its categories nor its results, and shows each
/// category's row count as a number.
pub(super) fn labels() -> ReferenceLabels {
    ReferenceLabels {
        title: tr("Keyboard shortcuts"),
        search: tr("Search actions or keys…"),
        categories: String::new(),
        // GTK's first category is "All" in every language.
        all: "All".to_owned(),
        results: String::new(),
        no_match: tr("No matching shortcuts"),
        count: std::rc::Rc::new(|count| integer(count as u64)),
        note: note(),
    }
}

/// `reference_note`, hint by hint.
fn note() -> Vec<String> {
    let label = |keys| keys::label(&keys);
    rust_i18n::t!(
        "%{category_keys} categories · %{search_keys} search · %{list_keys} list · %{move_keys} move · %{cycle_keys} cycle · %{close_keys} close",
        category_keys = label(Keys::One(REFERENCE_CATEGORIES.shown())),
        search_keys = label(Keys::One(REFERENCE_SEARCH.shown())),
        list_keys = label(Keys::One(REFERENCE_LIST.shown())),
        move_keys = label(Keys::Any(shown![
            DOWN.alternate(1),
            UP.alternate(1),
            UP.alternate(0),
            DOWN.alternate(0)
        ])),
        cycle_keys = label(Keys::One(FOCUS_NEXT.shown())),
        close_keys = label(Keys::One(ESCAPE.shown()))
    )
    .split(" · ")
    .map(str::to_owned)
    .collect()
}

/// The keys the open panel answers, as the keymap binds them now: F1 closes it, and the
/// three area keys move among its search, categories and results.
/// The panel's keys, as the keymap binds them; 10xer mode's `~` toggles it however the layout
/// types it (C19's `ReferenceKeys::tenxer`).
pub(super) fn panel_keys(tenxer: bool) -> ReferenceKeys {
    let mut toggle = chords(REFERENCE);
    if tenxer {
        for tilde in ReferenceKeys::tenxer().toggle {
            if !toggle.contains(&tilde) {
                toggle.push(tilde);
            }
        }
    }
    ReferenceKeys {
        toggle,
        categories: chords(REFERENCE_CATEGORIES),
        search: chords(REFERENCE_SEARCH),
        list: chords(REFERENCE_LIST),
    }
}

fn chords(command: Command) -> Vec<Chord> {
    Shown::Command(command)
        .bindings(keys::Map::active())
        .into_iter()
        .filter_map(chord)
        .collect()
}

/// A binding as Rune's chord, for a key Rune names: a function key, a named key or a key
/// that types a character.
fn chord(binding: Binding) -> Option<Chord> {
    let key = binding.key();
    let named = [
        (Key::Return, ChordKey::Enter),
        (Key::Tab, ChordKey::Tab),
        (Key::Escape, ChordKey::Escape),
        (Key::BackSpace, ChordKey::Backspace),
        (Key::Delete, ChordKey::Delete),
        (Key::Insert, ChordKey::Insert),
        (Key::Home, ChordKey::Home),
        (Key::End, ChordKey::End),
        (Key::Page_Up, ChordKey::PageUp),
        (Key::Page_Down, ChordKey::PageDown),
        (Key::Left, ChordKey::Left),
        (Key::Right, ChordKey::Right),
        (Key::Up, ChordKey::Up),
        (Key::Down, ChordKey::Down),
        (Key::Menu, ChordKey::ContextMenu),
    ];
    let function = key
        .keysym()
        .checked_sub(Key::F1.keysym())
        .filter(|n| *n < 24);
    let chord_key = if let Some(n) = function {
        ChordKey::Function(u8::try_from(n + 1).ok()?)
    } else if let Some((_, named)) = named.iter().find(|(named, _)| *named == key) {
        *named
    } else {
        ChordKey::Char(key.to_unicode()?)
    };
    let modifiers = binding.modifiers();
    let held = [
        (Modifiers::CONTROL_MASK, ChordMod::Ctrl),
        (Modifiers::ALT_MASK, ChordMod::Alt),
        (Modifiers::SHIFT_MASK, ChordMod::Shift),
        (Modifiers::SUPER_MASK, ChordMod::Super),
    ];
    Some(
        held.into_iter()
            .filter(|(mask, _)| modifiers.contains(*mask))
            .fold(Chord::new(chord_key), |chord, (_, held)| chord.with(held)),
    )
}

#[cfg(test)]
mod tests;

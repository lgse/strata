// SPDX-License-Identifier: MIT

use rune::{Chord, ChordKey, ChordMod};
use strata_desktop::keys::{
    Binding, Key, Keymap, Keys, Map,
    bindings::{MEDIA_SEEK_BACK, MEDIA_SEEK_FORWARD, REFERENCE, REFERENCE_CATEGORIES, RENAME},
    shown, use_keymap,
};

use super::{around_keys, caps, chord, panel_keys, rows};
use crate::shortcuts::contents::Mode;

fn library() {
    crate::startup::install_key_library().expect("the key library");
}

#[test]
fn keycaps_read_as_gtk_labels_the_keys() {
    library();
    let seek = Keys::Any(shown![MEDIA_SEEK_BACK, MEDIA_SEEK_FORWARD]);
    let caps = caps(&seek);
    assert_eq!(caps.0, [vec!["Ctrl+Alt+←"], vec!["→"]]);
    assert_eq!(caps.text(), strata_desktop::keys::label(&seek));
}

#[test]
fn a_context_puts_its_words_around_the_keys() {
    assert_eq!(
        around_keys("%{keys} at first pane"),
        (String::new(), " at first pane".to_owned())
    );
}

#[test]
fn a_saved_keymap_moves_a_rows_keys() {
    library();
    let keymap = Keymap::defaults().with(Map::Default, RENAME, vec![Binding::ctrl(Key::e)]);
    let _keymap = use_keymap(keymap);
    let data = rows(Mode::Columns, 1);
    let rename = data
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .find(|row| row.action == "Rename")
        .expect("a Rename row");
    assert_eq!(rename.key_text(), "Ctrl+E");
}

#[test]
fn the_panel_answers_the_keymaps_keys() {
    library();
    let keymap = Keymap::defaults().with(
        Map::Default,
        REFERENCE_CATEGORIES,
        vec![Binding::ctrl(Key::g)],
    );
    let _keymap = use_keymap(keymap);
    let keys = panel_keys();
    assert_eq!(keys.toggle, [Chord::new(ChordKey::Function(1))]);
    assert_eq!(
        keys.categories,
        [Chord::new(ChordKey::Char('g')).with(ChordMod::Ctrl)]
    );
    assert_eq!(
        chord(REFERENCE.bindings_in(Map::Default)[0]),
        Some(Chord::new(ChordKey::Function(1)))
    );
}

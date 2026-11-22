// SPDX-License-Identifier: MIT

use rune::{Chord, ChordKey, ChordMod};
use strata_desktop::keys::{
    Binding, CUSTOM_ACTIONS, Key, Keymap, Keys, Map,
    bindings::{MEDIA_SEEK_BACK, MEDIA_SEEK_FORWARD, REFERENCE, REFERENCE_CATEGORIES, RENAME},
    shown, use_keymap,
};

use super::{around_keys, caps, chord, labels, panel_keys, rows};
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
    assert_eq!(
        around_keys("%{keys} in %{go} / %{move_to} / %{copy_to}"),
        (
            String::new(),
            " in go \u{203a} / move to \u{203a} / copy to \u{203a}".to_owned()
        )
    );
    assert_eq!(
        around_keys("%{keys} in %{jump} / %{recent}"),
        (
            String::new(),
            " in jump \u{203a} / recent \u{203a}".to_owned()
        )
    );
}

fn assert_no_placeholder(context: &str, text: &str) {
    assert!(!text.contains("%{"), "uninterpolated {context}: {text:?}");
}

fn assert_rows_have_no_placeholders(mode: Mode, tenxer: bool) {
    let data = rows(mode, tenxer, 1);
    assert!(!data.sections.is_empty());
    for section in &data.sections {
        assert_no_placeholder("section title", &section.title);
        for shortcut in &section.rows {
            let tag = format!("{mode:?} (tenxer={tenxer})");
            assert_no_placeholder(&format!("action in {tag}"), &shortcut.action);
            assert_no_placeholder(&format!("key_text in {tag}"), &shortcut.key_text());
            for term in &shortcut.search {
                assert_no_placeholder(&format!("search term in {tag}"), term);
            }
        }
    }
}

#[test]
fn no_reference_text_leaves_uninterpolated_placeholders() {
    library();
    let keymap = Keymap::defaults().with(
        Map::Default,
        CUSTOM_ACTIONS[0],
        vec![Binding::ctrl_alt(Key::k)],
    );
    let _keymap = use_keymap(keymap);

    let panel_labels = labels();
    for text in [
        &panel_labels.title,
        &panel_labels.search,
        &panel_labels.all,
        &panel_labels.no_match,
    ]
    .into_iter()
    .chain(&panel_labels.note)
    {
        assert_no_placeholder("label", text);
    }

    for tenxer in [false, true] {
        for mode in [Mode::Columns, Mode::Icons, Mode::List] {
            assert_rows_have_no_placeholders(mode, tenxer);
        }
    }
}

#[test]
fn a_saved_keymap_moves_a_rows_keys() {
    library();
    let keymap = Keymap::defaults().with(Map::Default, RENAME, vec![Binding::ctrl(Key::e)]);
    let _keymap = use_keymap(keymap);
    let data = rows(Mode::Columns, false, 1);
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
    let keys = panel_keys(false);
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

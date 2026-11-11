//! Unit tests for the reference: the search, its lines, the state, the keys and the key map
//! rows (`app/shortcut-reference`, `app/keyboard-shortcuts` Labels).

use std::rc::Rc;

use rune::{Chord, Key, KeyEvent, Mods};

use super::data::{Keys, Section, Shortcut, ShortcutData};
use super::keys::edits;
use super::search::{Index, Line, fold, fold_query, lines};
use super::state::{ReferenceKeys, ReferenceState};
use super::{chord_label, keymap_rows, note};
use crate::collection::keyboard::{Command, KeyMap, Letters, Target};
use crate::contract::Direction;

fn data() -> ShortcutData {
    ShortcutData {
        sections: vec![
            Section {
                title: "Files and selection".to_owned(),
                rows: vec![
                    Shortcut::new("Copy", Keys::one("Ctrl+C")),
                    Shortcut::new("Rename the next item", Keys::any(["Tab", "Shift+Tab"]))
                        .context("", " in a rename"),
                    Shortcut::new("Kopieren", Keys::one("Strg+C")).search(["copy", "ctrl+c"]),
                ],
            },
            Section {
                title: "Search and tools".to_owned(),
                rows: vec![
                    Shortcut::new("Text size", Keys::any(["Ctrl++", "\u{2212}", "0"]))
                        .search(["ctrl+0"]),
                    Shortcut::new("Go to tab", Keys::one("Ctrl+Shift+1\u{2013}9")),
                ],
            },
        ],
        revision: 1,
    }
}

fn shown(d: &ShortcutData, query: &str, category: Option<usize>) -> Vec<String> {
    let index = Index::build(d);
    let l = lines(&d.sections, &index, &fold_query(query), category, 1);
    l.lines
        .iter()
        .filter_map(|line| match line {
            Line::Rows { section, first, .. } => {
                Some(d.sections[*section].rows[*first].action.clone())
            }
            Line::Heading { .. } => None,
        })
        .collect()
}

#[test]
fn a_row_stays_when_its_action_keys_context_or_terms_contain_the_query_in_any_case() {
    let d = data();
    assert_eq!(shown(&d, "COPY", None), ["Copy", "Kopieren"]);
    assert_eq!(shown(&d, "in a rename", None), ["Rename the next item"]);
    assert_eq!(shown(&d, "shift+tab", None), ["Rename the next item"]);
    assert_eq!(shown(&d, "strg+c", None), ["Kopieren"]);
    assert_eq!(shown(&d, "ctrl+c", None), ["Copy", "Kopieren"]);
    assert!(shown(&d, "zzz", None).is_empty());
    assert_eq!(
        shown(&d, "  ", None).len(),
        5,
        "a blank query keeps every row"
    );
}

#[test]
fn minus_and_en_dash_read_as_a_typed_hyphen() {
    let d = data();
    assert_eq!(shown(&d, "ctrl+0", None), ["Text size"]);
    assert_eq!(shown(&d, "/ -", None), ["Text size"]);
    assert_eq!(shown(&d, "1-9", None), ["Go to tab"]);
}

#[test]
fn a_category_shows_only_its_section() {
    let d = data();
    assert_eq!(shown(&d, "", Some(1)), ["Text size", "Go to tab"]);
    assert!(shown(&d, "copy", Some(1)).is_empty());
}

#[test]
fn lines_pair_rows_by_columns_and_number_every_row_shown() {
    let d = data();
    let index = Index::build(&d);
    let l = lines(&d.sections, &index, "", None, 2);
    assert_eq!(
        l.lines,
        [
            Line::Heading {
                section: 0,
                shown: 3
            },
            Line::Rows {
                section: 0,
                first: 0,
                second: Some(1)
            },
            Line::Rows {
                section: 0,
                first: 2,
                second: None
            },
            Line::Heading {
                section: 1,
                shown: 2
            },
            Line::Rows {
                section: 1,
                first: 0,
                second: Some(1)
            },
        ]
    );
    assert_eq!(l.matches, 5);
    assert_eq!(l.ordinal[1][1], Some(4));
    let l = lines(&d.sections, &index, "size", None, 2);
    assert_eq!(
        l.ordinal[1],
        [Some(0), None],
        "numbered among the rows shown"
    );
    assert_eq!(l.matches, 1);
}

#[test]
fn a_match_maps_back_onto_the_shown_text() {
    let text = "Größe ändern";
    let f = fold(text);
    let ranges = f.ranges(text, &fold_query("ÄND"));
    assert_eq!(&text[ranges[0].clone()], "änd");
    let keys = "Ctrl+\u{2212}";
    let f = fold(keys);
    let r = f.ranges(keys, "+-");
    assert_eq!(&keys[r[0].clone()], "+\u{2212}");
    assert!(f.ranges(keys, "").is_empty());
}

#[test]
fn the_state_keeps_its_query_and_category_across_closing() {
    let mut s = ReferenceState::new();
    s.toggle();
    assert!(s.is_open());
    s.set_query("copy");
    s.set_category(Some("Search and tools".to_owned()));
    s.toggle();
    assert!(!s.is_open());
    s.open();
    assert_eq!(s.query(), "copy");
    assert_eq!(s.category().as_deref(), Some("Search and tools"));
    assert_eq!(s.section(&data()), Some(1));
    s.set_category(Some("Gone".to_owned()));
    assert_eq!(
        s.section(&data()),
        None,
        "a category the data lost shows All"
    );
}

#[test]
fn the_lines_are_cached_until_the_query_category_or_revision_changes() {
    let mut s = ReferenceState::new();
    let mut d = data();
    let a = s.lines(&d, 1);
    assert!(Rc::ptr_eq(&a, &s.lines(&d, 1)));
    s.set_query("copy");
    let b = s.lines(&d, 1);
    assert!(!Rc::ptr_eq(&a, &b));
    d.sections[0].rows.remove(0);
    d.revision += 1;
    let c = s.lines(&d, 1);
    assert_eq!(c.matches, 1, "a new revision is indexed again");
}

fn key(key: Key, mods: Mods) -> KeyEvent {
    KeyEvent { key, mods }
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
}

#[test]
fn toggle_chords_match_exactly_and_tilde_only_where_typed() {
    let keys = ReferenceKeys::tenxer();
    assert!(keys.toggles_typed(&key(Key::Function(1), Mods::default())));
    assert!(!keys.toggles_typed(&key(Key::Function(1), shift())));
    assert!(!keys.toggles_typed(&key(Key::Function(1), ctrl())));
    assert!(keys.toggles_typed(&key(Key::Text("~".into()), shift())));
    assert!(!keys.toggles_typed(&key(Key::Text("\u{401}".into()), shift())));
    assert!(!ReferenceKeys::default().toggles_typed(&key(Key::Text("~".into()), shift())));
}

#[test]
fn the_search_field_keeps_its_editing_keys_and_takes_the_rest() {
    for (k, m) in [
        (Key::Text("a".into()), Mods::default()),
        (Key::Text("A".into()), shift()),
        (Key::Command('a'), ctrl()),
        (Key::Command('c'), ctrl()),
        (Key::Command('x'), ctrl()),
        (
            Key::Command('z'),
            Mods {
                shift: true,
                ..ctrl()
            },
        ),
        (Key::Left, ctrl()),
        (Key::Backspace, ctrl()),
        (Key::Delete, Mods::default()),
        (Key::Tab, shift()),
    ] {
        assert!(edits(&key(k.clone(), m)), "{k:?} {m:?} edits");
    }
    for (k, m) in [
        (Key::Command(','), ctrl()),
        (Key::Command('v'), ctrl()),
        (Key::Command('1'), ctrl()),
        (Key::Command('k'), ctrl()),
        (Key::Text(" ".into()), ctrl()),
        (
            Key::Left,
            Mods {
                alt: true,
                ..Mods::default()
            },
        ),
        (Key::Function(2), Mods::default()),
        (Key::Function(5), Mods::default()),
        (Key::Enter, Mods::default()),
        (Key::ContextMenu, Mods::default()),
    ] {
        assert!(!edits(&key(k.clone(), m)), "{k:?} {m:?} is taken");
    }
}

#[test]
fn the_note_names_the_bound_keys() {
    let mut keys = ReferenceKeys::default();
    assert_eq!(
        note(&keys, chord_label).join(" \u{b7} "),
        "Ctrl+B categories \u{b7} Ctrl+F search \u{b7} Ctrl+L list \u{b7} j / k / \u{2191} / \u{2193} move \u{b7} Tab cycle \u{b7} Escape close"
    );
    keys.categories = vec![Chord::parse("ctrl+g").unwrap()];
    keys.list.clear();
    let n = note(&keys, chord_label);
    assert_eq!(n[0], "Ctrl+G categories");
    assert!(!n.iter().any(|h| h.ends_with(" list")));
}

#[test]
fn chords_read_as_strata_labels_them() {
    let c = |s: &str| chord_label(&Chord::parse(s).unwrap());
    assert_eq!(c("j"), "j");
    assert_eq!(c("shift+j"), "J");
    assert_eq!(c("ctrl+shift+n"), "Ctrl+Shift+N");
    assert_eq!(c("ctrl+alt+shift+super+t"), "Ctrl+Alt+Shift+Super+T");
    assert_eq!(c("shift+backquote"), "~");
    assert_eq!(c("ctrl+minus"), "Ctrl+\u{2212}");
    assert_eq!(c("up"), "\u{2191}");
    assert_eq!(c("shift+left"), "Shift+\u{2190}");
    assert_eq!(c("escape"), "Escape");
    assert_eq!(c("space"), "Space");
    assert_eq!(c("f1"), "F1");
}

fn describe(c: Command) -> Option<String> {
    match c {
        Command::Swallow => None,
        c => Some(format!("{c:?}")),
    }
}

fn row<'a>(rows: &'a [Shortcut], action: &str) -> Option<&'a Shortcut> {
    rows.iter().find(|r| r.action == action)
}

#[test]
fn every_chord_of_a_command_is_listed_and_a_command_without_one_has_no_row() {
    let map = KeyMap::default_map();
    let rows = keymap_rows(&map, Letters::Pass, describe, Some("Same as"), chord_label);
    let first = format!("{:?}", Command::Select(Target::First));
    assert_eq!(
        row(&rows, &first).unwrap().keys.text(),
        "Home / Ctrl+\u{2191}"
    );
    assert_eq!(
        row(&rows, &format!("{:?}", Command::Clear))
            .unwrap()
            .keys
            .text(),
        "Escape",
        "a tolerated chord (Shift+Escape) isn't listed"
    );
    assert!(
        row(&rows, "Same as").is_none(),
        "aliases fire only with Arrows"
    );

    let mut map = map;
    map.unbind(Command::SelectAll);
    let rows = keymap_rows(&map, Letters::Pass, describe, None, chord_label);
    assert!(row(&rows, &format!("{:?}", Command::SelectAll)).is_none());

    let rows = keymap_rows(
        &KeyMap::tenxer(),
        Letters::Pass,
        describe,
        None,
        chord_label,
    );
    assert!(rows.iter().all(|r| !r.action.contains("Swallow")));
    let down = format!("{:?}", Command::Cursor(Target::Step(Direction::Down)));
    assert_eq!(row(&rows, &down).unwrap().keys.text(), "\u{2193} / j");
}

#[test]
fn aliases_make_one_row_in_arrow_order_after_the_first_command_they_stand_for() {
    let map = KeyMap::default_map();
    let rows = keymap_rows(
        &map,
        Letters::Arrows,
        describe,
        Some("Same as"),
        chord_label,
    );
    let at = rows.iter().position(|r| r.action == "Same as").unwrap();
    assert_eq!(rows[at].keys.text(), "h / j / k / l");
    assert_eq!(
        rows[at - 1].action,
        format!("{:?}", Command::Select(Target::Step(Direction::Up)))
    );
    let up = rows
        .iter()
        .find(|r| r.action == rows[at - 1].action)
        .unwrap();
    assert_eq!(
        up.keys.text(),
        "\u{2191}",
        "the alias isn't in its command's row"
    );
    let rows = keymap_rows(&map, Letters::Arrows, describe, None, chord_label);
    assert!(rows.iter().all(|r| r.keys.text() != "h / j / k / l"));
}

#[test]
fn keys_read_as_text_with_steps_and_alternatives() {
    let k = Keys(vec![
        vec!["g".into(), "h".into()],
        vec![],
        vec!["F2".into()],
    ]);
    assert_eq!(k.text(), "g h / F2");
    assert!(Keys::default().is_empty());
    let r = Shortcut::new("x", Keys::one("/")).context("`", "` in the preview");
    assert_eq!(r.key_text(), "`/` in the preview");
}

#[test]
fn shift_is_an_editing_chord_only_with_z_and_alt_space_is_taken() {
    let cs = Mods {
        shift: true,
        ..ctrl()
    };
    assert!(edits(&key(Key::Command('z'), cs)), "Ctrl+Shift+Z redoes");
    for c in ['a', 'c', 'x', 'y', 'v'] {
        assert!(!edits(&key(Key::Command(c), cs)), "Ctrl+Shift+{c} is taken");
    }
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    assert!(
        !edits(&key(Key::Text(" ".into()), alt)),
        "Alt+Space is taken"
    );
}

#[test]
fn a_tilde_typed_without_shift_toggles_in_tenxer() {
    // AltGr types `~` on German and French layouts: no Shift.
    let keys = ReferenceKeys::tenxer();
    assert!(keys.toggles_typed(&key(Key::Text("~".into()), Mods::default())));
    assert!(keys.toggles_typed(&key(Key::Text("~".into()), shift())));
    assert!(!keys.toggles_typed(&key(Key::Text("\u{401}".into()), shift())));
    assert!(!ReferenceKeys::default().toggles_typed(&key(Key::Text("~".into()), Mods::default())));
}

#[test]
fn a_category_the_data_lost_is_forgotten() {
    let mut s = ReferenceState::new();
    let mut d = data();
    s.set_category(Some("Search and tools".to_owned()));
    let gone = d.sections.pop().unwrap();
    d.revision += 1;
    let _ = s.lines(&d, 1);
    assert_eq!(s.category(), None);
    d.sections.push(gone);
    d.revision += 1;
    let _ = s.lines(&d, 1);
    assert_eq!(
        s.section(&d),
        None,
        "the section's return doesn't choose it again"
    );
}

/// The panel takes every key it doesn't use wherever focus is in it, an element other than the
/// categories and the results included (none is focusable today: a press on the header or the
/// note leaves the search field focused), so nothing reaches the window.
#[test]
fn the_panel_takes_keys_with_focus_elsewhere_in_it() {
    let route = super::keys::Route::<ReferenceState> {
        get_mut: |s| s,
        keys: ReferenceKeys::default(),
        categories: "r-category-".to_owned(),
        search: "r-search".to_owned(),
        chosen_category: "r-category-0".to_owned(),
        results: "r-results".to_owned(),
        line_key: Rc::new(|i| format!("r-line-{i}")),
        lines: Rc::default(),
    };
    let mut s = ReferenceState::new();
    s.open();
    let mut tx = rune::Tx::default();
    for ev in [
        key(Key::Delete, Mods::default()),
        key(Key::Text("x".into()), Mods::default()),
        key(Key::Command('v'), ctrl()),
    ] {
        assert!(route.panel_key(&mut s, &ev, &mut tx), "{ev:?} is taken");
    }
    assert!(
        !route.panel_key(&mut s, &key(Key::Tab, shift()), &mut tx),
        "Tab is Rune's"
    );
    assert!(s.is_open());
    assert!(route.panel_key(&mut s, &key(Key::Escape, ctrl()), &mut tx));
    assert!(!s.is_open(), "Escape with a modifier closes");
}

#[test]
fn super_with_a_letter_is_taken() {
    let logo = Mods {
        logo: true,
        ..Mods::default()
    };
    assert!(
        !edits(&key(Key::Text("e".into()), logo)),
        "Super+E is the desktop's or the app's"
    );
    assert!(!edits(&key(Key::Left, logo)));
}

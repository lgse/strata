// SPDX-License-Identifier: MIT

use super::*;
use crate::{test_support::gtk_test, ui::virtual_preview::SourceUnit};

fn units(text: &[&str]) -> Vec<PreviewUnit> {
    text.iter()
        .map(|text| {
            PreviewUnit::Source(SourceUnit {
                display_end: text.len(),
                source: (*text).into(),
                first_line: 1,
                line_count: 1,
                continuation: false,
            })
        })
        .collect()
}

#[test]
fn word_click_and_shift_extension_preserve_unicode_and_anchor() {
    gtk_test(
        "ui::virtual_preview::selection::tests::word_click_and_shift_extension_preserve_unicode_and_anchor",
        || {
            let units = units(&["éclair bravo\n"]);
            let word = clicked_selection(
                &units,
                None,
                SelectionPoint { unit: 0, offset: 2 },
                2,
                false,
            );
            assert_eq!((word.anchor.offset, word.focus.offset), (0, 6));
            let extended = clicked_selection(
                &units,
                Some(word),
                SelectionPoint {
                    unit: 0,
                    offset: 10,
                },
                1,
                true,
            );
            assert_eq!((extended.anchor.offset, extended.focus.offset), (0, 10));
            let reversed = clicked_selection(
                &units,
                Some(DocumentSelection {
                    anchor: SelectionPoint { unit: 0, offset: 6 },
                    focus: word.focus,
                }),
                word.anchor,
                1,
                true,
            );
            assert_eq!((reversed.anchor.offset, reversed.focus.offset), (6, 0));
            let line = clicked_selection(
                &units,
                None,
                SelectionPoint { unit: 0, offset: 9 },
                3,
                false,
            );
            assert_eq!((line.anchor.offset, line.focus.offset), (0, 13));
        },
    );
}

#[test]
fn keyboard_selection_crosses_recycled_units_and_graphemes() {
    gtk_test(
        "ui::virtual_preview::selection::tests::keyboard_selection_crosses_recycled_units_and_graphemes",
        || {
            let units = units(&["a\u{301}b", "cd\nef"]);
            let mut selection = None;
            for (motion, control, expected) in [
                (Motion::Right, false, SelectionPoint { unit: 0, offset: 2 }),
                (Motion::Right, false, SelectionPoint { unit: 0, offset: 3 }),
                (Motion::Right, false, SelectionPoint { unit: 1, offset: 1 }),
                (Motion::Down, false, SelectionPoint { unit: 1, offset: 4 }),
                (Motion::Up, false, SelectionPoint { unit: 1, offset: 1 }),
                (Motion::Home, false, SelectionPoint { unit: 1, offset: 0 }),
                (Motion::End, false, SelectionPoint { unit: 1, offset: 2 }),
                (Motion::End, true, SelectionPoint { unit: 1, offset: 5 }),
                (Motion::Left, true, SelectionPoint { unit: 1, offset: 3 }),
                (Motion::Left, true, SelectionPoint { unit: 1, offset: 0 }),
                (Motion::Home, true, SelectionPoint { unit: 0, offset: 0 }),
                (Motion::Left, true, SelectionPoint { unit: 0, offset: 0 }),
            ] {
                selection = move_selection(&units, selection, motion, control, true);
                let current = selection.expect("navigation produces a document selection");
                assert_eq!(current.focus, expected);
                assert_eq!(current.anchor, SelectionPoint { unit: 0, offset: 0 });
            }
            let selection = move_selection(&units, None, Motion::End, true, true)
                .expect("select to the document end");
            let collapsed = move_selection(&units, Some(selection), Motion::Left, false, false)
                .expect("collapse selection on Left");
            assert!(collapsed.is_empty());
            let word_units = self::units(&["one x", "a next a\u{301}", "é\u{301}"]);
            for (from, motion, control, expected) in [
                (
                    SelectionPoint { unit: 1, offset: 0 },
                    Motion::Left,
                    true,
                    SelectionPoint { unit: 0, offset: 4 },
                ),
                (
                    SelectionPoint { unit: 0, offset: 5 },
                    Motion::Right,
                    true,
                    SelectionPoint { unit: 1, offset: 1 },
                ),
                (
                    SelectionPoint { unit: 1, offset: 9 },
                    Motion::Right,
                    false,
                    SelectionPoint { unit: 2, offset: 2 },
                ),
                (
                    SelectionPoint { unit: 2, offset: 0 },
                    Motion::Left,
                    false,
                    SelectionPoint { unit: 1, offset: 7 },
                ),
                (
                    SelectionPoint { unit: 1, offset: 2 },
                    Motion::Up,
                    false,
                    SelectionPoint { unit: 0, offset: 2 },
                ),
            ] {
                let previous = Some(DocumentSelection {
                    anchor: from,
                    focus: from,
                });
                let selection = move_selection(&word_units, previous, motion, control, true)
                    .expect("navigate selection between document units");
                assert_eq!(selection.anchor, from);
                assert_eq!(selection.focus, expected);
            }
            assert_eq!(collapsed.focus, selection.anchor);
        },
    );
}

#[test]
fn case_insensitive_find_keeps_original_character_offsets() {
    assert_eq!(
        match_ranges("Éclair éclair İ λλ", "éclair"),
        vec![(0, 6), (7, 13)]
    );
    assert_eq!(
        match_ranges("Éclair éclair İ λλ", "i"),
        vec![(4, 5), (11, 12), (14, 15)]
    );
    assert_eq!(match_ranges("Éclair éclair İ λλ", "İ"), vec![(14, 15)]);
    assert_eq!(match_ranges("[A] abc a", "[a]"), vec![(0, 3)]);
    assert_eq!(match_ranges("aaaa", "aa"), vec![(0, 2), (2, 4)]);
    assert!(match_ranges("anything", "").is_empty());
}

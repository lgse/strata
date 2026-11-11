// SPDX-License-Identifier: MIT

use super::{super::tree::Seen, state};

/// A single-line field holding `text`, with the selection's ends at these clusters.
fn field(text: &str, anchor: u64, caret: u64) -> Seen {
    Seen {
        role: "text-input".to_owned(),
        name: "Rename".to_owned(),
        value: Some(text.to_owned()),
        selection: Some([1, anchor, 1, caret]),
        ..Seen::default()
    }
}

#[test]
fn a_fields_places_are_counted_in_characters_whatever_a_character_takes_in_bytes() {
    for (text, anchor, caret, selection, at) in [
        ("todo.txt", 0, 4, Some([0, 4]), 4),
        ("todo.txt", 8, 8, None, 8),
        // Backwards, as Shift+Home leaves it: the caret is the end that moved.
        ("todo.txt", 8, 0, Some([0, 8]), 0),
        // Two bytes and three bytes a character.
        ("māo", 0, 3, Some([0, 3]), 3),
        ("日本語.txt", 0, 3, Some([0, 3]), 3),
        ("résumé.pdf", 6, 10, Some([6, 10]), 10),
        // A letter and its combining accent: one cluster, two characters.
        ("re\u{301}sume\u{301}", 2, 6, Some([3, 8]), 8),
        ("", 0, 0, None, 0),
    ] {
        let read = state(&field(text, anchor, caret), true).expect("a field");

        assert_eq!(read.selection, selection, "{text:?}");
        assert_eq!(read.caret, at, "{text:?}");
        assert_eq!(read.text, text);
        assert_eq!(read.name, "Rename");
        assert!(read.focused);
    }
}

#[test]
fn only_a_text_field_whose_caret_the_tree_places_is_read() {
    let label = Seen {
        role: "label".to_owned(),
        ..field("todo.txt", 0, 4)
    };
    let on_a_later_line = Seen {
        role: "multiline-text-input".to_owned(),
        selection: Some([2, 0, 2, 3]),
        ..field("one\ntwo", 0, 0)
    };
    let unplaced = Seen {
        selection: None,
        ..field("todo.txt", 0, 0)
    };
    let empty = Seen {
        selection: None,
        ..field("", 0, 0)
    };

    assert_eq!(state(&label, false), None);
    assert_eq!(state(&on_a_later_line, true), None);
    assert_eq!(state(&unplaced, true), None);
    let read = state(&empty, false).expect("an empty field");
    assert_eq!((read.caret, read.selection, read.focused), (0, None, false));
}

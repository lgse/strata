// SPDX-License-Identifier: MIT

use super::descent::slash_over;

#[test]
fn a_lone_slash_over_a_query_enters_and_any_other_edit_types() {
    let cases = [
        // Typed after the query, over the whole of it (Ctrl+A), and inside it.
        ("doc", "doc/", Some("doc")),
        ("doc", "/", Some("doc")),
        ("ab", "a/b", Some("ab")),
        ("a/", "a//", Some("a/")),
        // A pasted path holds more than the `/`; a deletion holds no `/`.
        ("doc", "doc/x", None),
        ("doc", "do", None),
        ("doc", "doc", None),
        // No query to enter by.
        ("", "/", None),
        ("  ", "  /", None),
        // Text before the edit that is not ASCII.
        ("été", "été/", Some("été")),
    ];
    for (before, after, entered) in cases {
        assert_eq!(
            slash_over(before, after),
            entered,
            "{before:?} to {after:?}"
        );
    }
}

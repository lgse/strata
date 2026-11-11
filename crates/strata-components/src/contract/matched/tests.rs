use rune::{Cx, El, Env, Runtime, col};

use super::*;

/// One expected range: clippy reads `[a..b]` as a mistyped `vec`.
fn only(r: Range<usize>) -> Vec<Range<usize>> {
    vec![r]
}

#[test]
fn ranges_are_sorted_merged_and_clamped() {
    let text = "documents";
    let got = normalize_ranges(text, [5..7, 0..2, 1..3, 3..4, 8..20, 4..4, 30..40]);
    assert_eq!(
        got,
        [0..4, 5..7, 8..9],
        "overlapping and touching ranges merge"
    );
}

#[test]
fn a_range_inside_a_multibyte_character_takes_the_whole_character() {
    let text = "Café"; // é is bytes 3..5
    assert_eq!(normalize_ranges(text, Some(4..5)), only(3..5));
    assert_eq!(normalize_ranges(text, Some(3..4)), only(3..5));
    assert_eq!(normalize_ranges("", Some(0..3)), Vec::<Range<usize>>::new());
}

#[test]
fn character_positions_become_byte_ranges() {
    let text = "日本語のファイル";
    assert_eq!(char_ranges(text, [0, 1, 4, 99]), [0..6, 12..15]);
    assert_eq!(char_ranges("abc", [2, 0]), [0..1, 2..3]);
}

#[test]
fn fuzzy_finds_a_case_insensitive_subsequence_and_its_characters() {
    let m = fuzzy("DOC", "Documents").expect("a prefix matches");
    assert_eq!(m.ranges, only(0..3));
    let m = fuzzy("stc", "strata-components").expect("a scattered match");
    assert_eq!(m.ranges, [0..2, 7..8]);
    let m = fuzzy("dé", "Café déjà vu").expect("multibyte characters match");
    assert_eq!(m.ranges, only(6..9));
    assert_eq!(fuzzy("xyz", "Documents"), None);
    assert_eq!(
        fuzzy("docs", "doc"),
        None,
        "every query character must match"
    );
    let all = fuzzy("  ", "anything").expect("an empty query matches");
    assert_eq!((all.score, all.ranges.len()), (0, 0));
}

#[test]
fn fuzzy_ranks_contiguous_runs_and_word_starts_first() {
    let score = |text| fuzzy("doc", text).map(|m| m.score).expect("matches");
    assert!(
        score("Documents") > score("dark-orange-cat"),
        "a run beats word starts"
    );
    assert!(
        score("dark-orange-cat") > score("addendum-of-c"),
        "word starts beat mid-word"
    );
    assert!(
        score("doc") > score("documents"),
        "unmatched characters cost"
    );
}

/// The label for "Café déjà vu" with "dé" matched, keyed `label`.
fn view(_: &(), _: &mut Cx<()>) -> El {
    let k = Kit::neutral(false);
    let label = MatchedText::new("Café déjà vu", Some(6..9))
        .key("label")
        .build(&k);
    col().child(label)
}

#[test]
fn assistive_technology_hears_the_plain_text() {
    let mut rt = Runtime::new((), view, (400.0, 100.0), Env::new(true, 1.0));
    rt.frame(0.0);
    // Walk the tree (a `serde_json::Value`, which this crate doesn't name) for the label.
    let mut stack = vec![rt.accessibility_json()];
    let mut found = None;
    while let Some(node) = stack.pop() {
        if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
            stack.extend(children.iter().cloned());
        }
        if node["key"] == "label" {
            found = Some(node);
        }
    }
    let node = found.expect("the label is in the tree");
    assert_eq!(node["role"], "label");
    assert_eq!(
        node["name"], "Café déjà vu",
        "one name, the text with no markup"
    );
    assert!(
        node.get("children").is_none(),
        "the matched range is no node of its own"
    );
}

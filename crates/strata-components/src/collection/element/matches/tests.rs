// SPDX-License-Identifier: MIT

use std::ops::Range;

use strata_kit::look::Look;

use super::NameMatches;

type Pair = (usize, usize);

fn ranges(matches: NameMatches) -> Vec<Pair> {
    matches
        .ranges()
        .map(|range| (range.start, range.end))
        .collect()
}

fn of(pairs: &[Pair]) -> impl Iterator<Item = Range<usize>> + '_ {
    pairs.iter().map(|&(start, end)| start..end)
}

#[test]
fn new_skips_empty_ranges_and_keeps_the_first_eight() {
    assert!(NameMatches::new(of(&[(3, 3)])).is_empty());
    let many = NameMatches::new((0..10).map(|at| at * 2..at * 2 + 1));
    assert_eq!(
        ranges(many),
        (0..8).map(|at| (at * 2, at * 2 + 1)).collect::<Vec<_>>()
    );
}

#[test]
fn spans_paint_each_range_in_the_accent() {
    let look = Look::default();
    let spans = NameMatches::new(of(&[(0, 2), (5, 7)])).spans(&look);
    let painted: Vec<_> = spans
        .iter()
        .map(|span| (span.start, span.end, span.bg, span.color))
        .collect();
    let (accent, base) = (Some(look.kit.c.bg_accent), Some(look.kit.c.bg_base));
    assert_eq!(painted, [(0, 2, accent, base), (5, 7, accent, base)]);
}

/// A caption cut in the middle keeps the matches in its two ends, moved onto the text shown,
/// and drops the ones the ellipsis replaced.
#[test]
fn onto_a_cut_name_keeps_the_ends_matches() {
    let name = "report-draft-final.txt";
    let shown = "report-…l.txt";
    let cases: [(&[Pair], &[Pair]); 5] = [
        (&[(0, 6)], &[(0, 6)]),
        (&[(7, 12)], &[]),
        (&[(19, 22)], &[(12, 15)]),
        (&[(5, 9), (17, 19)], &[(5, 7), (10, 12)]),
        (&[], &[]),
    ];
    for (matched, expected) in cases {
        let moved = NameMatches::new(of(matched)).onto(name, shown);
        assert_eq!(ranges(moved), expected, "{matched:?}");
    }
    let whole = NameMatches::new(of(&[(2, 4)]));
    assert_eq!(whole.onto(name, name), whole);
}

#[test]
fn onto_keeps_character_boundaries() {
    let name = "Ärger-über-alles.md";
    let shown = "Ärger…es.md";
    let moved = NameMatches::new(of(&[(0, 2), (17, 20)])).onto(name, shown);
    assert_eq!(ranges(moved), [(0, 2), (10, 13)]);
}

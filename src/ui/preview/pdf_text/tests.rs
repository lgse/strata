// SPDX-License-Identifier: MIT

use super::*;

fn layer(text: &str, glyphs: Vec<[f32; 4]>) -> PdfTextLayer {
    PdfTextLayer {
        width: 400.0,
        height: 600.0,
        text: text.to_owned(),
        glyphs,
    }
}

/// Two lines of two glyphs each; the newline is a zero-area point.
fn two_line_layer() -> PdfTextLayer {
    layer(
        "ab\ncd",
        vec![
            [10.0, 10.0, 20.0, 22.0],
            [20.0, 10.0, 30.0, 22.0],
            [30.0, 22.0, 30.0, 22.0],
            [10.0, 40.0, 20.0, 52.0],
            [20.0, 40.0, 30.0, 52.0],
        ],
    )
}

#[test]
fn caret_snaps_to_glyph_midpoints_on_the_nearest_line() {
    let layer = two_line_layer();
    assert_eq!(caret_at(&layer, 5.0, 16.0), 0);
    assert_eq!(caret_at(&layer, 16.0, 16.0), 1);
    assert_eq!(caret_at(&layer, 26.0, 16.0), 2);
    // Same x on the second line maps into its own char range.
    assert_eq!(caret_at(&layer, 16.0, 46.0), 4);
    assert_eq!(caret_at(&layer, 26.0, 46.0), 5);
}

#[test]
fn caret_off_the_text_snaps_to_line_edges() {
    let layer = two_line_layer();
    // Far right of a line lands before its trailing newline.
    assert_eq!(caret_at(&layer, 300.0, 16.0), 2);
    // Above the first line snaps to the start.
    assert_eq!(caret_at(&layer, 5.0, -50.0), 0);
    // Below the last line lands at the end of the text.
    assert_eq!(caret_at(&layer, 300.0, 500.0), 5);
}

#[test]
fn hit_text_distinguishes_glyphs_from_margins() {
    let layer = two_line_layer();
    assert!(hit_text(&layer, 15.0, 16.0));
    assert!(hit_text(&layer, 25.0, 46.0));
    // Page margin far from any glyph stays a pan target.
    assert!(!hit_text(&layer, 350.0, 500.0));
    assert!(!hit_text(&layer, 15.0, 300.0));
}

#[test]
fn selection_runs_merge_per_line_and_skip_empty_ranges() {
    let tails = layer(
        "ay",
        vec![[10.0, 10.0, 20.0, 22.0], [20.0, 10.0, 30.0, 22.0]],
    );
    let layer = two_line_layer();
    assert!(selection_runs(&layer, 0, 0).is_empty());
    // "ab\ncd" has no descenders, so each band stops just past the baseline.
    let trimmed = |top: f32, bottom: f32| top + (bottom - top) * 0.82;
    assert_eq!(
        selection_runs(&layer, 1, 4),
        vec![
            [20.0, 10.0, 30.0, trimmed(10.0, 22.0)],
            [10.0, 40.0, 20.0, trimmed(40.0, 52.0)]
        ]
    );
    // A reversed range selects identically.
    assert_eq!(selection_runs(&layer, 4, 1), selection_runs(&layer, 1, 4));
    // A run containing a descender keeps the line's full bottom edge.
    assert_eq!(selection_runs(&tails, 0, 2), vec![[10.0, 10.0, 30.0, 22.0]]);
}

#[test]
fn selection_text_preserves_page_newlines() {
    let layer = two_line_layer();
    assert_eq!(selection_text(&layer, 0, 5), "ab\ncd");
    assert_eq!(selection_text(&layer, 1, 4), "b\nc");
    assert_eq!(selection_text(&layer, 3, 3), "");
}

#[test]
fn image_bounds_letterboxes_like_content_fit_contain() {
    let layer = layer("x", vec![[0.0, 0.0, 1.0, 1.0]]);
    // 400x600 page in a 100x600 widget: scale 0.25, centered vertically.
    let (x, y, scale) = image_bounds(&layer, 100.0, 600.0);
    assert_eq!((x, y, scale), (0.0, 225.0, 0.25));
}

#[test]
fn mismatched_glyph_counts_degrade_gracefully() {
    // A helper-side count mismatch never reaches the UI, but a short glyph list
    // must not panic if one slips through.
    let layer = layer("abc", vec![[0.0, 0.0, 10.0, 10.0]]);
    assert_eq!(caret_at(&layer, 5.0, 5.0), 1);
    assert_eq!(selection_text(&layer, 0, 3), "a");
}

#[test]
fn word_range_expands_to_whitespace_boundaries() {
    let layer = layer(
        "one two",
        vec![
            [10.0, 10.0, 20.0, 22.0],
            [20.0, 10.0, 30.0, 22.0],
            [30.0, 10.0, 40.0, 22.0],
            [40.0, 22.0, 40.0, 22.0],
            [50.0, 10.0, 60.0, 22.0],
            [60.0, 10.0, 70.0, 22.0],
            [70.0, 10.0, 80.0, 22.0],
        ],
    );
    assert_eq!(word_range(&layer, 0), (0, 3));
    assert_eq!(word_range(&layer, 1), (0, 3));
    assert_eq!(word_range(&layer, 4), (4, 7));
    // A press on the space itself selects nothing.
    assert_eq!(word_range(&layer, 3), (3, 3));
}

#[test]
fn line_range_covers_the_line_without_its_newline() {
    let layer = two_line_layer();
    assert_eq!(line_range(&layer, 0), (0, 2));
    assert_eq!(line_range(&layer, 2), (0, 2));
    assert_eq!(line_range(&layer, 4), (3, 5));
}

// SPDX-License-Identifier: MIT

use super::super::pixels::{self, Capture};

fn capture(width: usize, height: usize, pixels: Vec<[u8; 3]>) -> Capture {
    Capture {
        x: 100,
        y: 50,
        width,
        height,
        pixels,
    }
}

#[test]
fn text_is_measured_like_the_pil_luma_centroid() {
    assert_eq!(pixels::luma([255, 255, 255]), 255);
    assert_eq!(pixels::luma([255, 0, 0]), 76);
    assert_eq!(pixels::luma([0, 255, 0]), 150);
    let white = [255, 255, 255];
    let black = [0, 0, 0];
    let mut rows = vec![white; 12];
    rows[8] = black;
    let (position, contrast) = pixels::text_position(&capture(4, 3, rows));
    assert_eq!((position, contrast), (Some(2.0), 255));
    let mut rows = vec![white; 12];
    rows[0] = black;
    rows[8] = black;
    assert_eq!(
        pixels::text_position(&capture(4, 3, rows)),
        (Some(1.0), 510)
    );
    assert_eq!(
        pixels::text_position(&capture(4, 3, vec![white; 12])),
        (None, 0),
        "a box with no ink has no position"
    );
}

#[test]
fn hover_feedback_ignores_only_the_pointer_square() {
    let before = capture(60, 40, vec![[10, 10, 10]; 2400]);
    let changed_at = |column: usize, row: usize| {
        let mut after = before.clone();
        after.pixels[row * 60 + column] = [200, 200, 200];
        after
    };
    let pointer = (100 + 10, 50 + 5);
    assert!(!pixels::differs_beside_pointer(
        &before,
        &changed_at(8, 3),
        pointer
    ));
    assert!(!pixels::differs_beside_pointer(
        &before,
        &changed_at(42, 37),
        pointer
    ));
    assert!(pixels::differs_beside_pointer(
        &before,
        &changed_at(7, 3),
        pointer
    ));
    assert!(pixels::differs_beside_pointer(
        &before,
        &changed_at(43, 10),
        pointer
    ));
    assert!(!pixels::differs_beside_pointer(&before, &before, pointer));
}

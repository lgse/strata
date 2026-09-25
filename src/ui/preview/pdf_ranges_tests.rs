// SPDX-License-Identifier: MIT

use std::{collections::HashMap, sync::Arc};

use super::{PdfTextLayer, pdf_desired_ranges, pdf_shortcut_modifiers};

fn layer(text: &str) -> Arc<PdfTextLayer> {
    let glyphs = text
        .chars()
        .enumerate()
        .map(|(i, _)| [i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, 12.0])
        .collect();
    Arc::new(PdfTextLayer {
        width: 800.0,
        height: 600.0,
        text: text.to_owned(),
        glyphs,
    })
}

fn layers() -> HashMap<i32, Arc<PdfTextLayer>> {
    HashMap::from([
        (1, layer("one two\nthree")),
        (2, layer("four five")),
        (3, layer("six")),
    ])
}

#[test]
fn char_granularity_selects_between_endpoints() {
    let desired = pdf_desired_ranges(&layers(), (1, 1), (1, 5), 1);
    assert_eq!(desired, HashMap::from([(1, (1, 5))]));
}

#[test]
fn cross_page_drag_fills_intermediate_pages() {
    let desired = pdf_desired_ranges(&layers(), (1, 4), (3, 2), 1);
    assert_eq!(
        desired,
        HashMap::from([(1, (4, 13)), (2, (0, 9)), (3, (0, 2))])
    );
}

#[test]
fn backward_drag_covers_the_anchor_word_fully() {
    let desired = pdf_desired_ranges(&layers(), (2, 6), (1, 5), 2);
    assert_eq!(desired, HashMap::from([(1, (4, 13)), (2, (0, 9))]));
}

#[test]
fn word_granularity_snaps_endpoints_to_words() {
    let desired = pdf_desired_ranges(&layers(), (1, 1), (1, 5), 2);
    assert_eq!(desired, HashMap::from([(1, (0, 7))]));
}

#[test]
fn line_granularity_snaps_to_whole_lines() {
    let desired = pdf_desired_ranges(&layers(), (1, 1), (1, 9), 3);
    assert_eq!(desired, HashMap::from([(1, (0, 13))]));
}

#[test]
fn same_page_backward_drag_selects_upward() {
    let desired = pdf_desired_ranges(&layers(), (1, 9), (1, 1), 1);
    assert_eq!(desired, HashMap::from([(1, (1, 9))]));
}

#[test]
fn pdf_shortcuts_ignore_latch_bits_but_reject_shift_and_alt() {
    use gtk::gdk::ModifierType as M;
    assert!(pdf_shortcut_modifiers(M::CONTROL_MASK));
    assert!(pdf_shortcut_modifiers(M::CONTROL_MASK | M::LOCK_MASK));
    assert!(!pdf_shortcut_modifiers(M::empty()));
    assert!(!pdf_shortcut_modifiers(M::CONTROL_MASK | M::SHIFT_MASK));
    assert!(!pdf_shortcut_modifiers(M::CONTROL_MASK | M::ALT_MASK));
}

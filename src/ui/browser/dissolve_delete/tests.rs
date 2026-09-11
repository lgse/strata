// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn fragment_budget_is_bounded_for_large_batches() {
    assert_eq!(fragment_budget(1), MIN_FRAGMENT_BUDGET);
    assert_eq!(fragment_budget(4), MAX_FRAGMENT_BUDGET);
    assert_eq!(fragment_budget(1_000), MAX_FRAGMENT_BUDGET);
}

#[test]
fn one_row_stays_below_one_hundred_animated_fragments() {
    let width = 300.0;
    let height = 28.0;
    let tile = tile_size_for_budget(f64::from(width * height), fragment_budget(1));
    let mut random = Random::new(5);

    let fragments = fragments_for_size(width, height, tile, 0, &mut random);

    assert!(fragments.len() <= 100);
}

#[test]
fn viewport_batch_keeps_a_small_fixed_rendering_budget() {
    let rows = 20;
    let width = 300.0;
    let height = 28.0;
    let tile = tile_size_for_budget(
        f64::from(width * height) * rows as f64,
        fragment_budget(rows),
    );
    let mut random = Random::new(9);
    let fragments: usize = (0..rows)
        .map(|row| fragments_for_size(width, height, tile, row, &mut random).len())
        .sum();

    assert!(fragments <= 240);
}

#[test]
fn only_bounds_intersecting_the_viewport_animate() {
    assert!(bounds_intersect_viewport(
        gtk::graphene::Rect::new(10.0, 10.0, 300.0, 28.0),
        800,
        600,
    ));
    assert!(!bounds_intersect_viewport(
        gtk::graphene::Rect::new(10.0, 620.0, 300.0, 28.0),
        800,
        600,
    ));
}

#[test]
fn fragments_cover_the_complete_row() {
    let mut random = Random::new(7);
    let fragments = fragments_for_size(303.0, 29.0, 10.0, 0, &mut random);

    let right = fragments
        .iter()
        .map(|fragment| fragment.source.x() + fragment.source.width())
        .fold(0.0, f32::max);
    let bottom = fragments
        .iter()
        .map(|fragment| fragment.source.y() + fragment.source.height())
        .fold(0.0, f32::max);

    assert_eq!(right, 303.0);
    assert_eq!(bottom, 29.0);
}

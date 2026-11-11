use super::*;

const H: f32 = 24.0;

/// 100,000 rows, 400 wide, in a 240-point viewport (10 rows): far more than any element mounts.
fn rows() -> UniformRows {
    UniformRows::new(100_000, H, 400.0).viewport_height(240.0)
}

fn band(y0: f32, y1: f32) -> Rect {
    Rect::from_corners(Point::new(10.0, y0), Point::new(200.0, y1))
}

/// One expected range: clippy reads `[a..b]` as a mistyped `vec`.
fn only(r: Range<usize>) -> Vec<Range<usize>> {
    vec![r]
}

fn ranges(set: &IndexRanges) -> Vec<Range<usize>> {
    set.ranges().to_vec()
}

#[test]
fn a_band_selects_rows_far_past_the_mounted_window() {
    let g = rows();
    let set = g.range_in(band(5_000.0 * H, 15_000.0 * H));
    assert_eq!(ranges(&set), only(5_000..15_000));
    assert_eq!(set.count(), 10_000);
    let whole = g.range_in(band(-50.0, 100_001.0 * H));
    assert_eq!(
        ranges(&whole),
        only(0..100_000),
        "a band past both ends clamps to every row"
    );
}

#[test]
fn a_band_dragged_up_or_left_covers_the_same_rows() {
    let g = rows();
    let down = g.range_in(Rect::from_corners(
        Point::new(10.0, 30.0),
        Point::new(90.0, 100.0),
    ));
    let up = g.range_in(Rect::from_corners(
        Point::new(90.0, 100.0),
        Point::new(10.0, 30.0),
    ));
    let raw = g.range_in(Rect::new(90.0, 100.0, -80.0, -70.0));
    assert_eq!(ranges(&down), only(1..5));
    assert_eq!(up, down);
    assert_eq!(raw, down, "a rect with a negative size is normalised");
}

#[test]
fn a_band_ending_on_a_row_edge_leaves_that_row_out() {
    let g = rows();
    assert_eq!(ranges(&g.range_in(band(2.0 * H, 4.0 * H))), only(2..4));
    assert_eq!(
        ranges(&g.range_in(band(2.0 * H - 0.5, 4.0 * H + 0.5))),
        only(1..5)
    );
}

#[test]
fn a_zero_height_band_takes_the_row_it_lies_in() {
    let g = rows();
    assert_eq!(ranges(&g.range_in(band(3.5 * H, 3.5 * H))), only(3..4));
    assert_eq!(
        ranges(&g.range_in(band(3.0 * H, 3.0 * H))),
        only(3..4),
        "a row's top edge is in it"
    );
}

#[test]
fn a_band_outside_the_rows_takes_nothing() {
    let g = rows();
    assert!(
        g.range_in(Rect::new(400.0, 0.0, 50.0, 100.0)).is_empty(),
        "right of the rows"
    );
    assert!(
        g.range_in(Rect::new(-60.0, 0.0, 50.0, 100.0)).is_empty(),
        "left of the rows"
    );
    assert!(
        g.range_in(band(-100.0, -1.0)).is_empty(),
        "above the first row"
    );
    assert!(
        g.range_in(band(100_000.0 * H, 100_010.0 * H)).is_empty(),
        "below the last row"
    );
}

#[test]
fn the_top_inset_shifts_every_row() {
    let g = UniformRows::new(10, H, 400.0).top(30.0);
    assert_eq!(
        g.index_at(Point::new(5.0, 29.0)),
        None,
        "the inset holds no row"
    );
    assert_eq!(g.index_at(Point::new(5.0, 30.0)), Some(0));
    assert_eq!(g.rect_of(2), Rect::new(0.0, 30.0 + 2.0 * H, 400.0, H));
    assert_eq!(ranges(&g.range_in(band(0.0, 30.0 + H))), only(0..1));
}

#[test]
fn index_at_inverts_rect_of() {
    let g = rows();
    for i in [0, 1, 9, 10, 4_999, 50_000, 99_999] {
        assert_eq!(g.index_at(g.rect_of(i).center()), Some(i), "row {i}");
    }
    assert_eq!(
        g.index_at(Point::new(5.0, 100_000.0 * H)),
        None,
        "past the last row"
    );
    assert_eq!(
        g.index_at(Point::new(400.0, 5.0)),
        None,
        "the right edge is out"
    );
    assert_eq!(g.index_at(Point::new(-1.0, 5.0)), None);
    assert_eq!(
        g.index_at(Point::new(f32::NAN, 5.0)),
        None,
        "a NaN x hits nothing"
    );
    assert_eq!(
        g.index_at(Point::new(5.0, f32::NAN)),
        None,
        "nor does a NaN y"
    );
    assert!(g.range_in(Rect::new(0.0, f32::NAN, 10.0, 10.0)).is_empty());
}

#[test]
fn neighbors_stop_at_the_edges() {
    let g = rows();
    assert_eq!(g.neighbor(0, Direction::Up), None);
    assert_eq!(g.neighbor(0, Direction::Down), Some(1));
    assert_eq!(g.neighbor(99_999, Direction::Down), None);
    assert_eq!(g.neighbor(99_999, Direction::Up), Some(99_998));
    assert_eq!(
        g.neighbor(500, Direction::Left),
        None,
        "a list has no side neighbours"
    );
    assert_eq!(g.neighbor(500, Direction::Right), None);
    assert_eq!(
        g.neighbor(100_000, Direction::Up),
        None,
        "an index past the end has none"
    );
    let one = UniformRows::new(1, H, 400.0);
    assert_eq!(one.neighbor(0, Direction::Up), None);
    assert_eq!(one.neighbor(0, Direction::Down), None);
}

#[test]
fn an_empty_collection_hits_nothing() {
    let g = UniformRows::new(0, H, 400.0).viewport_height(240.0);
    assert_eq!(g.index_at(Point::new(5.0, 5.0)), None);
    assert!(g.range_in(band(0.0, 1_000.0)).is_empty());
    assert_eq!(g.neighbor(0, Direction::Down), None);
}

#[test]
fn a_page_is_the_whole_rows_that_fit_minus_one_and_never_zero() {
    assert_eq!(rows().page_step(), 9);
    assert_eq!(
        UniformRows::new(50, H, 400.0)
            .viewport_height(250.0)
            .page_step(),
        9
    );
    for (height, step) in [(H, 1), (2.0 * H, 1), (3.0 * H, 2)] {
        assert_eq!(
            UniformRows::new(50, H, 400.0)
                .viewport_height(height)
                .page_step(),
            step,
            "a viewport of {height} points"
        );
    }
    assert_eq!(
        UniformRows::new(50, H, 400.0)
            .viewport_height(10.0)
            .page_step(),
        1
    );
    assert_eq!(
        UniformRows::new(50, H, 400.0).page_step(),
        1,
        "no viewport yet"
    );
    assert_eq!(
        UniformRows::new(50, 0.0, 400.0)
            .viewport_height(240.0)
            .page_step(),
        1
    );
}

#[test]
fn index_ranges_merge_touching_ranges_and_answer_membership() {
    let mut set = IndexRanges::new();
    set.push(2..4);
    set.push(4..6);
    set.push(5..7);
    set.push(9..9);
    set.push(10..12);
    assert_eq!(ranges(&set), [2..7, 10..12]);
    assert_eq!(set.count(), 7);
    assert_eq!(set.indices().collect::<Vec<_>>(), [2, 3, 4, 5, 6, 10, 11]);
    for (i, inside) in [
        (1, false),
        (2, true),
        (6, true),
        (7, false),
        (10, true),
        (12, false),
    ] {
        assert_eq!(set.contains(i), inside, "{i}");
    }
}

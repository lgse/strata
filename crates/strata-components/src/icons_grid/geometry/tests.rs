use super::*;

/// 10 tiles, 4 a row (rows 0–3, 4–7, 8–9), cells 100 × 50 after 10 of padding, in a viewport
/// 3 rows tall.
fn grid() -> IconsGeometry {
    IconsGeometry {
        len: 10,
        per: 4,
        cell_w: 100.0,
        row_h: 50.0,
        pad: 10.0,
        viewport_h: 150.0,
    }
}

fn indices(r: IndexRanges) -> Vec<usize> {
    r.indices().collect()
}

#[test]
fn a_point_finds_the_tile_it_is_on_and_none_past_the_edges() {
    let g = grid();
    assert_eq!(g.index_at(Point::new(10.0, 0.0)), Some(0));
    assert_eq!(g.index_at(Point::new(9.9, 0.0)), None, "the side padding");
    assert_eq!(
        g.index_at(Point::new(409.9, 99.9)),
        Some(7),
        "a row's last cell"
    );
    assert_eq!(
        g.index_at(Point::new(410.0, 10.0)),
        None,
        "past the last column"
    );
    assert_eq!(g.index_at(Point::new(110.0, 100.0)), Some(9));
    assert_eq!(
        g.index_at(Point::new(210.0, 100.0)),
        None,
        "past the last tile of a short last row"
    );
    assert_eq!(
        g.index_at(Point::new(10.0, 150.0)),
        None,
        "below the last row"
    );
    assert_eq!(g.index_at(Point::new(f32::NAN, 10.0)), None);
    for i in 0..10 {
        assert_eq!(g.index_at(g.rect_of(i).center()), Some(i));
    }
}

#[test]
fn a_band_takes_a_block_of_columns_across_rows() {
    let g = grid();
    // Columns 1–2 of rows 0–2: 1, 2, 5, 6 and 9 (the short last row has no 10).
    let band = Rect::new(120.0, 10.0, 150.0, 120.0);
    assert_eq!(indices(g.range_in(band)), vec![1, 2, 5, 6, 9]);
    assert_eq!(g.range_in(band).ranges().len(), 3, "a range a row");
    // Dragged up and left from the same corners: the same block.
    let up = Rect::new(270.0, 130.0, -150.0, -120.0);
    assert_eq!(indices(g.range_in(up)), vec![1, 2, 5, 6, 9]);
}

#[test]
fn a_full_width_band_merges_its_rows_into_one_range() {
    let g = grid();
    let band = Rect::new(0.0, 20.0, 500.0, 60.0);
    let ranges = g.range_in(band);
    assert_eq!(ranges.ranges().len(), 1);
    assert_eq!(indices(ranges), (0..8).collect::<Vec<_>>());
}

#[test]
fn a_band_on_a_cell_edge_leaves_the_next_cell_out_and_a_line_takes_its_cells() {
    let g = grid();
    // Ending exactly on column 1's left edge and row 1's top edge.
    assert_eq!(
        indices(g.range_in(Rect::new(10.0, 0.0, 100.0, 50.0))),
        vec![0]
    );
    // A vertical line down column 2.
    assert_eq!(
        indices(g.range_in(Rect::new(250.0, 0.0, 0.0, 120.0))),
        vec![2, 6]
    );
    // A point.
    assert_eq!(
        indices(g.range_in(Rect::new(150.0, 60.0, 0.0, 0.0))),
        vec![5]
    );
    // In the padding, or below everything.
    assert!(g.range_in(Rect::new(0.0, 0.0, 0.0, 100.0)).is_empty());
    assert!(g.range_in(Rect::new(10.0, 200.0, 300.0, 50.0)).is_empty());
}

#[test]
fn up_and_down_move_by_rows_and_down_clamps_into_a_short_last_row() {
    let g = grid();
    assert_eq!(g.neighbor(5, Direction::Up), Some(1));
    assert_eq!(g.neighbor(1, Direction::Up), None, "the first row");
    assert_eq!(g.neighbor(1, Direction::Down), Some(5));
    assert_eq!(g.neighbor(5, Direction::Down), Some(9), "a tile below");
    assert_eq!(
        g.neighbor(6, Direction::Down),
        Some(9),
        "nothing under it: the last tile, as GTK's GridView moves"
    );
    assert_eq!(g.neighbor(9, Direction::Down), None, "the last row");
    let full = IconsGeometry { len: 8, ..grid() };
    assert_eq!(
        full.neighbor(5, Direction::Down),
        None,
        "a full last row has nothing below"
    );
}

#[test]
fn left_and_right_stay_in_the_row() {
    let g = grid();
    assert_eq!(g.neighbor(5, Direction::Left), Some(4));
    assert_eq!(
        g.neighbor(4, Direction::Left),
        None,
        "C1c's wrap crosses it"
    );
    assert_eq!(g.neighbor(3, Direction::Right), None);
    assert_eq!(g.neighbor(8, Direction::Right), Some(9));
    assert_eq!(g.neighbor(9, Direction::Right), None, "the last tile");
}

#[test]
fn a_page_is_the_rows_that_fit_less_one_times_the_tiles_a_row() {
    assert_eq!(grid().page_step(), 8);
    let one = IconsGeometry {
        viewport_h: 40.0,
        ..grid()
    };
    assert_eq!(one.page_step(), 4, "at least one row");
}

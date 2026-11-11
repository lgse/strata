//! Group header lines: how lines map to entries, and the grouped geometry.

use super::*;

const H: f32 = 20.0;

/// Three groups: lines `H0 e0 e1 e2 | H1 e3 e4 | H2 e5 e6 e7 e8`.
fn groups() -> Groups {
    Groups::new([
        Group::new(10, "Folders", 3),
        Group::new(20, "Images", 0),
        Group::new(30, "Documents", 2),
        Group::new(40, "Other", 4),
    ])
}

fn geometry() -> GroupedRows {
    GroupedRows::new(groups(), H, 300.0).viewport_height(5.0 * H)
}

#[test]
fn every_group_takes_its_header_line_then_a_line_an_entry() {
    let g = groups();
    assert_eq!(g.groups().len(), 3, "the empty group is dropped");
    assert_eq!(g.len(), 9);
    let map = LineMap::new(1, Some(&g), 9);
    assert_eq!(map.count(9), 12);
    let lines: Vec<Line> = (0..12).map(|l| map.line(l, 9)).collect();
    assert_eq!(lines[0], Line::Header(0));
    assert_eq!(lines[3], Line::Entries(2..3));
    assert_eq!(lines[4], Line::Header(1));
    assert_eq!(lines[7], Line::Header(2));
    assert_eq!(lines[11], Line::Entries(8..9));
    for i in 0..9 {
        assert_eq!(
            map.line(map.line_of(i), 9),
            Line::Entries(i..i + 1),
            "entry {i}"
        );
    }
    assert_eq!(g.group_of(3), Some(1));
    assert_eq!(g.entries(2), 5..9);
    assert_eq!(g.group_of(9), None);
}

#[test]
fn a_range_of_lines_holds_the_entries_on_them_and_no_header() {
    let map = LineMap::new(1, Some(&groups()), 9);
    assert_eq!(map.entries_in(0..1, 9), 0..0, "a header alone");
    assert_eq!(map.entries_in(3..6, 9), 2..4, "across a header");
    assert_eq!(map.entries_in(4..8, 9), 3..5);
    assert_eq!(map.entries_in(0..12, 9), 0..9);
}

#[test]
fn groups_draw_only_one_entry_a_line_over_the_whole_source() {
    let g = groups();
    assert!(LineMap::new(1, Some(&g), 9).groups().is_some());
    assert!(LineMap::new(1, Some(&g), 10).groups().is_none(), "short");
    assert!(LineMap::new(3, Some(&g), 9).groups().is_none(), "packed");
    assert!(
        LineMap::new(1, Some(&Groups::default()), 0)
            .groups()
            .is_none()
    );
}

#[test]
fn a_regroup_changes_the_fingerprint_and_the_same_groups_keep_it() {
    let a = groups();
    assert_eq!(a.fingerprint(), groups().fingerprint());
    let b = Groups::new([Group::new(10, "Folders", 4), Group::new(40, "Other", 5)]);
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn the_geometry_puts_entries_below_their_headers_and_headers_hold_none() {
    let g = geometry();
    assert_eq!(g.rect_of(0).y, H, "under the first header");
    assert_eq!(g.rect_of(3).y, 5.0 * H);
    assert_eq!(g.index_at(Point::new(10.0, 0.5 * H)), None, "a header");
    assert_eq!(g.index_at(Point::new(10.0, 4.5 * H)), None);
    assert_eq!(g.index_at(Point::new(10.0, 5.5 * H)), Some(3));
    assert_eq!(g.index_at(Point::new(10.0, 12.5 * H)), None, "past the end");
    assert_eq!(g.index_at(Point::new(-1.0, 5.5 * H)), None);
}

#[test]
fn a_band_over_a_header_takes_the_entries_either_side() {
    let g = geometry();
    let band = g.range_in(Rect::new(0.0, 3.5 * H, 50.0, 2.0 * H));
    assert_eq!(band, IndexRanges::from(2..4));
    let header = g.range_in(Rect::new(0.0, 4.2 * H, 50.0, 0.5 * H));
    assert!(header.is_empty(), "a header alone holds no entry");
    let point = g.range_in(Rect::new(10.0, 5.5 * H, 0.0, 0.0));
    assert_eq!(point, IndexRanges::from(3..4));
}

#[test]
fn up_and_down_step_over_headers() {
    let g = geometry();
    assert_eq!(g.neighbor(2, Direction::Down), Some(3));
    assert_eq!(g.neighbor(3, Direction::Up), Some(2));
    assert_eq!(g.neighbor(8, Direction::Down), None);
    assert_eq!(g.neighbor(3, Direction::Left), None);
}

/// A deviation from Strata, whose pages don't leave room for a stuck header.
#[test]
fn a_page_leaves_a_row_for_the_stuck_header_and_one_of_overlap() {
    assert_eq!(geometry().page_step(), 3, "5 rows fit");
    let short = GroupedRows::new(groups(), H, 300.0).viewport_height(H);
    assert_eq!(short.page_step(), 1);
}

#[test]
fn the_stuck_header_is_the_top_groups_and_the_next_header_pushes_it_out() {
    let map = LineMap::new(1, Some(&groups()), 9);
    assert_eq!(map.top_group(0.0, H), Some(0));
    assert_eq!(map.stuck_extent(0.0, H), H);
    assert_eq!(map.top_group(3.5 * H, H), Some(0));
    assert_eq!(
        map.stuck_extent(3.5 * H, H),
        0.5 * H,
        "half pushed by the next header"
    );
    assert_eq!(map.top_group(4.0 * H, H), Some(1));
    assert_eq!(map.stuck_extent(4.0 * H, H), H);
    assert_eq!(LineMap::Packed(1).stuck_extent(4.0 * H, H), 0.0);
}

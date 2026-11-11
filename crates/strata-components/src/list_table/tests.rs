//! The table's rules as plain data: the layout and widths, the sizing rule, where a dragged
//! heading drops, the sort a click asks for, hit regions and groups. `tests/runtime.rs` drives
//! the table in a headless runtime.

mod perf;
mod runtime;

use std::sync::Arc;

use rune::Align;
use strata_kit::look::Look;
use strata_kit::shape::LookParams;

use super::resolve::{Inputs, resolve};
use super::*;
use crate::collection::element::Region;
use crate::contract::Point;

fn id(s: &str) -> ColumnId {
    ColumnId::new(s)
}

fn ids(v: &[&str]) -> Vec<ColumnId> {
    v.iter().map(|s| id(s)).collect()
}

/// Strata's five columns, with its default widths and minimums
/// (`src/ui/browser_modes/list_columns.rs`).
fn strata() -> Vec<Column<()>> {
    let cell = |_: &(), _: &RowCx<'_>| Cell::text("");
    vec![
        Column::primary("name", "Name")
            .width(160.0)
            .min(160.0)
            .sortable(),
        Column::new("mode", "Mode", cell).width(160.0).min(80.0),
        Column::new("size", "Size", cell)
            .width(90.0)
            .min(70.0)
            .align(Align::End)
            .sortable(),
        Column::new("type", "Type", cell)
            .width(120.0)
            .min(80.0)
            .sortable(),
        Column::new("modified", "Modified", cell)
            .width(150.0)
            .min(110.0)
            .sortable(),
    ]
}

fn defaults() -> Vec<ColumnId> {
    ids(&["name", "mode", "size", "type", "modified"])
}

fn resolved(
    layout: &ColumnLayout,
    widths: &ColumnWidths,
    viewport: f32,
    scale: f32,
) -> resolve::Resolved {
    resolve(
        &strata(),
        &Inputs {
            layout,
            widths,
            live: None,
            viewport,
            lead: 0.0,
            scale,
        },
    )
}

fn widths_of(r: &resolve::Resolved) -> Vec<(String, f32)> {
    r.cols
        .iter()
        .map(|p| (p.id.as_str().to_owned(), p.w))
        .collect()
}

// The layout.

/// column-layout: "On load, duplicate columns are dropped and missing ones are appended in
/// default order", and "A hand-edited `list_hidden_columns` that names `name` loads with Name
/// shown" (the table's half: the host reads the file).
#[test]
fn a_saved_layout_drops_unknown_and_repeated_ids_appends_missing_ones_and_never_hides_name() {
    let layout = ColumnLayout::normalize(
        &defaults(),
        Some(&id("name")),
        ["type", "type", "bogus"],
        ["name", "size", "size", "bogus"],
    );
    assert_eq!(
        layout.order(),
        ids(&["type", "name", "mode", "size", "modified"])
    );
    assert!(layout.is_visible(&id("name")), "Name can't be hidden");
    assert_eq!(layout.hidden().cloned().collect::<Vec<_>>(), ids(&["size"]));
    assert_eq!(
        layout.visible().cloned().collect::<Vec<_>>(),
        ids(&["type", "name", "mode", "modified"])
    );
}

#[test]
fn moving_a_column_puts_it_before_its_target_or_after_the_last() {
    let layout = ColumnLayout::new(defaults());
    let site = |t: &str, after| DropSite {
        target: id(t),
        after,
    };
    assert_eq!(
        layout
            .moved(&id("mode"), &site("type", false))
            .unwrap()
            .order(),
        ids(&["name", "size", "mode", "type", "modified"])
    );
    assert_eq!(
        layout
            .moved(&id("name"), &site("modified", true))
            .unwrap()
            .order(),
        ids(&["mode", "size", "type", "modified", "name"])
    );
    assert_eq!(
        layout.moved(&id("mode"), &site("size", false)),
        None,
        "where it already is"
    );
}

// The sizing rule.

/// list: "Name takes the width left after the metadata columns until it is resized by hand;
/// below the minimum widths the table scrolls horizontally" (the widths; `horizontal-scroll.json`
/// checks that the wheel scrolls them sideways).
#[test]
fn the_fill_column_takes_what_the_others_leave_until_resized_and_overflows_below_its_width() {
    let layout = ColumnLayout::new(defaults());
    let wide = resolved(&layout, &ColumnWidths::new(), 1200.0, 1.0);
    assert_eq!(
        widths_of(&wide),
        [
            ("name".into(), 680.0),
            ("mode".into(), 160.0),
            ("size".into(), 90.0),
            ("type".into(), 120.0),
            ("modified".into(), 150.0)
        ]
    );
    let narrow = resolved(&layout, &ColumnWidths::new(), 400.0, 1.0);
    assert_eq!(narrow.cols[0].w, 160.0, "at least its own width");
    assert_eq!(narrow.content, 680.0, "wider than the viewport: it scrolls");
    let set = ColumnWidths::new().with(&id("name"), 200.0);
    let resized = resolved(&layout, &set, 1200.0, 1.0);
    assert_eq!(
        resized.cols[0].w, 200.0,
        "resized by hand, it stops filling"
    );
    let small = ColumnWidths::new().with(&id("size"), 10.0);
    assert_eq!(
        resolved(&layout, &small, 1200.0, 1.0).cols[2].w,
        70.0,
        "a width below the minimum is clamped"
    );
}

/// A saved width that isn't a finite number takes the default, and one past all reason the
/// ceiling.
#[test]
fn a_saved_width_that_isnt_a_number_or_is_absurd_is_bounded() {
    let layout = ColumnLayout::new(defaults());
    let size = |w: f32| {
        let widths = ColumnWidths::new().with(&id("size"), w);
        resolved(&layout, &widths, 1200.0, 1.0).cols[2].w
    };
    assert_eq!(size(f32::NAN), 90.0, "the default");
    assert_eq!(size(f32::INFINITY), 90.0, "not finite: the default");
    assert_eq!(size(1e9), WIDTH_CEILING);
    assert_eq!(size(-5.0), 70.0, "the minimum");
}

/// The table's own normalize knows its default order and its primary column.
#[test]
fn a_tables_normalize_knows_its_columns_and_its_primary_one() {
    let table = ListTable::new(
        "t",
        strata(),
        |_: &()| unreachable!("not built"),
        |_: &mut ()| unreachable!("not built"),
    );
    let layout = table.normalize(["size", "size"], ["name", "mode"]);
    assert_eq!(
        layout.order(),
        ids(&["size", "name", "mode", "type", "modified"])
    );
    assert!(layout.is_visible(&id("name")) && !layout.is_visible(&id("mode")));
}

/// Owner ruling (Q4): widths follow the text size, and stay in base units.
#[test]
fn a_larger_text_size_widens_the_columns_by_its_factor() {
    let base = Look::fallback(false);
    let big = Look::fallback(false).with_params(&LookParams {
        text_size: 26,
        ..LookParams::default()
    });
    assert_eq!(text_scale(&base), 1.0);
    assert_eq!(text_scale(&big), 2.0);
    let layout = ColumnLayout::new(defaults());
    let widths = ColumnWidths::new().with(&id("size"), 100.0);
    let at = |scale| resolved(&layout, &widths, 0.0, scale);
    let (one, two) = (at(text_scale(&base)), at(text_scale(&big)));
    for (a, b) in one.cols.iter().zip(&two.cols) {
        assert_eq!(b.w, a.w * 2.0, "{:?}", a.id);
    }
    assert_eq!(widths.get(&id("size")), Some(100.0), "kept in base units");
}

// Reordering.

/// column-layout: "Dragging Mode 180 px to the right and releasing gives the order Name, Size,
/// Mode, Type, Modified" (the drop rule), "The line sits on the left edge of the heading the
/// column would precede", "For a drop at the end, the line sits on the last heading's right
/// edge", and "A drop on the column's current position shows no line and changes nothing".
#[test]
fn a_dragged_heading_drops_before_the_first_other_heading_whose_middle_is_right_of_it() {
    let layout = ColumnLayout::new(defaults());
    // Name resized to its default 160, so the positions are Strata's at their defaults.
    let r = resolved(
        &layout,
        &ColumnWidths::new().with(&id("name"), 160.0),
        0.0,
        1.0,
    );
    // Mode spans 160 to 320; pressed at its middle and dragged 180 to the right.
    let site = r.drop_site(&id("mode"), 240.0 + 180.0).unwrap();
    assert_eq!(
        site,
        DropSite {
            target: id("type"),
            after: false
        }
    );
    assert_eq!(
        layout.moved(&id("mode"), &site).unwrap().order(),
        ids(&["name", "size", "mode", "type", "modified"])
    );
    assert_eq!(
        r.drop_site(&id("mode"), 2000.0),
        Some(DropSite {
            target: id("modified"),
            after: true
        })
    );
    assert_eq!(
        r.drop_site(&id("mode"), 240.0 + 11.0),
        None,
        "its own place"
    );
    assert_eq!(r.drop_site(&id("modified"), 2000.0), None, "already last");
}

// Sorting.

/// list: "Clicking the Name, Size, Type, or Modified heading sorts by that field ascending;
/// clicking the current sort heading reverses it" (the direction a click asks for).
#[test]
fn a_click_asks_for_the_other_way_on_the_sorted_column_and_ascending_elsewhere() {
    let shown = SortShown {
        column: id("size"),
        dir: SortDir::Ascending,
    };
    assert_eq!(
        next_direction(Some(&shown), &id("size")),
        SortDir::Descending
    );
    assert_eq!(
        next_direction(Some(&shown), &id("name")),
        SortDir::Ascending
    );
    assert_eq!(next_direction(None, &id("size")), SortDir::Ascending);
}

// Hit regions.

/// pointer-intent: "In List, unused Name-column space and its row padding are blank, even when
/// Name is not the first column; other column cells count as item content".
#[test]
fn the_name_columns_blank_space_and_the_padding_are_blank_and_other_cells_are_content() {
    let look = Look::fallback(false);
    let mut layout = ColumnLayout::new(defaults());
    layout = layout
        .moved(
            &id("name"),
            &DropSite {
                target: id("modified"),
                after: true,
            },
        )
        .unwrap();
    let mut r = resolved(&layout, &ColumnWidths::new(), 1200.0, 1.0);
    r.lead = 10.0;
    let name = r.cols.iter().find(|p| p.primary).unwrap().clone();
    let at = |x: f32| rows::region(&r, &look, "a.txt", Point::new(r.lead + x, 5.0));
    let icon = look.icon_size.md + look.space.sm;
    assert_eq!(at(name.x + 2.0), Region::Content, "the icon");
    assert_eq!(at(name.x + icon + 2.0), Region::Name, "the drawn name");
    assert_eq!(
        at(name.x + name.w - 20.0),
        Region::Gutter,
        "the rest of Name"
    );
    assert_eq!(at(r.cols[0].x + 30.0), Region::Content, "Mode's cell");
    assert_eq!(
        rows::region(&r, &look, "a.txt", Point::new(4.0, 5.0)),
        Region::Gutter,
        "the row's padding"
    );
}

// Groups.

#[test]
fn groups_are_runs_of_one_key_and_a_key_that_comes_back_makes_a_group_of_its_own() {
    let keys = [1u64, 1, 2, 2, 2, 3, 1];
    let groups = groups_from_runs(keys.len(), |i| {
        (keys[i], Arc::from(format!("g{}", keys[i])))
    });
    let got: Vec<(String, usize)> = groups
        .groups()
        .iter()
        .map(|g| (g.label.to_string(), g.len))
        .collect();
    assert_eq!(
        got,
        [
            ("g1".into(), 2),
            ("g2".into(), 3),
            ("g3".into(), 1),
            ("g1".into(), 1)
        ]
    );
    let mut k: Vec<u64> = groups.groups().iter().map(|g| g.key).collect();
    k.sort_unstable();
    k.dedup();
    assert_eq!(k.len(), 4, "keys stay unique");
    assert_eq!(groups.len(), keys.len());
}

/// Whatever keys the host gives: a key that comes back never takes one another group has.
#[test]
fn a_key_that_comes_back_never_takes_another_groups_key() {
    let keys = [0u64, 1, 0, 0x9e37_79b9_7f4a_7c15, 1, 2];
    let groups = groups_from_runs(keys.len(), |i| (keys[i], Arc::from("g")));
    let mut k: Vec<u64> = groups.groups().iter().map(|g| g.key).collect();
    let n = k.len();
    k.sort_unstable();
    k.dedup();
    assert_eq!(k.len(), n, "unique: {k:?}");
    assert_eq!(n, keys.len(), "a group per run");
}

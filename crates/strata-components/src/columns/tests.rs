//! Unit tests of the pure parts: the reveal rule, the width rules and the drawer fit.

mod perf;
mod reveal;

use strata_kit::look::Look;

use super::reveal::ColumnSpan;
use super::width::{STANDARD, dragged, drawn, sane};

fn span(left: f32, right: f32, trailing: f32) -> ColumnSpan {
    ColumnSpan {
        left,
        right,
        trailing,
    }
}

#[test]
fn a_column_clipped_even_a_little_is_revealed_whole_and_a_shown_one_stays() {
    // browser/view-modes/columns: A keyboard move reveals a column clipped by up to 4 px fully
    // and leaves an already visible column in place.
    let page = 900.0;
    // Clipped by 4 on the right: the strip moves 4.
    assert_eq!(
        span(600.0, 904.0, 0.0).reveal_target(0.0, page, 0.0, 300.0),
        4.0
    );
    // Clipped by 3 on the left, in a strip as wide as it: back to its left edge.
    assert_eq!(
        span(300.0, 600.0, 0.0).reveal_target(303.0, 300.0, 0.0, 600.0),
        300.0
    );
    // Shown: stays, wherever the strip is.
    assert_eq!(
        span(300.0, 600.0, 0.0).reveal_target(100.0, page, 0.0, 300.0),
        100.0
    );
    // A child column comes along where there is room, never at the column's expense.
    assert_eq!(
        span(600.0, 1_000.0, 300.0).reveal_target(0.0, page, 0.0, 400.0),
        400.0
    );
    assert_eq!(
        span(600.0, 1_000.0, 900.0).reveal_target(0.0, page, 0.0, 1_000.0),
        600.0
    );
}

#[test]
fn a_column_wider_than_the_strip_stays_while_it_fills_it_else_goes_to_its_left_edge() {
    // browser/view-modes/columns: A column wider than the viewport that already fills it stays
    // in place; a partly off-screen one scrolls to its left edge.
    let wide = span(300.0, 1_400.0, 0.0);
    assert_eq!(wide.reveal_target(400.0, 900.0, 0.0, 500.0), 400.0);
    assert_eq!(wide.reveal_target(0.0, 900.0, 0.0, 500.0), 300.0);
}

#[test]
fn widths_never_draw_below_a_standard_column_and_follow_the_text_size() {
    // browser/view-modes/columns: The width cap never goes below the 300 px standard column,
    // scaled by text size. And: Dragging a column's right edge resizes only that column ...
    // never below 300 px.
    assert_eq!(drawn(200.0, 1.0), STANDARD);
    assert_eq!(drawn(400.0, 1.5), 600.0);
    assert_eq!(dragged(300.0, -100.0, 1.0, None), STANDARD);
    assert_eq!(
        dragged(300.0, 150.0, 1.5, None),
        400.0,
        "travel in points, width in base"
    );
    assert_eq!(
        dragged(300.0, 900.0, 1.0, Some(700.0)),
        700.0,
        "the area stops it"
    );
    assert_eq!(
        dragged(300.0, 900.0, 1.0, Some(100.0)),
        STANDARD,
        "never a cap below one"
    );
    assert_eq!(sane(f32::NAN), None);
    assert_eq!(sane(-5.0), None);
    assert_eq!(sane(120.0), Some(STANDARD));
    assert_eq!(sane(1_729.0), Some(1_729.0));
}

#[test]
fn the_fit_tells_the_drawer_the_columns_through_the_active_one_and_after_it() {
    use crate::preview_drawer::DrawerRules;
    let look = Look::fallback(false);
    let mut st = super::ColumnsState::new("t", crate::scroll::Autoscroll::new());
    let mut tx = rune::Tx::default();
    let source = crate::contract::ListSource::new(Vec::new());
    for _ in 0..3 {
        st.push(
            super::Opening::new(&source, crate::collection::selection::Listing::Complete),
            super::Focus::No,
            &look,
            &mut tx,
        );
    }
    st.set_active(1, &look, &mut tx);
    let fit = super::fit::columns_fit(&st, &look, &DrawerRules::STRATA);
    assert_eq!(fit.navigated, 600.0);
    assert_eq!(fit.keep_focused, 300.0);
    assert_eq!(fit.trailing, 300.0);
    assert_eq!(fit.standard_column, 300.0);
}

#[test]
fn a_new_column_slides_in_and_a_sibling_fades_in_its_place() {
    // browser/view-modes/columns: With a folder open, opening a sibling fades the new column in
    // where the old one stood, with no 24 px slide. Deeper columns of the old branch shrink
    // away.
    use super::state::Entrance;
    let look = Look::fallback(false);
    let mut st = super::ColumnsState::new("t", crate::scroll::Autoscroll::new());
    let mut tx = rune::Tx::default();
    let source = crate::contract::ListSource::new(Vec::new());
    let open = super::Opening::new(&source, crate::collection::selection::Listing::Complete);
    for _ in 0..3 {
        st.push(open, super::Focus::Column, &look, &mut tx);
    }
    assert_eq!(st.columns[2].entrance, Entrance::Slide);
    let slot = st.columns[1].slot;
    st.swap_from(1, open, super::Focus::Column, &look, &mut tx);
    assert_eq!(st.len(), 2, "the deeper column goes");
    assert_eq!(st.columns[1].entrance, Entrance::Fade);
    assert_eq!(st.columns[1].slot, slot, "in the old one's place");
    assert_ne!(st.column_key(1), Some("t-col-2".into()), "a new collection");
}

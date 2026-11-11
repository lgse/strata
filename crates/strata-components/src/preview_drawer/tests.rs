//! The width rules and the state that outlives a close. `docs/preview-panel-layout.md` in
//! Strata states each rule; the tests name the bullet they hold.

use rune::{Env, Runtime, el};
use strata_kit::look::Look;

use super::*;

const R: DrawerRules = DrawerRules::STRATA;

fn columns(navigated: f32, keep: f32) -> Arrangement {
    Arrangement::Columns(ColumnsFit::new(&R, navigated, keep))
}

#[test]
fn in_columns_the_automatic_width_fills_the_space_right_of_the_focused_column() {
    // 1 point of divider line, then 600 of columns: the rest is the preview's.
    let s = solve(&R, &columns(600.0, 300.0), 1201.0, None);
    assert_eq!(s.width, 600.0);
    assert!(s.fits);
}

#[test]
fn the_columns_minimum_is_one_standard_column_and_yields_to_the_focused_column() {
    // Only 200 free beside the navigated columns: the preview takes its 300 minimum.
    let s = solve(&R, &columns(800.0, 300.0), 1001.0, None);
    assert_eq!(s.width, 300.0);
    // The focused column needs 750 to stay whole: the minimum yields down to what is left.
    let s = solve(&R, &columns(800.0, 750.0), 1001.0, None);
    assert_eq!(s.width, 250.0);
}

#[test]
fn a_dragged_width_in_columns_is_a_session_minimum_the_preview_still_fills_past() {
    let fit = columns(400.0, 300.0);
    // Wider free space than the manual width: the preview fills it.
    assert_eq!(solve(&R, &fit, 1201.0, Some(500.0)).width, 800.0);
    // Less free space: the columns yield down to the manual width.
    assert_eq!(solve(&R, &fit, 801.0, Some(500.0)).width, 500.0);
}

#[test]
fn in_icons_and_list_the_automatic_width_is_ninety_percent_of_half_within_its_bounds() {
    for a in [Arrangement::Icons, Arrangement::List] {
        assert_eq!(solve(&R, &a, 1001.0, None).width, 450.0);
        // The 240 floor, while the files keep one standard column.
        assert_eq!(solve(&R, &a, 541.0, None).width, 240.0);
        // The 3000 ceiling.
        assert_eq!(solve(&R, &a, 10_001.0, None).width, 3000.0);
    }
}

#[test]
fn in_icons_and_list_a_dragged_width_is_the_width_and_a_window_resize_keeps_it() {
    for available in [900.0, 1200.0, 1600.0] {
        assert_eq!(
            solve(&R, &Arrangement::List, available, Some(420.0)).width,
            420.0
        );
    }
}

#[test]
fn the_content_hides_when_less_than_240_would_remain_beside_the_focused_column() {
    // Columns: 1 + 300 standard + 239 left over.
    let fit = Arrangement::Columns(ColumnsFit::new(&R, 300.0, 300.0));
    assert!(!solve(&R, &fit, 540.0, None).fits);
    assert!(solve(&R, &fit, 541.0, None).fits);
    // Icons and List keep one standard column for the files.
    assert!(!solve(&R, &Arrangement::Icons, 540.0, None).fits);
    assert!(solve(&R, &Arrangement::Icons, 541.0, None).fits);
    // The narrow test counts the focused column at most one standard column.
    let wide = Arrangement::Columns(ColumnsFit::new(&R, 500.0, 500.0).keep_focused_standard(300.0));
    assert!(solve(&R, &wide, 600.0, None).fits);
}

#[test]
fn the_reserved_slot_lends_trailing_columns_its_width_and_shrinks_to_zero() {
    let fit = ColumnsFit::new(&R, 400.0, 300.0).trailing(250.0);
    let s = solve(&R, &Arrangement::Columns(fit), 1001.0, None);
    // The 600 point preview and the divider's line, less the 250 lent.
    assert_eq!(s.empty_slot, 351.0);
    // A narrow window: the slot shrinks to nothing instead of going.
    let s = solve(
        &R,
        &Arrangement::Columns(ColumnsFit::new(&R, 400.0, 400.0)),
        401.0,
        None,
    );
    assert_eq!(s.empty_slot, 0.0);
}

#[test]
fn a_manual_width_is_at_least_one_standard_column() {
    let mut st = DrawerState::new();
    st.set_manual_width(&R, 120.0);
    assert_eq!(st.manual_width(), Some(300.0));
    assert_eq!(st.demand(&R), 300.0);
    assert_eq!(DrawerState::new().demand(&R), 240.0);
}

/// Runs `f` on a drawer in a handler, as a host does.
fn in_handler(
    st: DrawerState,
    f: impl FnOnce(&mut DrawerState, &mut rune::Tx) + 'static,
) -> DrawerState {
    let mut rt = Runtime::new(
        st,
        |_: &DrawerState, _: &mut rune::Cx<DrawerState>| el(),
        (10.0, 10.0),
        Env::default(),
    );
    rt.frame(0.0);
    rt.update(f);
    rt.state.clone()
}

#[test]
fn closing_keeps_the_manual_width_and_ends_find() {
    let mut st = DrawerState::new();
    st.set_manual_width(&R, 480.0);
    let st = in_handler(st, |st, tx| {
        let look = Look::fallback(false);
        st.open(&look, tx);
        st.open_find();
        st.find.query.replace_text("needle");
        st.close(&look, tx);
    });
    assert!(!st.is_open());
    assert_eq!(
        st.manual_width(),
        Some(480.0),
        "reopening restores the width"
    );
    assert!(!st.is_finding());
    assert_eq!(st.find_query(), "", "closing the drawer clears the query");
}

#[test]
fn reopening_find_keeps_the_query_selected() {
    let mut st = DrawerState::new();
    st.open_find();
    st.find.query.replace_text("needle");
    st.find.open = false;
    st.open_find();
    assert!(st.find.query.focused);
    assert_eq!(st.find.query.selected(), "needle");
}

#[test]
fn a_closed_drawer_is_never_suspended_and_reopening_reconciles_with_the_host() {
    let mut st = DrawerState::new();
    st.open_at_once();
    st.suspended = true;
    assert!(st.is_suspended());
    let st = in_handler(st, |st, tx| st.close(&Look::fallback(false), tx));
    assert!(!st.is_suspended(), "nothing to pause while closed");
    // The host last heard Suspend(true); the next open's fit marker owes it Suspend(false).
    assert!(st.suspended);
}

#[test]
fn the_right_pane_tells_c2_what_the_empty_slot_keeps() {
    let fit = ColumnsFit::new(&R, 400.0, 300.0).trailing(250.0);
    let pane = right_pane(&DrawerState::new(), &R, &fit, 1001.0);
    assert_eq!(
        (pane.width, pane.empty_slot, pane.preview_shown),
        (600.0, 351.0, false)
    );
}

#[test]
fn in_columns_a_wider_standard_column_raises_the_manual_floor() {
    let fit = Arrangement::Columns(ColumnsFit::new(&R, 400.0, 300.0).standard_column(340.0));
    let s = solve(&R, &fit, 701.0, Some(300.0));
    assert_eq!((s.width, s.min), (340.0, 340.0));
}

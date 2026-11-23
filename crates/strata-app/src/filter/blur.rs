// SPDX-License-Identifier: MIT

use rune::Tx;

use super::{bar, place::Where, route};
use crate::{
    App,
    commands::{Holds, Situation},
};

/// Whether focus is currently inside the filtered column (its field or its rows).
fn in_filtered_column(app: &App, depth: usize, focused: Option<&str>) -> bool {
    let field = bar(app).field_key();
    focused == Some(field.as_str())
        || (focused == Some(crate::columns::VIEW) && app.columns.active() == depth)
}

/// Overlays and transitions that keep a Columns filter open while focus is outside its column.
fn keeps_filter_open(app: &App, tx: &Tx) -> bool {
    if app.filter.refocus_field || app.filter.state.tenxer() || app.sort.holds_focus() {
        return true;
    }
    matches!(
        Situation::of(app, tx).holds(),
        Some(Holds::Modal { .. } | Holds::Menu | Holds::Edit | Holds::Prompt)
    )
}

pub(super) fn follow(app: &mut App, tx: &mut Tx) -> bool {
    let Some(depth) = app
        .filter
        .open_in
        .as_ref()
        .filter(|open| open.place == Where::Columns && app.filter.state.is_open())
        .map(|open| open.depth)
    else {
        app.filter.seen_in_column = false;
        return false;
    };
    let focused = tx.focused();
    if in_filtered_column(app, depth, focused) {
        app.filter.seen_in_column = true;
        return false;
    }
    if !app.filter.seen_in_column || focused.is_none() || keeps_filter_open(app, tx) {
        return false;
    }
    let polled = app.filter.state.close();
    route(app, polled, tx);
    true
}

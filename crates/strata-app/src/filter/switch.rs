// SPDX-License-Identifier: MIT

//! A view switch keeps the filter: GTK's `set_view_mode` captures the active pane's filter
//! (`ActivePaneFilter`: its query, and whether it is revealed) and restores it in the view it
//! shows, and `restore_view_switch_focus` gives the keyboard back to the field when it had it,
//! else to the results while a query shows them (`src/ui/browser/view_modes.rs`).
//!
//! Here the results themselves move with their selection, so nothing is searched again.
//! GTK captures the active column's filter, so a filter open in another Columns column closes.

use rune::Tx;
use strata_components::collection::selection::Position;

use super::{OpenIn, place::Where, route};
use crate::{App, listing::Column};

/// The view shown changed: the filter follows it into the folder it shows, or closes.
pub(super) fn follow(app: &mut App, tx: &mut Tx) {
    let Some(open) = app.filter.open_in.clone() else {
        // Rune keeps a field's focus when it leaves the tree.
        app.filter.state.edit.focused = false;
        return;
    };
    let Some(place) = Where::shown(app).filter(|place| *place != open.place) else {
        return;
    };
    let field = app.filter.state.field_focused();
    let carried = take(app, open.place, open.depth);
    let kept = app.filter.state.is_open()
        && app.browser.active_depth() == Some(open.depth)
        && place.targeted(app) == Some(open.depth)
        && app.browser.location_at(open.depth).as_ref() == Some(&open.folder);
    if !kept {
        let polled = app.filter.state.close();
        route(app, polled, tx);
        app.filter.state.edit.focused = false;
        return;
    }
    app.filter.serial += 1;
    app.filter.open_in = Some(OpenIn { place, ..open });
    app.columns
        .hold_header((place == Where::Columns).then_some(open.depth));
    if let Some((hits, position)) = carried {
        place.show_hits(app, open.depth, hits, false, tx);
        restore(app, place, open.depth, &position);
    }
    refocus(app, place, field, tx);
}

/// The results the view that left showed, with their selection.
fn take(app: &mut App, place: Where, depth: usize) -> Option<(Column, Position)> {
    match place {
        Where::Columns => app.columns.take_hits(depth),
        _ => app.filter.pane.take(),
    }
}

fn restore(app: &mut App, place: Where, depth: usize, position: &Position) {
    match place {
        Where::Columns => app.columns.restore_hits_selection(depth, position),
        _ => app.filter.pane.restore(position),
    }
}

/// The keyboard where it was: the field, or the results while a query shows them.
fn refocus(app: &mut App, place: Where, field: bool, tx: &mut Tx) {
    if field {
        app.filter.state.focus_field();
        tx.focus(super::bar(app).field_key(), false);
        return;
    }
    app.filter.state.edit.focused = false;
    if !app.filter.state.text().trim().is_empty() {
        tx.focus(place.view_key(), true);
    }
}

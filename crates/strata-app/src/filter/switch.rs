// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_components::collection::selection::Position;

use super::{HitsUpdate, OpenIn, place::Where, route};
use crate::{App, listing::Column};

/// The view shown changed: the filter follows it into the folder it shows, or closes.
pub(super) fn follow(app: &mut App, tx: &mut Tx) {
    let Some(open) = app.filter.open_in.clone() else {
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
    app.filter.seen_in_column = false;
    app.filter.state.set_pick_last(place == Where::Columns);
    let next = OpenIn { place, ..open };
    app.filter.open_in = Some(next.clone());
    if place == Where::Columns {
        app.columns.activate_for_filter(next.depth, &app.look, tx);
    }
    app.columns
        .hold_header((place == Where::Columns).then_some(next.depth));
    if let Some((hits, position)) = carried {
        next.show_hits(app, HitsUpdate::plain(hits, false), tx);
        restore(app, place, next.depth, &position);
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
        tx.focus_when_shown(super::bar(app).field_key(), false);
        return;
    }
    app.filter.state.edit.focused = false;
    if !app.filter.state.text().trim().is_empty() {
        tx.focus_when_shown(place.view_key(), true);
    }
}

pub(super) fn tab_hidden(app: &mut App, tx: &mut Tx) {
    let Some(open) = app.filter.open_in.as_ref() else {
        return;
    };
    let field = app.filter.state.field_focused();
    if open.place == Where::Columns {
        let polled = app.filter.state.close();
        route(app, polled, tx);
    }
    app.filter.state.edit.focused = false;
    app.filter.back_to_field = field && app.filter.state.is_open();
}

pub(super) fn tab_shown(app: &mut App) {
    if !std::mem::take(&mut app.filter.back_to_field) {
        return;
    }
    let listed = app.filter.open_in.as_ref().is_some_and(|open| {
        app.filter.state.is_open()
            && app.browser.location_at(open.depth).as_ref() == Some(&open.folder)
            && open.place.listed(app, open.depth)
    });
    if listed {
        app.filter.state.focus_field();
    } else {
        app.filter.refocus_field = true;
    }
}

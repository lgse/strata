// SPDX-License-Identifier: MIT

//! Dragging the built-in places and the pins into a new order, as the GTK sidebar saves it
//! (`reorder_place` and `reorder_pinned_place` in `src/ui/window/sidebar_reorder.rs`): the
//! places into the saved `sidebar_order`, the pins into GTK's bookmarks file through the store
//! both interfaces share. A fix there has to be carried here until the GTK sidebar is deleted.
//!
//! C8 reports a move as the row's index in its block before the drag and after it. GTK moves a
//! row next to the row it was dropped on, so a move is turned into that: the row's neighbour in
//! the new order, and whether it lands after it.

use strata_core::model::Location;
use strata_fs::adapters::bookmarks;

use super::{places, relist};
use crate::{App, shell};

/// The row the moved one lands next to in the block's new order, by index, and whether it
/// lands after that row. None when the move leaves the order as it was.
pub(super) fn neighbour(len: usize, from: usize, to: usize) -> Option<(usize, bool)> {
    if from == to || from >= len || to >= len || len < 2 {
        return None;
    }
    // The block's order once the row is moved, by the indices before it.
    let mut order: Vec<usize> = (0..len).collect();
    let moved = order.remove(from);
    order.insert(to, moved);
    Some(match to {
        0 => (order[1], false),
        _ => (order[to - 1], true),
    })
}

/// `reorder_places`: `source` next to `target` in the saved order. False when nothing moved.
pub(super) fn reorder_places(
    order: &mut Vec<&'static str>,
    source: &str,
    target: &str,
    after: bool,
) -> bool {
    if source == target {
        return false;
    }
    let Some(source_index) = order.iter().position(|place| *place == source) else {
        return false;
    };
    let source = order.remove(source_index);
    let Some(target_index) = order.iter().position(|place| *place == target) else {
        order.insert(source_index, source);
        return false;
    };
    order.insert(target_index + usize::from(after), source);
    true
}

/// `reorder_pinned_places`: the pin at `source` next to the one at `target`. False when
/// nothing moved.
pub(super) fn reorder_pinned_places(
    places: &mut Vec<(Location, String)>,
    source: usize,
    target: usize,
    after: bool,
) -> bool {
    if source == target || source >= places.len() || target >= places.len() {
        return false;
    }
    let place = places.remove(source);
    // `target` is an index before the removal: one after `source` slides left.
    let target = target - usize::from(target > source);
    let destination = (target + usize::from(after)).min(places.len());
    if destination == source {
        places.insert(source, place);
        return false;
    }
    places.insert(destination, place);
    true
}

/// The built-in place at `from` in the block was dragged to `to`: the saved order follows.
pub(super) fn move_place(app: &mut App, from: usize, to: usize) {
    let ids: Vec<String> = app
        .sidebar
        .listed
        .places
        .iter()
        .map(|place| place.id.clone())
        .collect();
    let Some((target, after)) = neighbour(ids.len(), from, to) else {
        return;
    };
    let mut order = places::resolve_place_order(&shell::preferences(app).sidebar_order);
    if !reorder_places(&mut order, &ids[from], &ids[target], after) {
        return;
    }
    let order: Vec<String> = order.into_iter().map(str::to_owned).collect();
    shell::saved(app).change(move |settings| settings.sidebar_order = order);
    relist(app);
}

/// The pin at `from` among those listed was dragged to `to`: the bookmarks file follows, and
/// the sidebar hears it from the store. The pins are matched by location in the file as it is
/// then, as GTK merges into the shared file and never into a window's copy.
pub(super) fn move_pin(app: &App, from: usize, to: usize) {
    let listed = &app.sidebar.listed.pins;
    let Some((target, after)) = neighbour(listed.len(), from, to) else {
        return;
    };
    let location = |index: usize| {
        let saved: usize = listed.get(index)?.id.parse().ok()?;
        app.sidebar
            .pins
            .get(saved)
            .map(|(location, _)| location.clone())
    };
    let (Some(source), Some(target)) = (location(from), location(target)) else {
        return;
    };
    let edit = move |contents: &[u8]| {
        let mut places = bookmarks::parse_pinned_places(contents);
        let position = |wanted: &Location| places.iter().position(|(pinned, _)| pinned == wanted);
        let moved = match (position(&source), position(&target)) {
            (Some(source), Some(target)) => {
                reorder_pinned_places(&mut places, source, target, after)
            }
            _ => false,
        };
        moved.then(|| bookmarks::serialize_pinned_places(&places).into_bytes())
    };
    glib::MainContext::default().spawn_local(async move {
        if let Err(error) = bookmarks::update(edit).await {
            tracing::warn!(%error, "the pinned folders could not be saved");
        }
    });
}

#[cfg(test)]
mod tests;

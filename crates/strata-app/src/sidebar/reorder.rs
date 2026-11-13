// SPDX-License-Identifier: MIT

//! Dragging the built-in places and the pins into a new order, as the GTK sidebar saves it
//! (`reorder_place`, `reorder_pinned_place` and `reorder_pinned_into` in
//! `src/ui/window/sidebar_reorder.rs` and `sidebar_pins.rs`): the places into the saved
//! `sidebar_order`, the pins into GTK's bookmarks file through the store both interfaces share.
//! A fix there has to be carried here until the GTK sidebar is deleted.
//!
//! C8 reports where a drop landed as GTK's sidebar takes it ([`Landing`]), and the move follows
//! GTK's rule for each target, which is not the same as moving the row among those shown: the
//! saved order and the bookmarks file hold rows the sidebar does not show (hidden places, pins
//! of a user folder), and both files are shared with GTK Strata, and the second with every GTK
//! application.
//!
//! - **A place** dropped on a row goes before or after it; in the gap between two rows, after
//!   the row above; in the gap above the first, before it (GTK's place gaps).
//! - **A pin** dropped on a pin goes before or after it; in a gap, before the pin after the gap,
//!   else after the one before it (`pin_gap_index`).

use strata_components::sidebar::{ItemId, Landing};
use strata_core::model::Location;
use strata_fs::adapters::bookmarks;

use super::{places, relist};
use crate::{App, shell};

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

/// GTK's target for a place dropped at `landing` among the places shown: the row it goes
/// next to, and whether after it.
pub(super) fn place_target(landing: Landing) -> (usize, bool) {
    match landing {
        Landing::Row { index, after } => (index, after),
        Landing::Gap(0) => (0, false),
        Landing::Gap(index) => (index - 1, true),
    }
}

/// `pin_gap_index`: where a gap between `after` and `before`, as the sidebar showed them, is in
/// the pins as they are now. None when both of its neighbours were unpinned elsewhere, so a
/// drop cannot land somewhere unrelated.
pub(super) fn pin_gap_index(
    places: &[(Location, String)],
    (after, before): (Option<&Location>, Option<&Location>),
) -> Option<usize> {
    let position = |anchor: &Location| places.iter().position(|(location, _)| location == anchor);
    match (before.and_then(position), after.and_then(position)) {
        (Some(index), _) => Some(index),
        (None, Some(index)) => Some(index + 1),
        (None, None) if before.is_none() => Some(places.len()),
        (None, None) => None,
    }
}

/// `move_pinned_place`: the pin at `source` to the insertion point `index`, counted before it
/// moves. False when nothing moved.
pub(super) fn move_pinned_place(
    places: &mut Vec<(Location, String)>,
    source: usize,
    index: usize,
) -> bool {
    let out_of_range = source >= places.len() || index > places.len();
    let stays_put = index == source || index == source + 1;
    if out_of_range || stays_put {
        return false;
    }
    let place = places.remove(source);
    places.insert(index - usize::from(index > source), place);
    true
}

/// The pin at `source` dropped at `landing` among the pins `shown`, in the pins as they are now.
/// False when nothing moved.
pub(super) fn move_pin_in(
    places: &mut Vec<(Location, String)>,
    source: &Location,
    landing: Landing,
    shown: &[Location],
) -> bool {
    let position = |wanted: &Location| places.iter().position(|(pinned, _)| pinned == wanted);
    let Some(source) = position(source) else {
        return false;
    };
    match landing {
        Landing::Row { index, after } => {
            let Some(target) = shown.get(index).and_then(position) else {
                return false;
            };
            reorder_pinned_places(places, source, target, after)
        }
        Landing::Gap(index) => {
            let gap = (
                index.checked_sub(1).and_then(|i| shown.get(i)),
                shown.get(index),
            );
            pin_gap_index(places, gap).is_some_and(|at| move_pinned_place(places, source, at))
        }
    }
}

/// The built-in place at `from` in the block was dropped at `landing`: the saved order follows.
pub(super) fn move_place(app: &mut App, from: usize, landing: Landing) {
    let ids: Vec<String> = app
        .sidebar
        .listed
        .places
        .iter()
        .map(|place| place.id.clone())
        .collect();
    let (target, after) = place_target(landing);
    let (Some(source), Some(target)) = (ids.get(from), ids.get(target)) else {
        return;
    };
    let mut order = places::resolve_place_order(&shell::preferences(app).sidebar_order);
    if !reorder_places(&mut order, source, target, after) {
        return;
    }
    let order: Vec<String> = order.into_iter().map(str::to_owned).collect();
    shell::saved(app).change(move |settings| settings.sidebar_order = order);
    relist(app);
}

/// The pin at `from` among those listed was dropped at `landing`: the bookmarks file follows,
/// and the sidebar hears it from the store. The pins are matched by location in the file as it
/// is then, as GTK merges into the shared file and never into a window's copy.
pub(super) fn move_pin(app: &App, from: usize, landing: Landing) {
    let shown: Vec<Location> = app
        .sidebar
        .listed
        .rows
        .iter()
        .filter(|row| matches!(row.item, ItemId::Pin(_)))
        .map(|row| row.location.clone())
        .collect();
    let Some(source) = shown.get(from).cloned() else {
        return;
    };
    let edit = move |contents: &[u8]| {
        let mut places = bookmarks::parse_pinned_places(contents);
        move_pin_in(&mut places, &source, landing, &shown)
            .then(|| bookmarks::serialize_pinned_places(&places).into_bytes())
    };
    glib::MainContext::default().spawn_local(async move {
        if let Err(error) = bookmarks::update(edit).await {
            tracing::warn!(%error, "the pinned folders could not be saved");
        }
    });
}

#[cfg(test)]
mod tests;

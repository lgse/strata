// SPDX-License-Identifier: MIT

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

pub(super) fn place_target(landing: Landing) -> (usize, bool) {
    match landing {
        Landing::Row { index, after } => (index, after),
        Landing::Gap(0) => (0, false),
        Landing::Gap(index) => (index - 1, true),
    }
}

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

// SPDX-License-Identifier: MIT

//! Sidebar file drops, Trash drops, `PINNED` gap validation and insertion, spring-loaded folder
//! opening, and sidebar-row tab-opening drags (`src/ui/window/sidebar_file_drop.rs`,
//! `sidebar_trash.rs`, `pinned_drop.rs`, `sidebar_pins.rs`).

use std::{collections::HashSet, time::Duration};

use rune::{Mods, Tx};
use strata_components::{
    contract::dnd::{DropAction, DropOffer, Dropped},
    sidebar::{Gap, ItemId, Landing, PinVerdict},
};
use strata_core::{i18n::tr, model::Location};
use strata_fs::{
    adapters::{bookmarks, directory_validation::folder_names},
    services::io_error_message,
};

use super::{KEY, places::Environment, reorder};
use crate::{
    App, dnd,
    error_dialog::{self, Error},
    shell,
};

const VALIDATION_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) type ForeignPinCache = Option<(u64, Option<Vec<(Location, String)>>)>;

pub(super) fn drop_action(
    app: &App,
    item: &ItemId,
    offer: &DropOffer,
    mods: Mods,
) -> Option<DropAction> {
    match item {
        ItemId::Place(id) if id == "trash" => dnd::trash_action(app, offer),
        _ => {
            let row = app.sidebar.location_of(item)?;
            row.location.native_path()?;
            dnd::drop_action(app, &row.location, offer, mods)
        }
    }
}

pub(super) fn drop_into(app: &mut App, item: &ItemId, dropped: Dropped, tx: &mut Tx) {
    if matches!(item, ItemId::Place(id) if id == "trash") {
        dnd::commit_trash_drop(app, &dropped, tx);
        return;
    }
    let Some(destination) = app
        .sidebar
        .location_of(item)
        .map(|row| row.location.clone())
        .filter(|loc| loc.native_path().is_some())
    else {
        return;
    };
    dnd::commit_file_drop(app, destination, &dropped, tx);
}

pub(super) fn pin_hover(app: &mut App, serial: u64, uris: Option<Vec<String>>, local: bool) {
    if local {
        return;
    }
    if app
        .sidebar
        .state
        .pin_check
        .foreign
        .as_ref()
        .is_some_and(|(s, _)| *s == serial)
    {
        return;
    }
    let Some(uris) = uris else {
        app.sidebar.state.pin_check.foreign = Some((serial, PinVerdict::Checking));
        return;
    };
    let Some(locations) = valid_pin_locations(&uris) else {
        app.sidebar.foreign_pins = Some((serial, None));
        app.sidebar.state.pin_check.foreign = Some((serial, PinVerdict::Refused));
        return;
    };
    app.sidebar.state.pin_check.foreign = Some((serial, PinVerdict::Checking));
    let later = app.sidebar.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let checked = glib::future_with_timeout(VALIDATION_TIMEOUT, folder_names(locations))
            .await
            .ok()
            .flatten();
        later.then(move |app, tx| {
            let verdict = if checked.is_some() {
                PinVerdict::Folders
            } else {
                PinVerdict::Refused
            };
            app.sidebar.foreign_pins = Some((serial, checked));
            app.sidebar.state.pin_check.foreign = Some((serial, verdict));
            dnd::repick(app, tx);
            true
        });
    });
}

pub(super) fn pin_drop(app: &mut App, gap: Gap, dropped: Dropped, _tx: &mut Tx) {
    if shell::modal_open(app) || dnd::is_sidebar_drag(app) {
        return;
    }
    let shown = shown_pin_locations(app);
    let after = gap.index.checked_sub(1).and_then(|i| shown.get(i)).cloned();
    let before = shown.get(gap.index).cloned();
    match known_pin_folders(app, &dropped) {
        Some(Some(folders)) => save_pinned_folders(app, folders, (after, before)),
        Some(None) => {}
        None => validate_and_pin(app, &dropped, (after, before)),
    }
}

/// The folders a drop pins when they are known already: a local drag's, or a foreign drag's
/// its hover checked (`Some(None)` when they were refused). `None` when they are still to check.
fn known_pin_folders(app: &App, dropped: &Dropped) -> Option<Option<Vec<(Location, String)>>> {
    if dropped.offer.local {
        return Some(dnd::local_pin_folders(app).map(<[(Location, String)]>::to_vec));
    }
    let (serial, cached) = app.sidebar.foreign_pins.clone()?;
    (serial == dropped.offer.serial).then_some(cached)
}

/// Checks a foreign drop's folders, then pins them between `around`'s two pins.
fn validate_and_pin(app: &App, dropped: &Dropped, around: (Option<Location>, Option<Location>)) {
    let uris = dropped.data.uris();
    let Some(locations) = valid_pin_locations(&uris) else {
        return;
    };
    let later = app.sidebar.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let checked = glib::future_with_timeout(VALIDATION_TIMEOUT, folder_names(locations))
            .await
            .ok()
            .flatten();
        if let Some(folders) = checked {
            later.then(move |app, _| {
                save_pinned_folders(app, folders, around);
                false
            });
        }
    });
}

fn valid_pin_locations(uris: &[String]) -> Option<Vec<Location>> {
    let locations = dnd::locations_from_uris(uris)?;
    (locations.len() == uris.len() && locations.iter().all(dnd::can_pin_location))
        .then_some(locations)
}

fn shown_pin_locations(app: &App) -> Vec<Location> {
    app.sidebar
        .listed
        .rows
        .iter()
        .filter(|row| matches!(row.item, ItemId::Pin(_)))
        .map(|row| row.location.clone())
        .collect()
}

pub(super) fn pin_locations_at(
    places: &mut Vec<(Location, String)>,
    sources: &[(Location, String)],
    target_index: usize,
    env: &Environment,
) -> bool {
    let mut seen: HashSet<Location> = places.iter().map(|(loc, _)| loc.clone()).collect();
    let additions: Vec<(Location, String)> = sources
        .iter()
        .filter(|(loc, _)| !env.is_standard(loc) && seen.insert(loc.clone()))
        .cloned()
        .collect();
    if additions.is_empty() {
        return false;
    }
    let index = target_index.min(places.len());
    places.splice(index..index, additions);
    true
}

fn save_pinned_folders(
    app: &App,
    folders: Vec<(Location, String)>,
    (after, before): (Option<Location>, Option<Location>),
) {
    let later = app.sidebar.later.clone();
    let edit = move |contents: &[u8]| {
        let env = Environment::current();
        let mut places = bookmarks::parse_pinned_places(contents);
        let at = reorder::pin_gap_index(&places, (after.as_ref(), before.as_ref()))?;
        pin_locations_at(&mut places, &folders, at, &env)
            .then(|| bookmarks::serialize_pinned_places(&places).into_bytes())
    };
    glib::MainContext::default().spawn_local(async move {
        if let Err(error) = bookmarks::update(edit).await {
            let message = io_error_message(&error);
            later.then(move |app, _| {
                error_dialog::show(
                    app,
                    Error::new(tr("Unable to update pinned folders"), message),
                );
                true
            });
        }
    });
}

pub(super) fn row_drag_start(app: &mut App, item: &ItemId) {
    let Some(location) = app
        .sidebar
        .location_of(item)
        .map(|row| row.location.clone())
    else {
        return;
    };
    dnd::sidebar_drag_start(app, location);
}

pub(super) fn row_drag_move(app: &mut App, landing: Option<Landing>, at: [f32; 2], tx: &mut Tx) {
    dnd::sidebar_drag_move(app, landing.is_some(), at, tx);
}

pub(super) fn row_drag_end(app: &mut App, dropped_outside: Option<[f32; 2]>, tx: &mut Tx) {
    dnd::sidebar_drag_end(app, dropped_outside, tx);
}

/// Spring-loaded opening of a hovered sidebar place/pin row (`spring_navigation` in
/// `src/ui/window/sidebar_file_drop.rs`): navigates without selecting the first item and keeps
/// keyboard focus on the hovered sidebar row.
pub(crate) fn spring_open(app: &mut App, item: &ItemId, tx: &mut Tx) {
    let Some(row) = app.sidebar.location_of(item).cloned() else {
        return;
    };
    if row.location.native_path().is_none() {
        return;
    }
    app.sidebar.current = Some(row.item.clone());
    shell::publish(app);
    app.browser.navigate_location(row.location, false);
    shell::settle(app, tx);
    tx.focus(row.item.key(KEY), false);
}

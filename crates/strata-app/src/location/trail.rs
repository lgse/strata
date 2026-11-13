// SPDX-License-Identifier: MIT

//! The crumbs of the folder the bar names, as GTK lays them out (`set_location`,
//! `visible_crumbs`, `crumb_label`, `crumb_icon` and `ancestor_crumb_button` in
//! `src/ui/browser/location/breadcrumbs.rs`).

use std::path::Path;

use strata_components::location_bar::{Crumb, Trail};
use strata_core::model::Location;
use strata_kit::icons::IconName;

/// The trail for `location`, and the folder each crumb opens, in the trail's order. A
/// crumb's id is its index in that list.
pub(super) fn of(location: &Location, home: &Location) -> (Trail, Vec<Location>) {
    let folders = visible(location, home);
    let current = folders.len().saturating_sub(1);
    let crumbs = folders
        .iter()
        .enumerate()
        .map(|(index, folder)| crumb(index, folder, home, index < current))
        .collect();
    (Trail::new(crumbs, location.display_path()), folders)
}

/// The crumbs shown for `location`, starting at the home folder when it lies inside it.
fn visible(location: &Location, home: &Location) -> Vec<Location> {
    let mut folders = location.breadcrumbs();
    if let Some(at) = folders.iter().position(|folder| folder == home) {
        folders.drain(..at);
    }
    folders
}

/// Crumb `index`, with the lock before its label when it is an ancestor that is an
/// administrator root (`ancestor_crumb_button`; the current folder's label has none).
fn crumb(index: usize, folder: &Location, home: &Location, ancestor: bool) -> Crumb {
    let label = if folder == home {
        "~".to_owned()
    } else {
        folder.display_name()
    };
    // The root's crumb is followed by no separator: `/` reads as one.
    let joins = is_filesystem_root(folder);
    let root = folder.is_administrator() && folder.parent().is_none();
    let badge = (ancestor && root).then_some(IconName::Lock);
    Crumb::new(index.to_string(), label)
        .description(folder.display_path())
        .icon(icon(folder, home))
        .badge(badge)
        .joins_next(joins)
}

fn icon(folder: &Location, home: &Location) -> IconName {
    if folder == home {
        IconName::Home
    } else if is_filesystem_root(folder) {
        IconName::Drive
    } else if folder
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash://"))
    {
        IconName::Trash
    } else if folder.uri_value().is_some() {
        IconName::Network
    } else {
        IconName::Folder
    }
}

fn is_filesystem_root(location: &Location) -> bool {
    location.native_path() == Some(Path::new("/"))
}

#[cfg(test)]
mod tests;

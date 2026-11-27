// SPDX-License-Identifier: MIT

use std::path::Path;

use strata_components::context_menu::{Item, MenuModel, Section};
use strata_core::model::Location;

/// Entries listed; older ones stay reachable with Back (`HISTORY_MENU_MAX_ENTRIES`).
pub(super) const MOST: usize = 50;

pub(super) const HOLD: f64 = 0.5;

/// The action id of history entry `index`, counted from the most recent.
fn action(index: usize) -> String {
    format!("back-{index}")
}

/// The entry an action id names.
pub(super) fn index_of(action: &str) -> Option<usize> {
    action.strip_prefix("back-")?.parse().ok()
}

pub(super) fn model(heading: &str, back: &[Location], home: &Path) -> MenuModel {
    let items = back.iter().take(MOST).enumerate().map(|(index, location)| {
        let path = location
            .native_path()
            .map_or_else(|| location.display_path(), |path| abbreviate(path, home));
        let name = if location.native_path() == Some(home) {
            "~".to_owned()
        } else {
            location.display_name()
        };
        Item::new(action(index), name).description(path)
    });
    MenuModel::new()
        .header_line(heading.to_owned())
        .section(Section::new(items))
}

fn abbreviate(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests;

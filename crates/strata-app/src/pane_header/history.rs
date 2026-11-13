// SPDX-License-Identifier: MIT

//! The back history Back lists, most recent first, with a jump to a chosen entry in one step
//! (`attach_back_history` and `fill_back_history` in `src/ui/browser/pane_header.rs`): a
//! right-click, a held press or the Menu key on Back opens it under the button.

use std::path::Path;

use strata_components::context_menu::{Item, MenuModel, Section};
use strata_core::model::Location;

/// Entries listed; older ones stay reachable with Back (`HISTORY_MENU_MAX_ENTRIES`).
pub(super) const MOST: usize = 50;

/// How long a press on Back is held before it lists the history instead of going back:
/// GTK's long press, at `gtk-long-press-time`'s default.
pub(super) const HOLD: f64 = 0.5;

/// The action id of history entry `index`, counted from the most recent.
fn action(index: usize) -> String {
    format!("back-{index}")
}

/// The entry an action id names.
pub(super) fn index_of(action: &str) -> Option<usize> {
    action.strip_prefix("back-")?.parse().ok()
}

/// The menu for `back`, the browser's back history most recent first: a heading, then each
/// folder named by its name, `~` for the home folder, and described by its path, abbreviated
/// against home.
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

/// `abbreviate_home` in `src/ui/settings/general.rs`.
fn abbreviate(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests;

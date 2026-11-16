// SPDX-License-Identifier: MIT

//! Live click-activation modes per browser view (`PreferenceManager::click_activation` in
//! `src/ui/preferences/input_settings.rs` and `ClickActivation::default_for` in
//! `src/ui/browser_modes.rs`).

use strata_components::collection::pointer::{ClickCount, ClickModes};
use strata_desktop::settings::Preferences;

#[cfg(test)]
mod tests;

fn from_stored(stored: u8, default: ClickCount) -> ClickCount {
    match stored {
        1 => ClickCount::One,
        2 => ClickCount::Two,
        _ => default,
    }
}

/// Click activation for the Columns view: defaults to two clicks on files and one on folders.
pub(crate) fn columns(settings: &Preferences) -> ClickModes {
    ClickModes::new(
        from_stored(settings.columns_file_clicks, ClickCount::Two),
        from_stored(settings.columns_folder_clicks, ClickCount::One),
    )
}

/// Click activation for the Icons view: defaults to two clicks on files and two on folders.
pub(crate) fn icons(settings: &Preferences) -> ClickModes {
    ClickModes::new(
        from_stored(settings.icons_file_clicks, ClickCount::Two),
        from_stored(settings.icons_folder_clicks, ClickCount::Two),
    )
}

/// Click activation for the List view: defaults to two clicks on files and two on folders.
pub(crate) fn list(settings: &Preferences) -> ClickModes {
    ClickModes::new(
        from_stored(settings.list_file_clicks, ClickCount::Two),
        from_stored(settings.list_folder_clicks, ClickCount::Two),
    )
}

/// Whether a single click activates files under `modes`.
pub(crate) fn one_click_files(modes: ClickModes) -> bool {
    modes.files == ClickCount::One
}

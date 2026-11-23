// SPDX-License-Identifier: MIT

use strata_components::columns::width::STANDARD;
use strata_desktop::settings::Preferences;

use crate::{App, shell};

/// The width new columns open at, from the saved settings, in base points.
pub(super) fn saved(settings: &Preferences) -> f32 {
    settings
        .browser_column_width
        .map_or(STANDARD, |width| width as f32)
}

pub(super) fn chooser_saved(settings: &Preferences) -> f32 {
    settings
        .chooser_column_width
        .map_or(STANDARD, |width| width as f32)
}

pub(super) fn keep(app: &mut App, width: f32) {
    app.columns.state.set_default_width(width);
    let kept = width.round().max(STANDARD) as i32;
    let chooser = app.browser.is_chooser_mode();
    shell::saved(app).change(|settings| {
        if chooser {
            settings.chooser_column_width = Some(kept);
        } else {
            settings.browser_column_width = Some(kept);
        }
    });
}

pub(super) fn fit_area(app: &mut App, rect: [f32; 4]) {
    if shell::parts(app).drawer().is_some() {
        return;
    }
    let area = rect[2] - shell::sidebar_held_back(app);
    app.columns.state.set_width_cap(Some(area));
}

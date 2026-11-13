// SPDX-License-Identifier: MIT

//! The columns' width, as GTK's Columns view keeps it (`src/ui/browser/columns/resize.rs`):
//!
//! - **New columns open at the saved width,** `browser_column_width`, or one standard column
//!   when none is saved (`initial_column_width`), never narrower than a standard column.
//! - **A resize or an autofit saves its width** in base points, rounded, at least one standard
//!   column (`remember_column_width`), and new columns open at it. The component spreads a
//!   dragged width over the open columns and fits one column alone, as GTK does.
//! - **No column is wider than the browser's area:** the view's own width, which the
//!   component takes as its cap (`column_width_limit`). GTK takes the split's width less the
//!   sidebar at its full width; beside a sidebar drawn narrower than that, Rune's area is the
//!   wider by the difference.

use strata_components::columns::width::STANDARD;
use strata_desktop::settings::Preferences;

use crate::{App, shell};

/// The width new columns open at, from the saved settings, in base points.
pub(super) fn saved(settings: &Preferences) -> f32 {
    settings
        .browser_column_width
        .map_or(STANDARD, |width| width as f32)
}

/// A resize or an autofit ended at `width` base points: new columns open at it, and it is
/// saved.
pub(super) fn keep(app: &mut App, width: f32) {
    app.columns.state.set_default_width(width);
    let kept = width.round().max(STANDARD) as i32;
    shell::saved(app).change(|settings| settings.browser_column_width = Some(kept));
}

/// The view was laid out `rect` wide: its columns stay within it.
pub(super) fn fit_area(app: &mut App, rect: [f32; 4]) {
    app.columns.state.set_width_cap(Some(rect[2]));
}

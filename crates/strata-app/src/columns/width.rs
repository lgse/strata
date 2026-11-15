// SPDX-License-Identifier: MIT

//! The columns' width, as GTK's Columns view keeps it (`src/ui/browser/columns/resize.rs`):
//!
//! - **New columns open at the saved width,** `browser_column_width`, or one standard column
//!   when none is saved (`initial_column_width`), never narrower than a standard column.
//! - **A resize or an autofit saves its width** in base points, rounded, at least one standard
//!   column (`remember_column_width`), and new columns open at it. The component spreads a
//!   dragged width over the open columns and fits one column alone, as GTK does.
//! - **No column is wider than the browser's area:** the split's width (taking in the
//!   preview's slot, which yields to the columns, less its divider line) or, when no drawer
//!   is mounted, the view's own width, less what a railed sidebar holds back
//!   ([`shell::sidebar_held_back`]), which the component takes as its cap
//!   (`column_width_limit`). GTK counts a railed sidebar at its full width, "so columns still
//!   fit once it expands" (`publish_column_width_limit` in `src/ui/preview/layout.rs`).

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

/// The view was laid out `rect` wide: when no preview drawer wraps the view, its columns stay
/// within `rect`, less what a railed sidebar holds back. When a preview drawer wraps the view
/// ([`crate::slots::Place::Drawer`]), the drawer's split measures the whole browser area
/// (including the preview slot) and sets the width cap there instead.
pub(super) fn fit_area(app: &mut App, rect: [f32; 4]) {
    if shell::parts(app).drawer().is_some() {
        return;
    }
    let area = rect[2] - shell::sidebar_held_back(app);
    app.columns.state.set_width_cap(Some(area));
}

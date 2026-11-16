// SPDX-License-Identifier: MIT

//! The Icons view: Strata's `Browser` on one side, strata-components' `IconsGrid` on the
//! other, over the one folder a [`Pane`](crate::pane::Pane) shows. What it shares with the
//! List view is [`crate::single`]'s: the events in, the intents out, and one cursor and one
//! selection, the pane's. A folder opens in place, as GTK's Icons view opens it
//! (`activate_in_place`).

use strata_components::{context_menu::AnchorEdge, icons_grid::IconsGridState};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::settings::Preferences;

use crate::{
    fills::{Details, Fills},
    single::{Made, Single},
    slots::Setup,
    thumbnails::Viewer,
};

mod part;
pub(crate) mod size;
mod view;

pub(crate) use part::tick_active;

/// The grid's key: its collection's, and the element that takes the keyboard. Every element
/// key of the view starts with it.
pub(crate) const VIEW: &str = "icons";

pub(crate) fn origin_drag(
    app: &mut crate::App,
    d: &mut rune::Drag,
    clears: bool,
    tx: &mut rune::Tx,
) {
    let c = if app.filter.pane.shows() {
        view::results().collection(app)
    } else {
        view::grid().collection(app)
    };
    c.origin_drag(app, (d, clears), tx);
}

pub(crate) struct Icons {
    pub(crate) single: Single,
    grid: IconsGridState,
    /// The tile size in points, from 32 to 256 (`size.rs`).
    size: f32,
    /// The folder shown when the size was last set, so the part's poll can tell when another
    /// folder shows and set the size that folder follows.
    sized_for: Option<Location>,
    /// The folder views' revision the size was last read at: a change there is read again.
    sized_at: u64,
    /// The size the shown folder resolved to when last read: a change of the values applies
    /// only when this changes.
    resolved: f32,
    /// The grid's viewer in the window's thumbnails.
    thumbs: Viewer,
    /// The metadata fills of the rows near the viewport, with the icon details the tiles show.
    fills: Fills,
}

impl Icons {
    fn with_setup(setup: &Setup<'_>) -> Self {
        let settings: &Preferences = setup.settings;
        Self {
            single: Single::new(Made {
                key: VIEW,
                wrap_rows: true,
                left_leaves: false,
                described: tr("Icons view"),
                menu_at: AnchorEdge::Right,
                worn: setup.worn.clone(),
            }),
            grid: IconsGridState::new(),
            size: size::tile_size(settings.icons_thumbnail_size),
            sized_for: None,
            sized_at: 0,
            resolved: size::tile_size(settings.icons_thumbnail_size),
            thumbs: Viewer::new(setup.later.clone(), part::locate),
            fills: Fills::new(Details::All),
        }
    }
}

#[cfg(test)]
impl Icons {
    pub(crate) fn source(&self) -> &dyn strata_components::contract::EntrySource {
        self.single.pane.source()
    }
}

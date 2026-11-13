// SPDX-License-Identifier: MIT

//! The Icons view as a part of the window: the second of the view slot.

use rune::Tx;

use strata_components::{contract::EntryId, icons_grid::IconsGrid, thumbnails::ThumbnailStore};

use super::{Icons, VIEW, size, view};
#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App,
    commands::Action,
    rename::Rename,
    single::{self, Single, SinglePane},
    slots::{Listener, Part, Place, Setup, View},
    thumbnails::Viewer,
};

fn thumbnail_parts(app: &mut App) -> (&mut Viewer, &mut ThumbnailStore) {
    (&mut app.icons.thumbs, app.window.thumbnails_mut())
}

/// Where entry `id` of the folder shown is in the browser.
pub(super) fn locate(app: &App, id: EntryId) -> Option<(usize, usize)> {
    let pane = &app.icons.single.pane;
    let depth = pane.depth()?;
    Some((depth, pane.position(depth, id)?))
}

/// Asks for the fills the rows near the viewport lack, once a poll.
fn fill(app: &mut App) {
    let icons = &mut app.icons;
    let pane = &icons.single.pane;
    if let (Some(depth), Some(listing)) = (pane.depth(), pane.listing()) {
        icons.fills.request(&app.browser, depth, listing);
    }
}

/// Whether the saved `browser_mode` names this view: `icons`, or `grid`, which the GTK
/// application reads as Icons too (`src/ui/preferences/stored.rs`).
fn shows(mode: &str) -> bool {
    matches!(mode, "icons" | "grid")
}

impl SinglePane for Icons {
    const KEY: &'static str = VIEW;

    fn single(app: &App) -> &Single {
        &app.icons.single
    }

    fn single_mut(app: &mut App) -> &mut Single {
        &mut app.icons.single
    }

    fn renaming(app: &mut App) -> (&Single, &mut Rename) {
        (&app.icons.single, &mut app.rename)
    }

    /// C3's key: the tiles a row holds at the grid's last width, and their height.
    fn layout_key(app: &App) -> u64 {
        let width = app.icons.single.pane.collection().viewport().w;
        IconsGrid::<App>::layout_key(width, app.icons.size, &app.look)
    }
}

impl Part for Icons {
    const PLACE: Place<App> = Place::View(View {
        build: view::build,
        shows,
        focus: VIEW,
        supports_type_grouping: false,
        pointer_at: None,
        rows: single::rows::<Self>(),
        editing: single::editing::<Self>(),
        #[cfg(feature = "qa")]
        probe: Probe {
            // `view_name` in `src/ui/accessibility.rs`.
            name: "Icons",
            roles: ("grid", "grid-cell"),
            listed: single::listed::<Self>,
            lists: single::lists::<Self>,
        },
    });

    fn new(setup: &Setup<'_>) -> Self {
        Self::with_settings(setup.settings, setup.later.clone())
    }

    fn listen(setup: &Setup<'_>) -> Option<Listener<App>> {
        single::listen::<Self>(setup)
    }

    fn publish(app: &mut App) {
        single::publish::<Self>(app);
    }

    const COMMANDS: &'static [Action<App>] = size::COMMANDS;

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let said = single::poll::<Self>(app, clock, tx);
        size::follow_folder(app);
        let grid = view::grid();
        let ticked = grid.collection(app).tick(app, tx);
        grid.poll(app, tx);
        Viewer::pump(app, thumbnail_parts, clock);
        fill(app);
        ticked | said
    }
}

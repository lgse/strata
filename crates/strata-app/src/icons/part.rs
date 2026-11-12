// SPDX-License-Identifier: MIT

//! The Icons view as a part of the window: the second of the view slot.

use rune::Tx;

use std::path::{Path, PathBuf};

use strata_components::{contract::EntryId, thumbnails::ThumbnailStore};

use super::{Icons, VIEW, view};
#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App,
    rename::Rename,
    single::{self, Single, SinglePane},
    slots::{Listener, Part, Place, Setup, View},
    thumbnails::Viewer,
};

fn thumbnail_parts(app: &mut App) -> (&mut Viewer, &mut ThumbnailStore) {
    (&mut app.icons.thumbs, app.window.thumbnails_mut())
}

/// The file of entry `id` of the folder shown, through the browser.
fn entry_file(app: &App, id: EntryId) -> Option<PathBuf> {
    let pane = &app.icons.single.pane;
    let depth = pane.depth()?;
    let position = pane.position(depth, id)?;
    let entry = app.browser.entry_at(depth, position)?;
    entry.location.native_path().map(Path::to_path_buf)
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
}

impl Part for Icons {
    const PLACE: Place<App> = Place::View(View {
        build: view::build,
        shows,
        focus: VIEW,
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

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let said = single::poll::<Self>(app, clock, tx);
        let grid = view::grid();
        let ticked = grid.collection(app).tick(app, tx);
        grid.poll(app, tx);
        Viewer::pump(app, thumbnail_parts, entry_file, clock);
        ticked | said
    }
}

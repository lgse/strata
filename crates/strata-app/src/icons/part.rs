// SPDX-License-Identifier: MIT

//! The Icons view as a part of the window: the second of the view slot.

use std::time::Duration;

use rune::Tx;
use strata_fs::app::VisibleDepths;

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

/// Asks for the fills the rows near the viewport lack, once a poll after scrolling settles
/// (or immediately when a scrollbar thumb drag releases).
fn fill(app: &mut App, clock: f64, tx: &mut Tx) {
    let icons = &mut app.icons;
    let dragging = icons.grid.is_dragging();
    let pane = &icons.single.pane;
    if let (Some(depth), Some(listing)) = (pane.depth(), pane.listing())
        && let Some(left) = icons.fills.request_settled(
            &app.browser,
            depth,
            listing,
            crate::fills::ScrollPoll { clock, dragging },
        )
    {
        tx.wake_after(Duration::from_secs_f64(left));
    }
}

fn sync_worn(app: &mut App) {
    single::sync_worn::<Icons>(app);
    if let Some(id) = app.icons.thumbs.id() {
        let pane = &app.icons.single.pane;
        let version = pane.thumb_version();
        let source = pane.source();
        app.window.thumbnails_mut().resync(id, source, version);
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

    fn fills_mut(app: &mut App) -> &mut crate::fills::Fills {
        &mut app.icons.fills
    }

    fn renaming(app: &mut App) -> (&Single, &mut Rename) {
        (&app.icons.single, &mut app.rename)
    }

    fn clicks(
        settings: &strata_desktop::settings::Preferences,
    ) -> strata_components::collection::pointer::ClickModes {
        crate::clicks::icons(settings)
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
        rechecks: VisibleDepths::Active,
        apply: single::apply::<Self>,
        returns: None,
        pointer_at: None,
        end_gestures: |app| {
            app.icons.grid.reset();
            single::end_gestures::<Self>(app);
        },
        rows: single::rows::<Self>(),
        editing: single::editing::<Self>(),
        #[cfg(feature = "qa")]
        probe: Probe {
            // `view_name` in `src/ui/accessibility.rs`.
            name: "Icons",
            roles: ("grid", "grid-cell"),
            listed: single::listed::<Self>,
            lists: single::lists::<Self>,
            presented: single::presented::<Self>,
        },
    });

    fn new(setup: &Setup<'_>) -> Self {
        Self::with_setup(setup)
    }

    fn customized(app: &mut App) {
        sync_worn(app);
    }

    fn tab_shown(app: &mut App, _: &mut Tx) {
        sync_worn(app);
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
        // A filter's results tick in the grid's place, and the grid waits under them.
        let ticked = match app.filter.pane.shows() {
            true => {
                let results = view::results();
                let ticked = results.collection(app).tick(app, tx);
                let pruned = results.poll(app, tx);
                ticked | pruned
            }
            false => {
                let grid = view::grid();
                let ticked = grid.collection(app).tick(app, tx);
                let pruned = grid.poll(app, tx);
                ticked | pruned
            }
        };
        if !app.icons.grid.thumbs_deferred() {
            Viewer::pump(app, thumbnail_parts, clock);
        }
        fill(app, clock, tx);
        ticked | said
    }
}

pub(crate) fn tick_active(app: &mut App, tx: &mut Tx) -> bool {
    match app.filter.pane.shows() {
        true => view::results().collection(app).tick(app, tx),
        false => view::grid().collection(app).tick(app, tx),
    }
}

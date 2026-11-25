// SPDX-License-Identifier: MIT

//! The Icons view as a part of the window: the second of the view slot.

use std::time::Duration;

use rune::Tx;
use strata_core::util::DateFormat;
use strata_fs::app::VisibleDepths;

use strata_components::{
    contract::{EntryId, EntrySource},
    icons_grid::IconsGrid,
    thumbnails::ThumbnailStore,
};

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

    fn tab_shown(app: &mut App, tx: &mut Tx) {
        let format = DateFormat::parse(&crate::shell::preferences(app).date_format);
        app.icons.sync_date_format(format);
        sync_worn(app);
        single::restore_scroll::<Self>(app, tx);
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
        let resized = size::follow_folder(app);
        // A filter's results tick in the grid's place, and the grid waits under them.
        let ticked = match app.filter.pane.shows() {
            true => {
                let results = view::results();
                let ticked = app.filter.pane.collection().needs_tick()
                    && results.collection(app).tick(app, tx);
                let pruned = results.poll(app, tx);
                ticked | pruned
            }
            false => {
                let grid = view::grid();
                let ticked = app.icons.single.pane.collection().needs_tick()
                    && grid.collection(app).tick(app, tx);
                let pruned = grid.poll(app, tx);
                ticked | pruned
            }
        };
        if !app.icons.grid.thumbs_deferred() {
            Viewer::pump(app, thumbnail_parts, clock);
        }
        fill(app, clock, tx);
        ticked | said | resized | redate(app, clock, tx)
    }
}

/// Refreshes relative modified dates in Icons hover tooltips every 30 s while rows show.
fn redate(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let filter_rows =
        app.filter.pane.shows() && app.filter.pane.hits().is_some_and(|hits| !hits.is_empty());
    let icons = &mut app.icons;
    let rows = filter_rows
        || icons
            .single
            .pane
            .listing()
            .is_some_and(|listing| !listing.is_empty());
    let live = icons.date_format == DateFormat::Relative && rows;
    let tick = icons.dates.tick(live, clock);
    if let Some(wake) = tick.wake {
        tx.wake_after(wake);
    }
    tick.moved
}

pub(crate) fn tick_active(app: &mut App, tx: &mut Tx) -> bool {
    match app.filter.pane.shows() {
        true => {
            app.filter.pane.collection().needs_tick()
                && view::results().collection(app).tick(app, tx)
        }
        false => {
            app.icons.single.pane.collection().needs_tick()
                && view::grid().collection(app).tick(app, tx)
        }
    }
}

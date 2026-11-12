// SPDX-License-Identifier: MIT

//! The Icons view as a part of the window: the second of the view slot.

use rune::Tx;
use strata_components::{
    contract::EntrySource,
    rename::{By, Initial, Polled},
};

use super::{Icons, VIEW, intents, view};
#[cfg(feature = "qa")]
use crate::listing::Load;
#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App, events,
    pane::Applied,
    slots::{At, Editing, Listener, Part, Place, Rows, Setup, View},
};

/// Whether the saved `browser_mode` names this view: `icons`, or `grid`, which the GTK
/// application reads as Icons too (`src/ui/preferences/stored.rs`).
fn shows(mode: &str) -> bool {
    matches!(mode, "icons" | "grid")
}

impl Part for Icons {
    const PLACE: Place<App> = Place::View(View {
        build: view::build,
        shows,
        focus: VIEW,
        pointer_at: None,
        rows: Rows {
            position: |app, (depth, id)| app.icons.pane.position(depth, id),
            neighbour: |app, (depth, id), step| app.icons.pane.neighbour(depth, id, step),
            activate: intents::clicked_open,
            select_only: |app, (depth, id), tx| {
                app.icons.pane.select_only(depth, id);
                tx.focus(VIEW, false);
            },
            // Select All in a single pane is the browser's (`select_all` in
            // `src/ui/browser/view_state.rs`): the grid adopts it.
            select_all: |app, depth| app.browser.select_all(depth),
        },
        editing: Editing {
            begin: begin_rename,
            tick: tick_rename,
            hold: None,
            follow_now: None,
        },
        #[cfg(feature = "qa")]
        probe: Probe {
            // `view_name` in `src/ui/accessibility.rs`.
            name: "Icons",
            roles: ("grid", "grid-cell"),
            listed: |app| matches!(app.icons.pane.load(), Some(Load::Listed | Load::Failed(_))),
            lists: |app| match app.icons.pane.listing() {
                Some(_) => vec![VIEW.to_owned()],
                None => Vec::new(),
            },
        },
    });

    fn new(setup: &Setup<'_>) -> Self {
        Self::with_settings(setup.settings)
    }

    fn listen(setup: &Setup<'_>) -> Option<Listener<App>> {
        let ids = setup.ids.clone();
        Some(Box::new(move |event, browser, heard| {
            let read = events::read(event, browser, &ids);
            heard.copied(read.copied);
            for change in read.changes {
                heard.then(move |app: &mut App, tx: &mut Tx| {
                    if app.icons.pane.apply(change, tx) == Applied::Entered {
                        // The GTK window gives a new listing the keyboard.
                        tx.focus(VIEW, false);
                    }
                    true
                });
            }
        }))
    }

    fn publish(app: &mut App) {
        app.icons.pane.publish(&app.browser);
    }

    fn poll(app: &mut App, _clock: f64, tx: &mut Tx) -> bool {
        let grid = view::grid();
        let ticked = grid.collection(app).tick(app, tx);
        grid.poll(app, tx);
        ticked
    }
}

/// Opens the editor on the tile of entry `id` and shows it. `None` when the grid does not
/// show the entry: a hidden one has no tile to edit in.
fn begin_rename(app: &mut App, (depth, id): At, tx: &mut Tx) -> Option<Polled> {
    let pane = &app.icons.pane;
    let listing = pane.listing().filter(|_| pane.depth() == Some(depth))?;
    let index = listing.index_of(id)?;
    let entry = listing.entry(index);
    let polled = app
        .rename
        .state
        .begin(VIEW, id, entry.name, entry.kind, Initial::Stem);
    pane.collection().reveal(VIEW, listing, index, tx);
    Some(polled)
}

/// One poll of the editor against the grid's listing and viewport. An edit whose folder has
/// gone ends, and keeps the name.
fn tick_rename(app: &mut App) -> Polled {
    let pane = &app.icons.pane;
    match pane.listing() {
        Some(listing) => app.rename.state.tick(VIEW, listing, pane.collection()),
        None => app.rename.state.cancel(By::Listing),
    }
}

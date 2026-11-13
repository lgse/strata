// SPDX-License-Identifier: MIT

//! The List view as a part of the window: the third of the view slot.

use rune::Tx;
use strata_components::contract::EntrySource;

use super::{List, VIEW, view};
#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App,
    rename::Rename,
    single::{self, Single, SinglePane},
    slots::{Listener, Part, Place, Setup, View},
};

/// Asks for the fills the rows near the viewport lack, once a poll, as the Icons view does.
fn fill(app: &mut App) {
    let list = &mut app.list;
    let pane = &list.single.pane;
    if let (Some(depth), Some(listing)) = (pane.depth(), pane.listing()) {
        list.fills.request(&app.browser, depth, listing);
    }
}

/// Has the rows drawn read their relative dates again every 30 s, as GTK's labels do. True
/// when they do now.
fn redate(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let list = &mut app.list;
    let rows = list
        .single
        .pane
        .listing()
        .is_some_and(|listing| !listing.is_empty());
    let tick = list.dates.tick(list.relative_dates && rows, clock);
    if let Some(wake) = tick.wake {
        tx.wake_after(wake);
    }
    tick.moved
}

/// Whether the saved `browser_mode` names this view: `list`, or `explorer`, which the GTK
/// application reads as List too (`src/ui/preferences/stored.rs`).
fn shows(mode: &str) -> bool {
    matches!(mode, "list" | "explorer")
}

impl SinglePane for List {
    const KEY: &'static str = VIEW;

    fn single(app: &App) -> &Single {
        &app.list.single
    }

    fn single_mut(app: &mut App) -> &mut Single {
        &mut app.list.single
    }

    fn renaming(app: &mut App) -> (&Single, &mut Rename) {
        (&app.list.single, &mut app.rename)
    }
}

impl Part for List {
    const PLACE: Place<App> = Place::View(View {
        build: view::build,
        shows,
        focus: VIEW,
        // `BrowserMode::supports_type_grouping`: the List view groups by file type.
        supports_type_grouping: true,
        pointer_at: None,
        rows: single::rows::<Self>(),
        editing: single::editing::<Self>(),
        #[cfg(feature = "qa")]
        probe: Probe {
            // `view_name` in `src/ui/accessibility.rs`.
            name: "List",
            roles: ("grid", "row"),
            listed: single::listed::<Self>,
            lists: single::lists::<Self>,
        },
    });

    fn new(setup: &Setup<'_>) -> Self {
        Self::with_settings(setup.settings)
    }

    fn listen(setup: &Setup<'_>) -> Option<Listener<App>> {
        single::listen::<Self>(setup)
    }

    fn publish(app: &mut App) {
        single::publish::<Self>(app);
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let said = single::poll::<Self>(app, clock, tx);
        let ticked = view::ticked(app).tick(app, tx);
        fill(app);
        ticked | said | redate(app, clock, tx)
    }
}

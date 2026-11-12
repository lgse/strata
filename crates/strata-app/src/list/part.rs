// SPDX-License-Identifier: MIT

//! The List view as a part of the window: the third of the view slot.

use rune::Tx;

use super::{List, VIEW, view};
#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App,
    rename::Rename,
    single::{self, Single, SinglePane},
    slots::{Listener, Part, Place, Setup, View},
};

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

    fn poll(app: &mut App, _clock: f64, tx: &mut Tx) -> bool {
        view::ticked(app).tick(app, tx)
    }
}

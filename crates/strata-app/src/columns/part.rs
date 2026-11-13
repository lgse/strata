// SPDX-License-Identifier: MIT

//! The Columns view as a part of the window: the first of the view slot, and the worked
//! example of [`crate::slots`].

use rune::Tx;

#[cfg(feature = "qa")]
use super::Load;
use super::{Columns, Hands, VIEW, fills, intents, thumbs, view};
#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App, events,
    slots::{Editing, Listener, Part, Place, Rows, Setup, View},
};

/// The saved `browser_mode` that names this view.
const MODE: &str = "columns";

impl Part for Columns {
    const PLACE: Place<App> = Place::View(View {
        build: |app, frame| {
            app.columns.thumbs.set_scale(frame.cx.env().scale);
            Some(view(app).build(app))
        },
        shows: |mode| mode == MODE,
        focus: VIEW,
        supports_type_grouping: false,
        pointer_at: Some(|app, at| app.columns.pointer_at(at)),
        rows: Rows {
            position: |app, (depth, id)| app.columns.position(depth, id),
            neighbour: |app, (depth, id), step| app.columns.neighbour(depth, id, step),
            activate: intents::clicked_open,
            select_only: |app, at, tx| app.columns.select_only(at, &app.look, tx),
            select_all: |app, depth| app.columns.select_all(depth),
            reveal: |app, at, tx| app.columns.reveal_row(at, tx),
        },
        editing: Editing {
            begin: |app, at, tx| app.columns.begin_rename(&mut app.rename.state, at, tx),
            tick: |app| app.columns.tick_rename(&mut app.rename.state),
            hold: Some(|app, editing| app.columns.hold_mirror(editing)),
            follow_now: Some(|app| app.columns.mirror_now(&app.browser)),
        },
        #[cfg(feature = "qa")]
        probe: Probe {
            // `view_name` in `src/ui/accessibility.rs`.
            name: "Columns",
            roles: ("list-box", "list-box-option"),
            listed: |app| {
                let root = app.columns.load(0);
                matches!(root, Some(Load::Listed | Load::Failed(_)))
            },
            lists: |app| app.columns.list_keys(),
        },
    });

    fn new(setup: &Setup<'_>) -> Self {
        Self::with_settings(setup.settings, setup.ids.clone(), setup.later.clone())
    }

    fn listen(setup: &Setup<'_>) -> Option<Listener<App>> {
        let ids = setup.ids.clone();
        Some(Box::new(move |event, browser, heard| {
            let read = events::read(event, browser, &ids);
            heard.copied(read.copied);
            for change in read.changes {
                heard.then(move |app: &mut App, tx: &mut Tx| {
                    let hands = &mut Hands {
                        browser: &app.browser,
                        look: &app.look,
                        tx,
                    };
                    app.columns.apply(change, hands);
                    true
                });
            }
        }))
    }

    fn publish(app: &mut App) {
        app.columns.publish(&app.browser, None);
    }

    fn caught_up(app: &mut App) {
        app.columns.caught_up();
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let said = app.columns.watch_surface(clock, tx);
        let pruned = thumbs::poll(app, clock);
        fills::poll(app, clock, tx);
        view(app).tick(app, clock, tx) | said | pruned
    }
}

// SPDX-License-Identifier: MIT

//! The Columns view as a part of the window: the first of the view slot, and the worked
//! example of [`crate::slots`].

use rune::Tx;
use strata_fs::app::VisibleDepths;

#[cfg(feature = "qa")]
use super::Load;
use super::{Columns, Hands, VIEW, fills, intents, thumbs, view, width};
#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App,
    events::{self, Change},
    shell,
    slots::{Editing, Listener, Part, Place, Rows, Setup, View},
};

/// The saved `browser_mode` that names this view.
const MODE: &str = "columns";

/// Applies one change the listener read, or a rebuild's.
fn apply(app: &mut App, change: Change, tx: &mut Tx) -> bool {
    let hands = &mut Hands {
        browser: &app.browser,
        look: &app.look,
        tx,
    };
    app.columns.apply(change, hands);
    true
}

impl Part for Columns {
    const PLACE: Place<App> = Place::View(View {
        build: |app, frame| {
            let scale = frame.cx.env().scale;
            app.columns.rebuilding.set(false);
            app.columns.scale.set(scale);
            app.columns.thumbs.set_scale(scale);
            let built = view(app).build(app);
            let host = rune::col()
                .key("columns-host")
                .w_full()
                .h_full()
                .min_w(0.0)
                .min_h(0.0)
                .child(built);
            let host = crate::dnd::drop_target_by_point(
                app,
                host,
                "columns-host",
                intents::strip_zone(),
                intents::strip_action,
            );
            Some(host.on_layout(|app: &mut App, rect| width::fit_area(app, rect)))
        },
        shows: |mode| mode == MODE,
        focus: VIEW,
        supports_type_grouping: false,
        rechecks: VisibleDepths::All,
        apply,
        returns: Some(|app| app.columns.mirrors()),
        pointer_at: Some(|app, at| app.columns.pointer_at(at)),
        end_gestures: |app| app.columns.end_gestures(),
        rows: Rows {
            position: |app, (depth, id)| app.columns.position(depth, id),
            neighbour: |app, (depth, id), step| app.columns.neighbour(depth, id, step),
            activate: intents::clicked_open,
            select_only: |app, at, tx| {
                app.columns.select_only(at, &app.look, tx);
                if app.columns.shows_hits(at.0) {
                    crate::filter::picked(app);
                }
            },
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
            presented: |app| app.columns.presented(),
        },
    });

    fn new(setup: &Setup<'_>) -> Self {
        Self::with_setup(setup)
    }

    fn customized(app: &mut App) {
        let worn = shell::worn(app).clone();
        app.columns.set_worn(&worn);
    }

    fn listen(setup: &Setup<'_>) -> Option<Listener<App>> {
        let ids = setup.ids.clone();
        Some(Box::new(move |event, browser, heard| {
            if matches!(event, strata_fs::app::BrowserEvent::FolderCountsInvalidated) {
                heard.then(|app: &mut App, _: &mut Tx| {
                    app.columns.fills.invalidate();
                    true
                });
            }
            let read = events::read(event, browser, &ids);
            heard.copied(read.copied);
            for change in read.changes {
                heard.then(move |app: &mut App, tx: &mut Tx| apply(app, change, tx));
            }
        }))
    }

    fn publish(app: &mut App) {
        app.columns
            .refresh_mirror_preferences(shell::preferences(app));
        app.columns.publish(&app.browser, None);
    }

    fn caught_up(app: &mut App) {
        app.columns.caught_up();
    }

    /// What another tab changed while this one was hidden: the sizes shown, the width new
    /// columns open at, and saved item customizations.
    fn tab_shown(app: &mut App, _: &mut Tx) {
        let saved = shell::preferences(app);
        app.columns.refresh_mirror_preferences(saved);
        let (sizes, width) = (saved.columns_show_sizes, super::width::saved(saved));
        let worn = shell::worn(app).clone();
        app.columns.show_sizes(sizes);
        app.columns.state.set_default_width(width);
        app.columns.set_worn(&worn);
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let said = app.columns.watch_surface(clock, tx);
        let pruned = thumbs::poll(app, clock);
        fills::poll(app, clock, tx);
        view(app).tick(app, clock, tx) | said | pruned
    }
}

// SPDX-License-Identifier: MIT

//! The Columns view as a part of the window: the first of the view slot, and the worked
//! example of [`crate::slots`].

use rune::{El, Tx};

use super::{Columns, Hands, VIEW, view};
use crate::{
    App, events,
    slots::{Frame, Listener, Part, Setup, Slot},
};

/// The saved `browser_mode` that names this view.
const MODE: &str = "columns";

impl Part for Columns {
    const SLOT: Slot = Slot::View;
    const FOCUS: Option<&'static str> = Some(VIEW);

    fn new(setup: &Setup<'_>) -> Self {
        Self::with_settings(setup.settings, setup.ids.clone())
    }

    fn shows(mode: &str) -> bool {
        mode == MODE
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

    fn build(app: &App, _: &mut Frame<'_, App>) -> Option<El> {
        Some(view(app).build(app))
    }

    fn publish(app: &mut App) {
        app.columns.publish(&app.browser, None);
    }

    fn caught_up(app: &mut App) {
        app.columns.caught_up();
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        view(app).tick(app, clock, tx)
    }

    fn pointer_at(app: &mut App, at: (f32, f32)) {
        app.columns.pointer_at(at);
    }
}

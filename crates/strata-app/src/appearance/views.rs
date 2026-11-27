// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_desktop::keys::{
    Modifiers,
    bindings::{VIEW_COLUMNS, VIEW_ICONS, VIEW_LIST},
};

use crate::{
    App,
    commands::{Action, Keys, Outcome, Situation, Stage},
    rename, shell,
};

pub(super) const COLUMNS: &str = "columns";
pub(super) const ICONS: &str = "icons";
pub(super) const LIST: &str = "list";

/// Ctrl+1, Ctrl+2 and Ctrl+3 are read outside a text field, or in the filter's.
fn outside_text(_: &App, at: &Situation) -> bool {
    !at.text() || at.filter()
}

pub(crate) const SHOW_COLUMNS: Action<App> =
    Action::new("view-columns", |app, _, tx| switch(app, COLUMNS, tx))
        .command(VIEW_COLUMNS)
        .keys(&[Keys::at(
            Stage::WindowCommands,
            &[VIEW_COLUMNS.ignoring(Modifiers::empty())],
        )
        .when(outside_text)]);

pub(crate) const SHOW_ICONS: Action<App> =
    Action::new("view-icons", |app, _, tx| switch(app, ICONS, tx))
        .command(VIEW_ICONS)
        .keys(&[Keys::at(
            Stage::WindowCommands,
            &[VIEW_ICONS.ignoring(Modifiers::empty())],
        )
        .when(outside_text)]);

pub(crate) const SHOW_LIST: Action<App> =
    Action::new("view-list", |app, _, tx| switch(app, LIST, tx))
        .command(VIEW_LIST)
        .keys(&[Keys::at(
            Stage::WindowCommands,
            &[VIEW_LIST.ignoring(Modifiers::empty())],
        )
        .when(outside_text)]);

pub(crate) fn switch(app: &mut App, mode: &'static str, tx: &mut Tx) -> Outcome {
    let shown = shell::shown_view(app).is_some_and(|view| (view.shows)(mode));
    if !shown {
        if let Some(leaving) = shell::shown_view(app) {
            (leaving.end_gestures)(app);
        }
        rename::cancel(app, tx);
        shell::show_view(app, mode, tx);
    }
    shell::saved(app).change(|settings| settings.browser_mode = mode.to_owned());
    Outcome::Ran
}

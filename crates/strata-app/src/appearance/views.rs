// SPDX-License-Identifier: MIT

//! The view switch, as GTK's window switches views (`apply_browser_mode` in
//! `src/ui/window/key_routing.rs`): the view is shown at once, with an open rename ended
//! first (`cancel_rename` in `prepare_mode`), and then the choice is saved. The view shown
//! already is left as it is, as GTK's `set_view_mode` leaves it. `shell::show_view` is the
//! switch itself.
//!
//! The keys are read at the dispatcher's `window_commands` stage, with its guard: outside a
//! text field, or in the filter's (`src/ui/window/keyboard/commands.rs`).
//!
//! Left out, with what brings each back: the filter's query and focus carried across the
//! switch, and the listing search, are the filter's and the search's, A10's; a peek closed on
//! the way to Columns is the preview's.

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

/// The saved `browser_mode` of each view: GTK's serialized names (`BrowserMode`).
pub(super) const COLUMNS: &str = "columns";
pub(super) const ICONS: &str = "icons";
pub(super) const LIST: &str = "list";

fn outside_text(_: &App, at: &Situation) -> bool {
    !at.text() || at.filter()
}

pub(super) const COMMANDS: &[Action<App>] = &[
    Action::new("view-columns", |app, _, tx| switch(app, COLUMNS, tx))
        .command(VIEW_COLUMNS)
        .keys(&[Keys::at(
            Stage::WindowCommands,
            &[VIEW_COLUMNS.ignoring(Modifiers::empty())],
        )
        .when(outside_text)]),
    Action::new("view-icons", |app, _, tx| switch(app, ICONS, tx))
        .command(VIEW_ICONS)
        .keys(&[Keys::at(
            Stage::WindowCommands,
            &[VIEW_ICONS.ignoring(Modifiers::empty())],
        )
        .when(outside_text)]),
    Action::new("view-list", |app, _, tx| switch(app, LIST, tx))
        .command(VIEW_LIST)
        .keys(&[Keys::at(
            Stage::WindowCommands,
            &[VIEW_LIST.ignoring(Modifiers::empty())],
        )
        .when(outside_text)]),
];

/// Shows the view saved as `mode` and saves it. The key is taken whether or not the view
/// changes, as GTK's stage takes it.
pub(super) fn switch(app: &mut App, mode: &'static str, tx: &mut Tx) -> Outcome {
    let shown = shell::shown_view(app).is_some_and(|view| (view.shows)(mode));
    if !shown {
        rename::cancel(app, tx);
        shell::show_view(app, mode, tx);
    }
    shell::saved(app).change(|settings| settings.browser_mode = mode.to_owned());
    Outcome::Ran
}

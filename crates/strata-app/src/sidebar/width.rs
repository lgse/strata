// SPDX-License-Identifier: MIT

//! The width toggle: `toggle-sidebar`, which the header's button and the catalog's `SIDEBAR`
//! (Ctrl+B, Ctrl+N in 10xer mode) run, steps the sidebar full, rail, hidden and full again,
//! and saves each step (`bind_sidebar_modes` in `src/ui/window/composition/layout.rs`). The
//! header names the button as GTK's `SidebarToggle::describe` does
//! (`src/ui/sidebar_toggle.rs`): its name stays "Toggle sidebar (Ctrl+B)" at every step, and
//! its tooltip and description name the step the next press takes.

use rune::Tx;
use strata_components::sidebar::Step;
use strata_desktop::{
    keys::{Keys as Shown, Modifiers, bindings::SIDEBAR},
    settings::Preferences,
};

use super::saved_step;
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Stage},
    location::labels::with_keys,
    shell::{self, Face},
};

/// GTK's dispatcher reads `SIDEBAR` at its `sidebar_commands` stage, wherever the press lands
/// (`src/ui/window/keyboard/commands.rs`).
pub(super) const COMMANDS: &[Action<App>] = &[Action::new("toggle-sidebar", stepped)
    .command(SIDEBAR)
    .keys(&[Keys::at(
        Stage::SidebarCommands,
        &[SIDEBAR.ignoring(Modifiers::empty())],
    )])];

/// One press of the toggle, from what shows: GTK's `SidebarMode::next`.
fn stepped(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    if !app.sidebar.started {
        super::start(app);
    }
    let step = app.sidebar.state.width.advance();
    shell::saved(app).change(|settings| save(settings, step));
    Outcome::Ran
}

/// What GTK saves for each step: showing the sidebar asks for the full one, so a later rail
/// starts from it.
fn save(settings: &mut Preferences, step: Step) {
    match step {
        Step::Full => {
            settings.sidebar_compact = false;
            settings.sidebar_expanded = true;
        }
        Step::Rail => settings.sidebar_compact = true,
        Step::Hidden => settings.sidebar_expanded = false,
    }
}

/// What shows: before the sidebar's first poll, the step the settings saved.
fn shown(app: &App) -> Step {
    if app.sidebar.started {
        return app.sidebar.state.width.shown();
    }
    let saved = shell::preferences(app);
    saved_step(saved.sidebar_expanded, saved.sidebar_compact)
}

/// The header's toggle as GTK names it, with the keys as bound now. It is pressed while the
/// sidebar shows, as GTK's toggle button is active.
pub(crate) fn toggle_face(app: &App) -> Face {
    let keys = Shown::One(SIDEBAR.shown());
    let shown = shown(app);
    let (next, keyless) = match shown {
        Step::Full => ("Compact sidebar (%{keys})", "Compact sidebar"),
        Step::Rail => ("Hide sidebar (%{keys})", "Hide sidebar"),
        Step::Hidden => ("Show sidebar (%{keys})", "Show sidebar"),
    };
    Face {
        name: with_keys("Toggle sidebar (%{keys})", Some("Toggle sidebar"), keys),
        tooltip: Some(with_keys(next, Some(keyless), keys)),
        checked: Some(shown != Step::Hidden),
    }
}

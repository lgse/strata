// SPDX-License-Identifier: MIT

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

pub(super) const TOGGLE: Action<App> = Action::new("toggle-sidebar", stepped)
    .command(SIDEBAR)
    .keys(&[Keys::at(
        Stage::SidebarCommands,
        &[SIDEBAR.ignoring(Modifiers::empty())],
    )]);

fn stepped(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if app.chooser.is_confined() {
        return Outcome::Ran;
    }
    if !app.sidebar.started {
        super::start(app);
    }
    let step = app.sidebar.state.width.advance();
    app.sidebar.state.scroll_bar.release();
    if !app.browser.is_chooser_mode() {
        shell::saved(app).change(|settings| save(settings, step));
    }
    let in_sidebar = tx.focused().is_some_and(|key| key.starts_with(super::KEY));
    if step == Step::Hidden
        && in_sidebar
        && let Some(view) = shell::view_focus(app)
    {
        tx.focus(view, true);
    }
    Outcome::Ran
}

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

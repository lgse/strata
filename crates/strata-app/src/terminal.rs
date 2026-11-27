// SPDX-License-Identifier: MIT

mod launcher;
#[cfg(test)]
mod tests;

pub(crate) use launcher::{Emulator, no_terminal_message, spawn_reaped};

use rune::{El, Tx};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_desktop::keys::{
    Map, Modifiers,
    bindings::{ACTIONS_TERMINAL, TERMINAL},
};

use crate::{
    App,
    clipboard::paths,
    commands::{Action, Call, Keys, On, Outcome, Stage},
    error_dialog::{self, Error},
    menus, shell,
    slots::{Frame, Later, Part, Place, Setup},
};

pub(crate) struct Terminal {
    later: Later<App>,
}

fn nothing(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    None
}

impl Part for Terminal {
    const PLACE: Place<App> = Place::Overlays(nothing);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
        }
    }

    const COMMANDS: &'static [Action<App>] = &[
        Action::new("terminal", open_terminal)
            .command(TERMINAL)
            .keys(&[
                Keys::at(
                    Stage::FileCommands,
                    &[TERMINAL.ignoring(Modifiers::empty())],
                ),
                Keys::at(Stage::AfterYield, &[TERMINAL.ignoring(Modifiers::empty())])
                    .when(|app, _| reaches_window_actions(app)),
            ]),
        Action::new("actions-terminal", open_focused_action)
            .command(ACTIONS_TERMINAL)
            .keys(&[Keys::at(
                Stage::DirectCommands,
                &[ACTIONS_TERMINAL.in_map(Map::Default)],
            )
            .when(|app, at| !shell::preferences(app).tenxer_mode && !at.text() && at.in_view())]),
    ];
}

fn reaches_window_actions(app: &App) -> bool {
    !shell::preferences(app).tenxer_mode && !app.menu.state.is_open()
}

pub(crate) fn can_open(location: &Location) -> bool {
    location.native_path().is_some() && !paths::is_trash(location)
}

pub(crate) fn open_focused_folder(app: &mut App) -> bool {
    let Some(location) = focused_listing_depth(app)
        .and_then(|depth| app.browser.location_at(depth))
        .filter(can_open)
    else {
        return false;
    };
    launch_at(app, &location);
    true
}

fn open_focused_action(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    open_focused_folder(app);
    Outcome::Ran
}

fn open_terminal(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    match &call.on {
        On::Row((depth, id)) => {
            let Some((entry, _)) =
                menus::context_entries(app, *depth, *id).filter(|(entry, _)| entry.is_directory())
            else {
                return Outcome::Declined;
            };
            launch_at(app, &entry.location);
            Outcome::Ran
        }
        On::Column(depth) => {
            if let Some(location) = app.browser.location_at(*depth) {
                launch_at(app, &location);
            }
            Outcome::Ran
        }
        On::Place(location) => {
            let target = location.clone();
            launch_at(app, &target);
            Outcome::Ran
        }
        On::Focus => {
            if let Some(location) = focus_terminal_location(app) {
                launch_at(app, &location);
            }
            Outcome::Ran
        }
    }
}

fn shows_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

fn focused_listing_depth(app: &App) -> Option<usize> {
    if shows_columns(app) {
        Some(app.columns.active())
            .filter(|depth| app.browser.location_at(*depth).is_some())
            .or_else(|| app.browser.active_depth())
    } else {
        app.browser.active_depth()
    }
}

fn selected_terminal_location(entries: &[FileEntry]) -> Option<Location> {
    let [entry] = entries else {
        return None;
    };
    entry.is_directory().then(|| entry.location.clone())
}

fn focus_terminal_location(app: &App) -> Option<Location> {
    let selected = app.browser.selected_entries();
    selected_terminal_location(&selected).or_else(|| {
        if !shows_columns(app) {
            let depth = app.browser.active_depth()?;
            return app.browser.location_at(depth);
        }
        let depth = app
            .menu
            .state
            .is_open()
            .then(|| app.menu.depth())
            .filter(|depth| app.browser.location_at(*depth).is_some())
            .unwrap_or_else(|| app.columns.targeted());
        let directory = app.browser.location_at(depth);
        let opened = app.browser.location_at(depth + 1);
        let selected_in_destination: Vec<_> = app
            .browser
            .selected_positions(depth)
            .into_iter()
            .filter_map(|position| app.browser.entry_at(depth, position))
            .collect();
        match selected_terminal_location(&selected_in_destination) {
            Some(selected)
                if app.browser.active_depth() == Some(depth + 1)
                    && opened.as_ref() == Some(&selected) =>
            {
                Some(selected)
            }
            _ => directory,
        }
    })
}

pub(crate) fn launch_at(app: &mut App, location: &Location) {
    let Some(path) = location.native_path() else {
        failed(app, tr("This location is not a local folder"));
        return;
    };
    if paths::is_trash(location) {
        failed(app, tr("Terminal cannot be opened in Trash"));
        return;
    }
    let path = path.to_path_buf();
    tracing::debug!(
        location = %location.diagnostic_path(),
        "opening terminal"
    );
    let later = app.terminal.later.clone();
    spawn_reaped(
        later,
        move || {
            let Some(terminal) = Emulator::resolve() else {
                tracing::warn!("no terminal emulator found on PATH");
                return Err(no_terminal_message());
            };
            terminal.directory_command(&path).spawn().map_err(|error| {
                let program = terminal.program().to_string_lossy();
                tracing::warn!(%error, %program, "unable to launch terminal");
                terminal.launch_failure(&error)
            })
        },
        failed,
    );
}

fn failed(app: &mut App, detail: String) {
    error_dialog::show(app, Error::new(tr("Unable to open terminal"), detail));
}

// SPDX-License-Identifier: MIT

//! Default-map custom-action slots (`CUSTOM_ACTIONS` 1..10 in `src/ui/window/keyboard/direct.rs`
//! and `src/ui/window/keyboard/files.rs`) and numbered target helpers for 10xer chords
//! (`src/ui/browser/file_commands.rs`).

use std::{path::PathBuf, rc::Rc};

use rune::Tx;
use strata_core::{
    i18n::tr,
    model::{ActionInput, FileEntry, Location},
};
use strata_desktop::keys::{Map, bindings::*};
use strata_fs::services::{ActionCatalog, ActionHandle, InvocationSource, JobRequest};

use super::{
    dialog::run_action,
    menu::{folder_input, inputs_for_entries, native_paths},
};
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Situation, Stage},
    filter, shell,
};

pub(crate) struct NumberedActions {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "exposed for A11 10xer numbered action chord target guard"
        )
    )]
    pub(crate) targets: Vec<Location>,
    pub(crate) actions: Vec<Rc<ActionHandle>>,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) parent: Option<PathBuf>,
    pub(crate) source: InvocationSource,
}

macro_rules! slot_command {
    ($fn_name:ident, $index:expr) => {
        fn $fn_name(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
            run_listed_action(app, $index, tx);
            Outcome::Ran
        }
    };
}

slot_command!(slot_1, 0);
slot_command!(slot_2, 1);
slot_command!(slot_3, 2);
slot_command!(slot_4, 3);
slot_command!(slot_5, 4);
slot_command!(slot_6, 5);
slot_command!(slot_7, 6);
slot_command!(slot_8, 7);
slot_command!(slot_9, 8);
slot_command!(slot_10, 9);

pub(super) const COMMANDS: &[Action<App>] = &[
    Action::new("custom-action-1", slot_1)
        .command(CUSTOM_ACTION_1)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_1.in_map(Map::Default)],
        )
        .when(in_listing)]),
    Action::new("custom-action-2", slot_2)
        .command(CUSTOM_ACTION_2)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_2.in_map(Map::Default)],
        )
        .when(in_listing)]),
    Action::new("custom-action-3", slot_3)
        .command(CUSTOM_ACTION_3)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_3.in_map(Map::Default)],
        )
        .when(in_listing)]),
    Action::new("custom-action-4", slot_4)
        .command(CUSTOM_ACTION_4)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_4.in_map(Map::Default)],
        )
        .when(in_listing)]),
    Action::new("custom-action-5", slot_5)
        .command(CUSTOM_ACTION_5)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_5.in_map(Map::Default)],
        )
        .when(in_listing)]),
    Action::new("custom-action-6", slot_6)
        .command(CUSTOM_ACTION_6)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_6.in_map(Map::Default)],
        )
        .when(in_listing)]),
    Action::new("custom-action-7", slot_7)
        .command(CUSTOM_ACTION_7)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_7.in_map(Map::Default)],
        )
        .when(in_listing)]),
    Action::new("custom-action-8", slot_8)
        .command(CUSTOM_ACTION_8)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_8.in_map(Map::Default)],
        )
        .when(in_listing)]),
    Action::new("custom-action-9", slot_9)
        .command(CUSTOM_ACTION_9)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_9.in_map(Map::Default)],
        )
        .when(in_listing)]),
    Action::new("custom-action-10", slot_10)
        .command(CUSTOM_ACTION_10)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[CUSTOM_ACTION_10.in_map(Map::Default)],
        )
        .when(in_listing)]),
];

fn in_listing(app: &App, at: &Situation) -> bool {
    !shell::preferences(app).tenxer_mode && !at.text() && (at.in_view() || at.filter())
}

fn run_listed_action(app: &mut App, index: usize, tx: &mut Tx) {
    let catalog = super::shared_registry(app).catalog();
    if catalog.actions().get(index).is_none() {
        app.tenxer
            .flash
            .show(rust_i18n::t!("No action %{digit}", digit = index + 1), tx);
        return;
    }
    let numbered = actions_for_targets(app, |cat, inputs| {
        cat.listed(index, inputs).into_iter().collect()
    });
    match numbered.actions.first().cloned() {
        Some(action) => run_numbered(app, numbered, action),
        None => app.tenxer.flash.show(tr("No matching actions"), tx),
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "exposed for A11 10xer numbered action chords")
)]
pub(crate) fn numbered_actions(app: &mut App, source: InvocationSource) -> NumberedActions {
    match source {
        InvocationSource::Selection => actions_for_targets(app, ActionCatalog::numbered),
        InvocationSource::Background => actions_for_folder(app),
    }
}

pub(crate) fn run_numbered(app: &mut App, numbered: NumberedActions, action: Rc<ActionHandle>) {
    let Some(parent) = numbered.parent else {
        return;
    };
    run_action(
        app,
        JobRequest {
            action,
            inputs: numbered.paths,
            parent,
            source: numbered.source,
        },
    );
}

fn actions_for_folder(app: &mut App) -> NumberedActions {
    let folder = focused_folder(app);
    let parent = folder
        .as_ref()
        .and_then(|location| location.native_path().map(PathBuf::from));
    let actions = folder
        .as_ref()
        .and_then(folder_input)
        .map_or_else(Vec::new, |input| {
            super::shared_registry(app).catalog().numbered(&[input])
        });
    NumberedActions {
        targets: folder.into_iter().collect(),
        actions,
        paths: parent.iter().cloned().collect(),
        parent,
        source: InvocationSource::Background,
    }
}

fn actions_for_targets(
    app: &mut App,
    choose: impl FnOnce(&ActionCatalog, &[ActionInput]) -> Vec<Rc<ActionHandle>>,
) -> NumberedActions {
    let parent = focused_folder(app).and_then(|location| location.native_path().map(PathBuf::from));
    let entries = command_targets(app);
    let targets = entries.iter().map(|entry| entry.location.clone()).collect();
    let (inputs, paths) = match (inputs_for_entries(&entries), native_paths(&entries)) {
        (Some(inputs), Some(paths)) if parent.is_some() => (inputs, paths),
        _ => (Vec::new(), Vec::new()),
    };
    let actions = if inputs.is_empty() {
        Vec::new()
    } else {
        let catalog = super::shared_registry(app).catalog();
        choose(&catalog, &inputs)
    };
    NumberedActions {
        targets,
        actions,
        paths,
        parent,
        source: InvocationSource::Selection,
    }
}

fn focused_folder(app: &App) -> Option<Location> {
    if let Some((_, depth)) = filter::targets::selected_with_depth(app) {
        return app.browser.location_at(depth);
    }
    let shows_columns = shell::shown_view(app).is_some_and(|view| (view.shows)("columns"));
    if shows_columns {
        app.browser.location_at(app.columns.active_depth())
    } else {
        app.browser.active_location()
    }
}

fn command_targets(app: &mut App) -> Vec<FileEntry> {
    let tenxer = shell::preferences(app).tenxer_mode;
    if let Some(results) = filter::targets::selected(app) {
        if !results.is_empty() || !tenxer {
            return results;
        }
        return filter::targets::under_cursor(app)
            .map(|(_, entry)| entry)
            .into_iter()
            .collect();
    }
    shell::publish(app);
    if !tenxer {
        return app.browser.transfer_entries();
    }
    let shows_columns = shell::shown_view(app).is_some_and(|view| (view.shows)("columns"));
    let depth = if shows_columns {
        Some(app.columns.targeted())
    } else {
        app.browser.active_depth()
    };
    depth
        .map(|d| app.browser.command_entries(d))
        .unwrap_or_default()
}

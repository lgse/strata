// SPDX-License-Identifier: MIT

//! Context-menu custom-action rows (`MenuPlacement::Top` and the `"Actions"` submenu) and menu
//! activation (`src/ui/browser/context_menu/actions.rs`). As on GTK
//! (`rebuild_for_selection` / `rebuild_for_folder`), the rows are built once, from the catalog
//! as it stands when the menu opens; a reload the opening starts shows on the next menu, and a
//! row whose manifest changed meanwhile answers "Actions changed" when it runs.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use strata_components::{
    context_menu::{Item, Section},
    contract::EntryId,
};
use strata_core::{
    i18n::tr,
    model::{ActionInput, FOLDER_CONTENT_TYPE, FileEntry, InputKind, Location, MenuPlacement},
};
use strata_fs::services::{InvocationSource, JobRequest, MatchedAction, guess_content_type};
use strata_kit::icons::IconName;

use super::{dialog::run_action, icons::action_icon};
use crate::{App, menus};

const ACTION_PREFIX: &str = "custom-action:";
const SUBMENU_ACTION: &str = "custom-action:submenu";
const RUN_PREFIX: &str = "custom-action:run:";

#[derive(Clone)]
pub(super) struct MenuContext {
    inputs: Vec<ActionInput>,
    paths: Vec<PathBuf>,
    parent: PathBuf,
    source: InvocationSource,
    matched: Vec<MatchedAction>,
}

pub(crate) fn inputs_for_entries(entries: &[FileEntry]) -> Option<Vec<ActionInput>> {
    if entries.is_empty() {
        return None;
    }
    entries.iter().map(entry_input).collect()
}

fn entry_input(entry: &FileEntry) -> Option<ActionInput> {
    entry.location.native_path()?;
    let kind = if entry.is_directory() {
        InputKind::Folder
    } else {
        InputKind::File
    };
    Some(ActionInput {
        kind,
        name: entry.native_name.clone(),
        content_type: content_type_for(kind, &entry.native_name),
    })
}

pub(crate) fn folder_input(location: &Location) -> Option<ActionInput> {
    location.native_path()?;
    Some(ActionInput::folder(location.file_name()?))
}

fn content_type_for(kind: InputKind, name: &OsStr) -> Option<String> {
    if kind == InputKind::Folder {
        return Some(FOLDER_CONTENT_TYPE.to_owned());
    }
    // Menu matching must not read file contents (`src/ui/actions.rs`).
    let guessed_name = name.to_string_lossy();
    let (guessed, _) = guess_content_type(Path::new(guessed_name.as_ref()));
    (!guessed.is_empty()).then_some(guessed)
}

pub(crate) fn native_paths(entries: &[FileEntry]) -> Option<Vec<PathBuf>> {
    entries
        .iter()
        .map(|entry| entry.location.native_path().map(PathBuf::from))
        .collect()
}

/// Builds the custom-actions [`Section`] when a context menu opens and remembers its target
/// so a click on one of its rows acts on the captured inputs.
pub(crate) fn begin_menu(app: &mut App, depth: usize, entry: Option<EntryId>) -> Section {
    app.custom_actions.menu_context = None;
    let Some(mut context) = build_context(app, depth, entry) else {
        return Section::default();
    };
    let catalog = super::shared_registry(app).catalog();
    context.matched = catalog.matches(&context.inputs);
    let section = build_section(&context.matched);
    app.custom_actions.menu_context = Some(context);
    section
}

fn build_context(app: &App, depth: usize, entry: Option<EntryId>) -> Option<MenuContext> {
    match entry {
        Some(id) => selection_context(app, depth, id),
        None => background_context(app, depth),
    }
}

fn selection_context(app: &App, depth: usize, id: EntryId) -> Option<MenuContext> {
    let (_, entries) = menus::context_entries(app, depth, id)?;
    let parent = app
        .browser
        .location_at(depth)
        .and_then(|loc| loc.native_path().map(PathBuf::from))?;
    let inputs = inputs_for_entries(&entries)?;
    let paths = native_paths(&entries)?;
    Some(MenuContext {
        inputs,
        paths,
        parent,
        source: InvocationSource::Selection,
        matched: Vec::new(),
    })
}

fn background_context(app: &App, depth: usize) -> Option<MenuContext> {
    let location = app.browser.location_at(depth)?;
    let input = folder_input(&location)?;
    let path = location.native_path()?.to_path_buf();
    Some(MenuContext {
        inputs: vec![input],
        paths: vec![path.clone()],
        parent: path,
        source: InvocationSource::Background,
        matched: Vec::new(),
    })
}

fn build_section(matched: &[MatchedAction]) -> Section {
    let mut top = Vec::new();
    let mut submenu = Vec::new();
    for (index, entry) in matched.iter().enumerate() {
        let item = action_item(index, entry);
        if entry.placement == MenuPlacement::Top {
            top.push(item);
        } else {
            submenu.push(item);
        }
    }
    if !submenu.is_empty() {
        top.push(
            Item::submenu(SUBMENU_ACTION, tr("Actions"), [Section::new(submenu)])
                .icon(IconName::Play),
        );
    }
    Section::new(top)
}

fn action_item(index: usize, matched: &MatchedAction) -> Item {
    let handle = &matched.action;
    let mut item = Item::new(format!("{RUN_PREFIX}{index}"), handle.name().to_owned())
        .icon(action_icon(handle.definition.icon.as_deref()))
        .enabled(handle.is_available());
    if let Some(hint) = handle
        .unavailable_reason()
        .or(handle.definition.description.as_deref())
    {
        item = item.description(hint.to_owned());
    }
    item
}

pub(crate) fn is_menu_action(action: &str) -> bool {
    action.starts_with(ACTION_PREFIX)
}

pub(crate) fn activate_menu_action(app: &mut App, action: &str) {
    let Some(index) = action
        .strip_prefix(RUN_PREFIX)
        .and_then(|rest| rest.parse::<usize>().ok())
    else {
        return;
    };
    let Some(context) = app.custom_actions.menu_context.take() else {
        return;
    };
    let Some(matched) = context.matched.get(index) else {
        return;
    };
    run_action(
        app,
        JobRequest {
            action: matched.action.clone(),
            inputs: context.paths,
            parent: context.parent,
            source: context.source,
        },
    );
}

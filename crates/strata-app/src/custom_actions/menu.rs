// SPDX-License-Identifier: MIT

//! Context-menu custom-action rows (`MenuPlacement::Top` and the `"Actions"` submenu), live
//! catalog updates on an open menu, and menu activation (`src/ui/browser/context_menu/actions.rs`).

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use strata_components::{
    context_menu::{ContextMenuState, Item, MenuModel, Section},
    contract::EntryId,
};
use strata_core::{
    i18n::tr,
    model::{ActionInput, FOLDER_CONTENT_TYPE, FileEntry, InputKind, Location, MenuPlacement},
};
use strata_fs::services::{
    ActionCatalog, InvocationSource, JobRequest, MatchedAction, guess_content_type,
};
use strata_kit::icons::IconName;

use super::{dialog::run_action, icons::action_icon};
use crate::{App, menus};

const ACTION_PREFIX: &str = "custom-action:";
const SUBMENU_ACTION: &str = "custom-action:submenu";
const RUN_PREFIX: &str = "custom-action:run:";

#[derive(Clone)]
pub(super) struct MenuContext {
    depth: usize,
    entry: Option<EntryId>,
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
/// so catalog reloads and menu clicks act on the captured inputs.
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
        depth,
        entry: Some(id),
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
        depth,
        entry: None,
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

pub(super) fn refresh_open_menu(app: &mut App, catalog: &ActionCatalog) -> bool {
    if !app.menu.state.is_open() {
        return false;
    }
    let Some(context) = app.custom_actions.menu_context.as_mut() else {
        return false;
    };
    if context.depth != app.menu.depth()
        || context.entry != app.menu.state.target().and_then(|t| t.entry())
    {
        return false;
    }
    context.matched = catalog.matches(&context.inputs);
    let next = build_section(&context.matched);
    let custom_slot = app.menu.custom_slot();
    replace_slot(&mut app.menu.state, custom_slot, next)
}

fn replace_slot(state: &mut ContextMenuState, custom_slot: usize, next: Section) -> bool {
    let Some(mut model) = state.model().cloned() else {
        return false;
    };
    let Some(slot) = model.sections.get(custom_slot) else {
        return false;
    };
    if *slot == next {
        return false;
    }
    let open_actions = open_submenu_actions(&model, state.submenu());
    model.sections[custom_slot] = next;
    let reopened = resolve_submenu_actions(&model, &open_actions);
    state.update_model(model);
    state.close_submenu(0);
    if !reopened.is_empty() {
        state.open_submenu(&reopened);
    }
    true
}

fn open_submenu_actions(model: &MenuModel, path: &[usize]) -> Vec<String> {
    let mut actions = Vec::with_capacity(path.len());
    for depth in 1..=path.len() {
        let Some(item) = model.item_at(&path[..depth]) else {
            break;
        };
        actions.push(item.action.to_string());
    }
    actions
}

fn resolve_submenu_actions(model: &MenuModel, actions: &[String]) -> Vec<usize> {
    let mut path = Vec::with_capacity(actions.len());
    let mut sections: &[Section] = &model.sections;
    for wanted in actions {
        let mut flat = sections.iter().flat_map(|s| s.items.iter());
        let Some(pos) = flat.position(|item| &*item.action == wanted) else {
            break;
        };
        path.push(pos);
        let Some(sub) = model
            .item_at(&path)
            .and_then(|item| item.submenu.as_deref())
        else {
            break;
        };
        sections = sub;
    }
    path
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

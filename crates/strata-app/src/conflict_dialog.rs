// SPDX-License-Identifier: MIT

//! Destination and replay conflict prompt on C15's [`StrataDialog`], matching GTK's
//! `src/ui/browser/transfer/conflict_dialog.rs` and shared collision helpers from
//! `collisions.rs` and `paths.rs`.
//!
//! Both destination transfers ([`crate::transfer`]) and move undo/redo replays ([`crate::undo`])
//! open prompts here:
//! - **Footer order:** `[Cancel, Skip?, Keep Both?, Merge?, Replace?]`, with `Replace` as the
//!   danger primary button when offered and no default button so Return activates only the
//!   focused control (`activate_focused_button`).
//! - **Initial focus:** `Keep Both` when `focus == KeepBoth` or when `Replace` is absent, else
//!   `Replace`, else `Cancel` when neither is offered.
//! - **Dismissal (`Cancel`, `Escape`, `Close dialog`, backdrop):** abandons the operation
//!   without calling `on_choice` and returns keyboard focus to the listing.

use std::{ffi::OsStr, path::Path, rc::Rc};

use rune::El;
use rune_kit::Checkbox;
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::{i18n::tr, model::Location};
use strata_fs::{adapters::TransferTargetCollision, services::TransferConflict};
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::Holds,
    shell,
    slots::{Frame, Later, Part, Place, Scope, Setup},
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConflictChoice {
    Replace,
    Merge,
    Skip,
    KeepBoth,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ConflictFocus {
    #[default]
    Replace,
    KeepBoth,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ConflictActions {
    pub(crate) replace: bool,
    pub(crate) keep_both: bool,
    pub(crate) merge: bool,
    pub(crate) focus: ConflictFocus,
}

/// What one conflict prompt shows besides Cancel.
pub(crate) struct ConflictPrompt {
    pub(crate) name: String,
    pub(crate) explanation: String,
    pub(crate) apply_to_all_visible: bool,
    pub(crate) skip_visible: bool,
    pub(crate) actions: ConflictActions,
}

impl ConflictPrompt {
    /// Apply to All only matters with collisions left; Skip only when something else remains.
    pub(crate) fn visibility(remaining: usize, accepted: usize) -> (bool, bool) {
        (remaining > 0, accepted > 0 || remaining > 0)
    }
}

/// Splits the collisions an Apply to All answer covers from those that still need their own
/// prompt, such as a file meeting a folder after Replace.
pub(crate) fn covered_by_answer<T>(
    remaining: Vec<T>,
    conflict: TransferConflict,
    kinds: impl Fn(&T) -> TransferTargetCollision,
) -> (Vec<T>, Vec<T>) {
    remaining
        .into_iter()
        .partition(|collision| kinds(collision).allows(conflict))
}

pub(crate) fn unreplaceable_explanation(name: &str, folder: &str) -> String {
    rust_i18n::t!(
        "An item named “%{name}” already exists in %{folder}. A file and a folder can’t replace one another.",
        name = name,
        folder = folder
    )
    .into_owned()
}

pub(crate) fn shown_name(name: &OsStr) -> String {
    name.to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_control() {
                char::REPLACEMENT_CHARACTER
            } else {
                c
            }
        })
        .collect()
}

pub(crate) fn compact_display_path(location: &Location) -> String {
    location
        .native_path()
        .map(compact_native_path)
        .unwrap_or_else(|| location.display_path())
}

fn compact_native_path(path: &Path) -> String {
    let home = glib::home_dir();
    if path == home {
        return "~".to_owned();
    }
    path.strip_prefix(&home)
        .ok()
        .map(|suffix| format!("~/{}", suffix.to_string_lossy()))
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

type OnChoice = Rc<dyn Fn(&mut App, ConflictChoice, bool)>;

struct Shown {
    icon: IconName,
    prompt: ConflictPrompt,
    apply_to_all: bool,
    generation: u64,
    on_choice: Option<OnChoice>,
}

pub(crate) struct ConflictDialog {
    later: Later<App>,
    shown: Option<Shown>,
    open: bool,
    opened: u64,
}

impl Part for ConflictDialog {
    const PLACE: Place<App> = Place::Dialogs(build);
    const SCOPE: Scope = Scope::Tab;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            shown: None,
            open: false,
            opened: 0,
        }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.conflict_dialog
            .open
            .then_some(Holds::Modal { field: false })
    }
}

/// Opens a conflict prompt; cancelling abandons the operation without calling `on_choice`.
pub(crate) fn prompt(
    app: &mut App,
    prompt: ConflictPrompt,
    on_choice: impl Fn(&mut App, ConflictChoice, bool) + 'static,
) {
    prompt_with_icon(app, IconName::Copy, prompt, on_choice);
}

/// Opens a conflict prompt with `icon` in its header box (`ask_to_replace_archive` uses
/// [`IconName::Archive`]).
pub(crate) fn prompt_with_icon(
    app: &mut App,
    icon: IconName,
    prompt: ConflictPrompt,
    on_choice: impl Fn(&mut App, ConflictChoice, bool) + 'static,
) {
    let dialog = &mut app.conflict_dialog;
    dialog.opened += 1;
    let generation = dialog.opened;
    dialog.shown = Some(Shown {
        icon,
        prompt,
        apply_to_all: false,
        generation,
        on_choice: Some(Rc::new(on_choice)),
    });
    dialog.open = true;
}

#[cfg(test)]
pub(crate) fn is_open(app: &App) -> bool {
    app.conflict_dialog.open
}

fn toggle_apply_all(app: &mut App) {
    if let Some(shown) = app.conflict_dialog.shown.as_mut() {
        shown.apply_to_all = !shown.apply_to_all;
    }
}

fn dismiss(app: &mut App) {
    if !app.conflict_dialog.open {
        return;
    }
    app.conflict_dialog.open = false;
    if let Some(shown) = app.conflict_dialog.shown.as_mut() {
        shown.on_choice = None;
    }
    let later = app.conflict_dialog.later.clone();
    later.then(|app, _| {
        app.browser.focus_active();
        true
    });
}

fn choose(app: &mut App, choice: ConflictChoice) {
    if !app.conflict_dialog.open {
        return;
    }
    app.conflict_dialog.open = false;
    let Some(shown) = app.conflict_dialog.shown.as_mut() else {
        return;
    };
    let Some(on_choice) = shown.on_choice.take() else {
        return;
    };
    let apply_to_all = shown.apply_to_all;
    let later = app.conflict_dialog.later.clone();
    later.then(move |app, _| {
        on_choice(app, choice, apply_to_all);
        if !app.conflict_dialog.open {
            app.browser.focus_active();
        }
        true
    });
}

/// A prompt without Replace starts on Keep Both, or on Cancel when it offers neither.
fn initial_focus(actions: ConflictActions) -> Focus {
    let keep_both_first = actions.focus == ConflictFocus::KeepBoth || !actions.replace;
    if keep_both_first && actions.keep_both {
        Focus::Action("keep-both".to_owned())
    } else if actions.replace {
        Focus::Primary
    } else {
        Focus::Action("cancel".to_owned())
    }
}

fn add_actions(mut dialog: StrataDialog<App>, prompt: &ConflictPrompt) -> StrataDialog<App> {
    dialog = dialog.action("cancel", tr("Cancel"), dismiss);
    if prompt.skip_visible {
        dialog = dialog.action("skip", tr("Skip"), |app| choose(app, ConflictChoice::Skip));
    }
    if prompt.actions.keep_both {
        dialog = dialog.action("keep-both", tr("Keep Both"), |app| {
            choose(app, ConflictChoice::KeepBoth)
        });
    }
    if prompt.actions.merge {
        dialog = dialog.action("merge", tr("Merge"), |app| {
            choose(app, ConflictChoice::Merge)
        });
    }
    if prompt.actions.replace {
        dialog = dialog.primary(tr("Replace"), |app| choose(app, ConflictChoice::Replace));
    }
    dialog
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let state = &app.conflict_dialog;
    let shown = state.shown.as_ref()?;
    let prompt = &shown.prompt;
    let key = format!("conflict-{}", shown.generation);
    let mut dialog = StrataDialog::new(key.clone(), tr("File already exists"), dismiss)
        .open(state.open)
        .subtitle(prompt.name.clone())
        .icon(shown.icon)
        .tone(Tone::Danger);
    for paragraph in prompt.explanation.split("\n\n") {
        dialog = dialog.message(paragraph);
    }
    if prompt.apply_to_all_visible {
        let checkbox = Checkbox::new(shown.apply_to_all)
            .label(tr("Apply to All"))
            .on_toggle(toggle_apply_all)
            .build(&app.look.kit)
            .key(format!("{key}-apply-all"));
        dialog = dialog.child(checkbox);
    }
    dialog = add_actions(dialog, prompt)
        .initial_focus(initial_focus(prompt.actions))
        .default_button(false)
        .close_label(tr("Close dialog"))
        .width(MESSAGE_WIDTH);
    if let Some(view) = shell::view_focus(app) {
        dialog = dialog.return_focus(view);
    }
    dialog.build(&app.look, frame.layer);
    None
}

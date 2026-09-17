// SPDX-License-Identifier: MIT

//! Shared custom-action state for the UI layer.
//!
//! One registry serves the whole process, like [`crate::ui::theme::ThemeManager`],
//! so a menu, Settings, and every window see the same definitions and edits apply
//! without a restart. This module also owns the two translation steps the UI needs
//! and nothing else does: turning selected entries into matchable inputs, and
//! turning a matched action into a queued job.

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::{Rc, Weak},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::LocalActionStore,
    assets::icons,
    model::{ActionInput, FOLDER_CONTENT_TYPE, FileEntry, InputKind, Location},
    services::{ActionHandle, ActionRegistry, InvocationSource, JobRequest},
    ui::{
        controls::{ModalTone, message_dialog_description, message_dialog_layout},
        modal::{ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog},
    },
};

thread_local! {
    static SHARED_REGISTRY: RefCell<Weak<ActionRegistry>> = const { RefCell::new(Weak::new()) };
}

/// The one action registry for this process.
///
/// Definitions live in `$XDG_CONFIG_HOME/strata/actions`, which is user-owned and
/// hand-editable, so the registry reloads after every edit and treats load
/// problems as reportable state rather than fatal errors.
pub(crate) fn shared() -> Rc<ActionRegistry> {
    SHARED_REGISTRY.with(|shared| {
        if let Some(registry) = shared.borrow().upgrade() {
            return registry;
        }
        let registry = Rc::new(ActionRegistry::new(LocalActionStore::new()));
        shared.replace(Rc::downgrade(&registry));
        registry
    })
}

/// Bundled Lucide icons an action may choose, as `(manifest slug, asset name)`.
///
/// Only bundled icons are listed, so a theme change re-colors them through
/// `assets::primary_icon` instead of leaving a fallback-colored image behind.
pub(crate) const ACTION_ICON_CHOICES: &[(&str, &str)] = &[
    ("play", icons::PLAY),
    ("terminal", icons::TERMINAL),
    ("image", icons::PICTURES),
    ("file-code", icons::FILE_CODE),
    ("code-xml", icons::CODE_XML),
    ("file-archive", icons::FILE_ARCHIVE),
    ("file-text", icons::DOCUMENTS),
    ("copy", icons::COPY),
    ("scissors", icons::SCISSORS),
    ("pencil", icons::PENCIL),
    ("refresh", icons::REFRESH),
    ("download", icons::DOWNLOADS),
    ("scale", icons::SCALE),
    ("bug", icons::BUG),
    ("key", icons::KEY),
    ("lock", icons::LOCK),
    ("printer", icons::PRINTER),
    ("search", icons::SEARCH),
    ("folder", icons::FOLDER),
    ("check", icons::CHECK),
    ("triangle-alert", icons::TRIANGLE_ALERT),
    ("globe", icons::GLOBE),
    ("plus", icons::PLUS),
    ("monitor", icons::MONITOR),
];

pub(crate) const DEFAULT_ACTION_ICON: &str = icons::PLAY;

/// Resolves a manifest icon slug to a bundled asset, falling back to a default.
pub(crate) fn action_icon(icon: Option<&str>) -> &'static str {
    icon.and_then(|slug| {
        ACTION_ICON_CHOICES
            .iter()
            .find(|(candidate, _)| *candidate == slug)
            .map(|(_, asset)| *asset)
    })
    .unwrap_or(DEFAULT_ACTION_ICON)
}

pub(crate) fn is_known_action_icon(slug: &str) -> bool {
    ACTION_ICON_CHOICES
        .iter()
        .any(|(candidate, _)| *candidate == slug)
}

/// Matchable inputs for a selection, or `None` when no action can apply.
///
/// Custom actions are local-only in this release, so a selection containing a
/// remote location offers nothing rather than silently acting on a subset.
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

/// Matchable input for the folder a background context menu was opened on.
pub(crate) fn folder_input(location: &Location) -> Option<ActionInput> {
    location.native_path()?;
    Some(ActionInput::folder(location.file_name()?))
}

fn content_type_for(kind: InputKind, name: &std::ffi::OsStr) -> Option<String> {
    if kind == InputKind::Folder {
        return Some(FOLDER_CONTENT_TYPE.to_owned());
    }
    // The guess inspects the file name only; matching rules that need real
    // content stay out of scope, so opening a menu never reads file data.
    let guessed_name = name.to_string_lossy();
    let (guessed, _) = gio::content_type_guess(Some(guessed_name.as_ref()), None::<&[u8]>);
    (!guessed.is_empty()).then(|| guessed.to_string())
}

/// Paths for an invocation, or `None` when any input is not a native path.
pub(crate) fn native_paths(entries: &[FileEntry]) -> Option<Vec<PathBuf>> {
    entries
        .iter()
        .map(|entry| entry.location.native_path().map(PathBuf::from))
        .collect()
}

/// Queues an action for `paths`, confirming first when the action asks for it.
pub(crate) fn run_action(
    anchor: &impl IsA<gtk::Widget>,
    action: Rc<ActionHandle>,
    paths: Vec<PathBuf>,
    parent: PathBuf,
    source: InvocationSource,
) {
    let jobs = super::jobs::shared();
    if action.definition.run.confirm {
        confirm_and_run(anchor, action, paths, parent, source);
        return;
    }
    enqueue(anchor, &jobs, action, paths, parent, source);
}

fn enqueue(
    anchor: &impl IsA<gtk::Widget>,
    jobs: &Rc<crate::services::JobService>,
    action: Rc<ActionHandle>,
    paths: Vec<PathBuf>,
    parent: PathBuf,
    source: InvocationSource,
) {
    if let Err(error) = jobs.enqueue(JobRequest {
        action,
        inputs: paths,
        parent,
        source,
    }) {
        show_error_dialog(anchor, "Unable to run action", &error.to_string());
    }
}

/// Asks before running an action that declared `confirm = true`.
///
/// This is the author's request for a speed bump, not a security boundary: the
/// script still runs with the user's permissions once confirmed.
fn confirm_and_run(
    anchor: &impl IsA<gtk::Widget>,
    action: Rc<ActionHandle>,
    paths: Vec<PathBuf>,
    parent: PathBuf,
    source: InvocationSource,
) {
    let Some(ModalHost {
        overlay,
        blurred_root,
    }) = ModalHost::blurred_for(anchor)
    else {
        enqueue(
            anchor,
            &super::jobs::shared(),
            action,
            paths,
            parent,
            source,
        );
        return;
    };
    let jobs = super::jobs::shared();
    let layout = message_dialog_layout(
        action_icon(action.definition.icon.as_deref()),
        "Run this action?",
        action.name(),
        "Run",
        ModalTone::Danger,
    );
    let count = paths.len();
    layout.body.append(&message_dialog_description(&format!(
        "This runs “{}” on {}.",
        action.name(),
        item_count_label(count)
    )));
    if let Some(description) = action.definition.description.as_deref() {
        layout.body.append(&message_dialog_description(description));
    }
    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let run = layout.confirm;
    let layer = modal_layer(&content, &overlay, blurred_root.clone(), None);
    overlay.add_overlay(&layer);
    let weak_cancel = cancel.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(cancel) = weak_cancel.upgrade() {
            cancel.grab_focus();
        }
    });
    for button in [close, cancel] {
        let dismiss_layer = layer.clone();
        let dismiss_overlay = overlay.clone();
        let dismiss_root = blurred_root.clone();
        button.connect_clicked(move |_| {
            dismiss_modal_layer(&dismiss_layer, &dismiss_overlay, dismiss_root.as_ref());
        });
    }
    let run_layer = layer.clone();
    let run_overlay = overlay;
    let run_root = blurred_root;
    let error_anchor = anchor.as_ref().clone();
    run.connect_clicked(move |_| {
        dismiss_modal_layer(&run_layer, &run_overlay, run_root.as_ref());
        enqueue(
            &error_anchor,
            &jobs,
            action.clone(),
            paths.clone(),
            parent.clone(),
            source,
        );
    });
}

fn item_count_label(count: usize) -> String {
    match count {
        1 => "1 selected item".to_owned(),
        count => format!("{count} selected items"),
    }
}

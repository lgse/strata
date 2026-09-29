// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::{Rc, Weak},
};

use gtk::{gio, prelude::*};

use crate::{
    adapters::LocalActionStore,
    assets::icons,
    model::{ActionInput, FOLDER_CONTENT_TYPE, FileEntry, InputKind, Location},
    services::{ActionHandle, ActionRegistry, InvocationSource, JobRequest},
    ui::{
        controls::{ModalTone, focus_button, message_dialog_description, message_dialog_layout},
        modal::{
            ModalHost, dismiss_modal_layer, dismiss_modal_layer_then, modal_layer,
            show_error_dialog,
        },
    },
};

thread_local! {
    static SHARED_REGISTRY: RefCell<Weak<ActionRegistry>> = const { RefCell::new(Weak::new()) };
}

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

/// Manifest slugs paired with bundled Lucide assets.
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

fn content_type_for(kind: InputKind, name: &std::ffi::OsStr) -> Option<String> {
    if kind == InputKind::Folder {
        return Some(FOLDER_CONTENT_TYPE.to_owned());
    }
    // Menu matching must not read file contents.
    let guessed_name = name.to_string_lossy();
    let (guessed, _) = gio::content_type_guess(Some(guessed_name.as_ref()), None::<&[u8]>);
    (!guessed.is_empty()).then(|| guessed.to_string())
}

pub(crate) fn native_paths(entries: &[FileEntry]) -> Option<Vec<PathBuf>> {
    entries
        .iter()
        .map(|entry| entry.location.native_path().map(PathBuf::from))
        .collect()
}

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
    match jobs.enqueue(JobRequest {
        action,
        inputs: paths,
        parent,
        source,
    }) {
        Ok(id) => super::jobs::present_for(anchor, id),
        Err(error) => show_error_dialog(anchor, "Unable to run action", &error.to_string()),
    }
}

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
        show_error_dialog(
            anchor,
            "Unable to confirm action",
            "Open the action from a browser window.",
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
    focus_button(&run);
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
        let anchor = error_anchor.clone();
        let jobs = jobs.clone();
        let action = action.clone();
        let paths = paths.clone();
        let parent = parent.clone();
        dismiss_modal_layer_then(&run_layer, &run_overlay, run_root.as_ref(), move || {
            enqueue(&anchor, &jobs, action, paths, parent, source);
        });
    });
}

fn item_count_label(count: usize) -> String {
    match count {
        1 => "1 selected item".to_owned(),
        count => format!("{count} selected items"),
    }
}

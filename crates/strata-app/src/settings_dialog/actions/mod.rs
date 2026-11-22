// SPDX-License-Identifier: MIT

//! Settings -> Actions page state, live `ActionRegistry` observation, and async CRUD/import/export
//! coordination, ported from `src/ui/settings/actions.rs`.

mod editor;
mod editor_view;
mod examples;
mod folder_picker;
mod icon_chooser;
mod readiness;
#[cfg(test)]
mod tests;
mod view;

use std::{path::PathBuf, rc::Rc};

use rune::{El, Tx};
use strata_core::{
    i18n::tr,
    model::{ActionRuntime, ExecutionMode, MAX_ACTION_NAME_CHARS},
};
use strata_fs::services::{
    ActionAvailability, ActionCatalog, ActionHandle, ActionScript, ActionWriteRequest,
    ListenerGuard,
};

pub(super) use view::build;

use crate::{
    App, custom_actions,
    error_dialog::{self, Error},
    preview,
    slots::Later,
};
use editor::{EditorDraft, EditorMode, EditorSession, draft_definition, python_template};

#[cfg(test)]
type FolderPickerOverride = Rc<dyn Fn(&str) -> Result<Option<PathBuf>, String>>;

pub(super) struct DeleteConfirm {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) generation: u64,
}

pub(super) struct Notice {
    pub(super) title: String,
    pub(super) detail: String,
    pub(super) generation: u64,
}

pub(super) struct ActionsState {
    later: Later<App>,
    catalog: Option<Rc<ActionCatalog>>,
    observer: Option<ListenerGuard<Rc<dyn Fn()>>>,
    editor: Option<EditorSession>,
    delete_confirm: Option<DeleteConfirm>,
    notice: Option<Notice>,
    generation: u64,
    #[cfg(test)]
    folder_picker: Option<FolderPickerOverride>,
}

impl ActionsState {
    pub(super) fn new(later: Later<App>) -> Self {
        Self {
            later,
            catalog: None,
            observer: None,
            editor: None,
            delete_confirm: None,
            notice: None,
            generation: 0,
            #[cfg(test)]
            folder_picker: None,
        }
    }

    pub(super) fn is_observing(&self) -> bool {
        self.observer.is_some()
    }

    pub(super) fn popover_open(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(EditorSession::popover_open)
    }

    pub(super) fn has_focused_field(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(EditorSession::has_focused_field)
    }

    pub(super) fn catalog(&self, app: &App) -> Rc<ActionCatalog> {
        self.catalog
            .clone()
            .or_else(|| app.custom_actions.catalog())
            .unwrap_or_default()
    }

    /// Releases the `ActionRegistry` observer and closes any action modal when Settings closes.
    pub(super) fn closed(&mut self) {
        self.observer = None;
        self.editor = None;
        self.delete_confirm = None;
        self.notice = None;
    }

    pub(super) fn poll_editor_highlight(&mut self) -> bool {
        self.editor
            .as_mut()
            .is_some_and(EditorSession::poll_highlight)
    }

    fn next_generation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
}

pub(super) fn ensure_observing(app: &mut App) {
    if app.settings_dialog.actions.is_observing() {
        return;
    }
    let registry = custom_actions::shared_registry(app);
    let weak = app.settings_dialog.actions.later.downgrade();
    let observed_registry = Rc::downgrade(&registry);
    let guard = registry.observe(Rc::new(move || {
        let Some(registry) = observed_registry.upgrade() else {
            return;
        };
        let catalog = registry.catalog();
        if let Some(later) = weak.upgrade() {
            later.then(move |app, _| {
                app.settings_dialog.actions.catalog = Some(catalog);
                true
            });
        }
    }));
    app.settings_dialog.actions.catalog = Some(registry.catalog());
    app.settings_dialog.actions.observer = Some(guard);
}

pub(super) fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let Some(editor) = app.settings_dialog.actions.editor.as_mut() else {
        return false;
    };
    editor.poll(tx)
}

pub(super) fn build_overlays(app: &App, layer: &mut Vec<El>) {
    editor_view::build_dialog(app, layer);
    view::build_delete_dialog(app, layer);
    view::build_notice_dialog(app, layer);
}

pub(super) fn set_enabled(app: &mut App, id: String, enabled: bool) {
    let registry = custom_actions::shared_registry(app);
    let weak = app.settings_dialog.actions.later.downgrade();
    let saved = registry.set_enabled(id, enabled);
    glib::MainContext::default().spawn_local(async move {
        let outcome = saved.await;
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| match outcome {
            Ok(()) => {
                refresh_catalog(app);
                true
            }
            Err(error) => {
                error_dialog::show(
                    app,
                    Error::new(tr("Unable to save the action"), error.user_message()),
                );
                true
            }
        });
    });
}

pub(super) fn start_new(app: &mut App, tx: &mut Tx) {
    let action = ActionHandle {
        directory: PathBuf::new(),
        definition: draft_definition(ActionRuntime::Python, ExecutionMode::WholeSelection),
        availability: ActionAvailability::Unavailable {
            reason: tr("This action has not been saved yet"),
        },
    };
    let script = ActionScript {
        file_name: "main.py".to_owned(),
        contents: python_template(),
    };
    open_editor(
        app,
        EditorDraft {
            mode: EditorMode::Create,
            generation: 0,
            action: &action,
            script: Some(script),
        },
        tx,
    );
}

pub(super) fn start_edit(app: &mut App, id: String) {
    let registry = custom_actions::shared_registry(app);
    let existing = registry
        .catalog()
        .actions()
        .iter()
        .find(|action| action.id() == id)
        .cloned();
    let read = registry.read(id);
    let weak = app.settings_dialog.actions.later.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let outcome = read.await;
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, tx| {
            if !app.settings_dialog.is_open() {
                return false;
            }
            match outcome {
                Ok(stored) => {
                    let handle = match existing {
                        Some(prev) => ActionHandle {
                            definition: stored.definition,
                            ..(*prev).clone()
                        },
                        None => ActionHandle {
                            directory: PathBuf::new(),
                            definition: stored.definition,
                            availability: ActionAvailability::Unavailable {
                                reason: String::new(),
                            },
                        },
                    };
                    open_editor(
                        app,
                        EditorDraft {
                            mode: EditorMode::Edit,
                            generation: 0,
                            action: &handle,
                            script: stored.script,
                        },
                        tx,
                    );
                    true
                }
                Err(error) => {
                    error_dialog::show(
                        app,
                        Error::new(tr("Unable to read the action"), error.user_message()),
                    );
                    true
                }
            }
        });
    });
}

fn open_editor(app: &mut App, mut draft: EditorDraft<'_>, tx: &mut Tx) {
    let highlighter = preview::highlighter(app);
    draft.generation = app.settings_dialog.actions.next_generation();
    let weak = app.settings_dialog.actions.later.downgrade();
    let session = EditorSession::new(draft, &highlighter, weak);
    app.settings_dialog.actions.editor = Some(session);
    tx.focus_when_shown(editor::EDITOR_NAME_KEY, tx.focus_visible() || tx.by_keys());
}

pub(super) fn close_editor(app: &mut App) {
    app.settings_dialog.actions.editor = None;
}

pub(super) fn confirm_editor(app: &mut App, tx: &mut Tx) {
    let parsed = {
        let Some(editor) = app.settings_dialog.actions.editor.as_mut() else {
            return;
        };
        if editor.saving {
            return;
        }
        match editor.read() {
            Ok(result) => {
                editor.saving = true;
                editor.error = None;
                editor.error_tab = None;
                (editor.mode, editor.generation, result)
            }
            Err(err) => {
                editor.show_error(err.message, Some(err.tab));
                editor.tabs.show_error(err.tab, err.field_key, tx);
                return;
            }
        }
    };
    let (mode, generation, (definition, script)) = parsed;
    let request = ActionWriteRequest { definition, script };
    let registry = custom_actions::shared_registry(app);
    let weak = app.settings_dialog.actions.later.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let outcome = match mode {
            EditorMode::Create => registry.create(request).await,
            EditorMode::Edit => registry.write(request).await,
        };
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| {
            let Some(editor) = app.settings_dialog.actions.editor.as_mut() else {
                return false;
            };
            if editor.generation != generation {
                return false;
            }
            editor.saving = false;
            match outcome {
                Ok(()) => {
                    app.settings_dialog.actions.editor = None;
                    refresh_catalog(app);
                }
                Err(error) => {
                    editor.show_error(error.user_message(), None);
                }
            }
            true
        });
    });
}

pub(super) fn duplicate_action(app: &mut App, id: String, name: String) {
    let registry = custom_actions::shared_registry(app);
    let weak = app.settings_dialog.actions.later.downgrade();
    let duplicated = registry.duplicate(id, copy_name(&name));
    glib::MainContext::default().spawn_local(async move {
        let outcome = duplicated.await;
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| match outcome {
            Ok(_) => {
                refresh_catalog(app);
                true
            }
            Err(error) => {
                error_dialog::show(
                    app,
                    Error::new(tr("Unable to duplicate the action"), error.user_message()),
                );
                true
            }
        });
    });
}

pub(super) fn ask_delete(app: &mut App, id: String, name: String, _tx: &mut Tx) {
    let generation = app.settings_dialog.actions.next_generation();
    app.settings_dialog.actions.delete_confirm = Some(DeleteConfirm {
        id,
        name,
        generation,
    });
}

pub(super) fn dismiss_delete(app: &mut App) {
    app.settings_dialog.actions.delete_confirm = None;
}

pub(super) fn confirm_delete(app: &mut App) {
    let Some(confirm) = app.settings_dialog.actions.delete_confirm.take() else {
        return;
    };
    let registry = custom_actions::shared_registry(app);
    let weak = app.settings_dialog.actions.later.downgrade();
    let deleted = registry.delete(confirm.id);
    glib::MainContext::default().spawn_local(async move {
        let outcome = deleted.await;
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| match outcome {
            Ok(()) => {
                refresh_catalog(app);
                true
            }
            Err(error) => {
                error_dialog::show(
                    app,
                    Error::new(tr("Unable to delete the action"), error.user_message()),
                );
                true
            }
        });
    });
}

pub(super) fn start_import(app: &mut App, _tx: &mut Tx) {
    let title = tr("Import an action folder");
    let registry = custom_actions::shared_registry(app);
    let weak = app.settings_dialog.actions.later.downgrade();
    let override_result = test_pick_folder(app, &title);
    glib::MainContext::default().spawn_local(async move {
        let picked = match override_result {
            Some(result) => result,
            None => folder_picker::select_folder(title).await,
        };
        let path = match picked {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(message) => {
                if let Some(later) = weak.upgrade() {
                    later.then(move |app, _| {
                        error_dialog::show(
                            app,
                            Error::new(tr("Unable to import the action"), message),
                        );
                        true
                    });
                }
                return;
            }
        };
        let outcome = registry.import(path).await;
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| match outcome {
            Ok(id) => {
                refresh_catalog(app);
                tracing::info!(action = %id, "imported a custom action");
                if !app.settings_dialog.is_open() {
                    return false;
                }
                show_notice(
                    app,
                    tr("Action imported"),
                    rust_i18n::t!(
                        "“%{id}” was imported disabled. Review its script, then enable it.",
                        id = id
                    )
                    .into_owned(),
                );
                true
            }
            Err(error) => {
                error_dialog::show(
                    app,
                    Error::new(tr("Unable to import the action"), error.user_message()),
                );
                true
            }
        });
    });
}

pub(super) fn start_export(app: &mut App, id: String) {
    let title = tr("Export this action to a folder");
    let registry = custom_actions::shared_registry(app);
    let weak = app.settings_dialog.actions.later.downgrade();
    let override_result = test_pick_folder(app, &title);
    glib::MainContext::default().spawn_local(async move {
        let picked = match override_result {
            Some(result) => result,
            None => folder_picker::select_folder(title).await,
        };
        let folder = match picked {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(message) => {
                if let Some(later) = weak.upgrade() {
                    later.then(move |app, _| {
                        error_dialog::show(
                            app,
                            Error::new(tr("Unable to export the action"), message),
                        );
                        true
                    });
                }
                return;
            }
        };
        let outcome = registry.export(id, folder).await;
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| match outcome {
            Ok(target) => {
                if !app.settings_dialog.is_open() {
                    return false;
                }
                show_notice(app, tr("Action exported"), target.display().to_string());
                true
            }
            Err(error) => {
                error_dialog::show(
                    app,
                    Error::new(tr("Unable to export the action"), error.user_message()),
                );
                true
            }
        });
    });
}

fn show_notice(app: &mut App, title: String, detail: String) {
    let generation = app.settings_dialog.actions.next_generation();
    app.settings_dialog.actions.notice = Some(Notice {
        title,
        detail,
        generation,
    });
}

pub(super) fn dismiss_notice(app: &mut App) {
    app.settings_dialog.actions.notice = None;
}

fn refresh_catalog(app: &mut App) {
    let catalog = app.custom_actions.catalog();
    if let Some(catalog) = catalog {
        app.settings_dialog.actions.catalog = Some(catalog);
    }
}

fn test_pick_folder(_app: &App, _title: &str) -> Option<Result<Option<PathBuf>, String>> {
    #[cfg(test)]
    if let Some(hook) = &_app.settings_dialog.actions.folder_picker {
        return Some(hook(_title));
    }
    None
}

fn copy_name(name: &str) -> String {
    let candidate = rust_i18n::t!("%{name} copy", name = name);
    candidate.chars().take(MAX_ACTION_NAME_CHARS).collect()
}

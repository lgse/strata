// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::assets::{self, icons};
use crate::model::{ActionDefinition, ActionRuntime, ExecutionMode, InputKind};
#[cfg(test)]
use crate::model::{ErrorPolicy, MenuPlacement, WorkingDirectory};
use crate::services::{
    ActionHandle, ActionLoadFailure, ActionRegistry, ActionScript, ActionWriteRequest,
};
use crate::ui::{
    actions::action_icon,
    controls::{
        ModalLayout, ModalTone, message_dialog_description, message_dialog_layout, modal_layout,
    },
    modal::{ModalHost, close_on_escape, dismiss_modal_layer, modal_layer, show_error_dialog},
};

use super::{append_heading, page_content, scrollable_page, search};
use crate::model::action::examples::{ACTION_EXAMPLES, ActionExample};

mod editor;
mod examples;
mod icon_chooser;
mod readiness;

#[cfg(test)]
use editor::text_contents;
use editor::{EditorForm, EditorMode, draft_definition, label_control, python_template};

#[cfg(test)]
mod tests;

const MAX_EXTENSION_FIELD_CHARS: usize = 256;
const MAX_ARGUMENTS_FIELD_CHARS: usize =
    crate::model::action::MAX_ARGUMENTS * (crate::model::action::MAX_ARGUMENT_CHARS + 1);
const GENERAL_TAB: u32 = 0;
const SCRIPT_TAB: u32 = 1;
const BEHAVIOR_TAB: u32 = 2;

pub(super) fn actions_page() -> gtk::Widget {
    let content = page_content();
    content.add_css_class("settings-actions-page");
    append_heading(&content, "CUSTOM ACTIONS");
    let description = gtk::Label::new(Some(&crate::i18n::tr(
        "Add your own scripts to the file and folder context menus. \
         Actions run with your permissions, so only enable scripts you trust.",
    )));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-section-description");
    content.append(&description);

    let toolbar = super::wrap::WrapRow::new(8);
    toolbar.set_end_align(true);
    toolbar.set_hexpand(true);
    toolbar.set_margin_top(8);
    toolbar.set_margin_bottom(8);
    let new_action = gtk::Button::with_label(&crate::i18n::tr("New action…"));
    new_action.add_css_class("settings-action-button");
    new_action.add_css_class("settings-actions-create-button");
    search::tag(&new_action, "New custom action");
    let import = gtk::Button::with_label(&crate::i18n::tr("Import…"));
    import.add_css_class("settings-action-button");
    import.add_css_class("settings-actions-create-button");
    search::tag(&import, "Import custom action");
    toolbar.append(&new_action);
    toolbar.append(&import);
    content.append(&toolbar);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("settings-group");
    list.set_overflow(gtk::Overflow::Hidden);
    search::tag(&list, "Custom actions");
    content.append(&list);
    let problems = gtk::Box::new(gtk::Orientation::Vertical, 0);
    search::tag(&problems, "Custom actions");
    content.append(&problems);

    let registry = crate::ui::actions::shared();
    let state = Rc::new(PageState {
        registry: registry.clone(),
        list: list.clone(),
        problems: problems.clone(),
    });

    let new_state = state.clone();
    new_action.connect_clicked(move |button| PageState::start_new(&new_state, button));
    let import_state = state.clone();
    import.connect_clicked(move |button| PageState::import(&import_state, button));

    PageState::render(&state);
    let observer = Rc::new(RefCell::new(Some(registry.observe({
        let state = state.clone();
        Rc::new(move || PageState::render(&state))
    }))));
    list.connect_destroy(move |_| {
        observer.borrow_mut().take();
    });

    scrollable_page(&content, Some("settings-actions-scroll"))
}

struct PageState {
    registry: Rc<ActionRegistry>,
    list: gtk::Box,
    problems: gtk::Box,
}

impl PageState {
    fn render(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        while let Some(child) = self.problems.first_child() {
            self.problems.remove(&child);
        }
        let catalog = self.registry.catalog();
        if catalog.actions().is_empty() && catalog.failures().is_empty() {
            let empty = gtk::Label::new(Some(&crate::i18n::tr(
                "No custom actions yet. Create one to add it to the context menu.",
            )));
            empty.set_xalign(0.0);
            empty.set_wrap(true);
            empty.add_css_class("settings-option-description");
            empty.add_css_class("settings-actions-empty");
            self.list.append(&empty);
        }
        for action in catalog.actions() {
            self.list.append(&self.row(action));
        }
        if !catalog.failures().is_empty() {
            append_heading(&self.problems, "PROBLEMS");
            let note = gtk::Label::new(Some(&crate::i18n::tr(
                "These files could not be loaded. Fix them in the actions folder, or remove them.",
            )));
            note.set_xalign(0.0);
            note.set_wrap(true);
            note.add_css_class("settings-option-description");
            self.problems.append(&note);
            let group = super::settings_group(&self.problems, "");
            for failure in catalog.failures() {
                group.append(&problem_row(failure));
            }
        }
    }

    fn row(self: &Rc<Self>, action: &Rc<ActionHandle>) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("settings-option");
        row.add_css_class("settings-action-row");
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        heading.set_hexpand(true);
        heading.set_valign(gtk::Align::Center);
        let icon = crate::assets::text_icon(action_icon(action.definition.icon.as_deref()), 20);
        icon.set_valign(gtk::Align::Start);
        heading.append(&icon);
        heading.append(&action_copy(action));
        heading.append(&self.enabled_switch(action, &row));
        row.append(&heading);
        let buttons = super::wrap::WrapRow::new(6);
        buttons.set_end_align(true);
        buttons.set_valign(gtk::Align::Center);
        for (label, icon, handler) in [
            ("Edit", icons::PENCIL, ActionRowAction::Edit),
            ("Duplicate", icons::COPY, ActionRowAction::Duplicate),
            ("Export…", icons::DOWNLOADS, ActionRowAction::Export),
            ("Delete", icons::TRASH, ActionRowAction::Delete),
        ] {
            buttons.append(&self.row_button(action, (label, icon, handler)));
        }
        row.append(&buttons);
        row
    }

    fn enabled_switch(self: &Rc<Self>, action: &ActionHandle, anchor: &gtk::Box) -> gtk::Switch {
        let toggle = gtk::Switch::builder()
            .active(action.definition.enabled)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Center)
            .build();
        crate::ui::accessibility::set_own_label(&toggle, &crate::i18n::tr("Enabled"));
        let state = self.clone();
        let action_id = action.id().to_owned();
        let anchor = anchor.clone();
        toggle.connect_state_set(move |_, enabled| {
            state.set_enabled(&action_id, enabled, &anchor);
            glib::Propagation::Proceed
        });
        toggle
    }

    fn row_button(
        self: &Rc<Self>,
        action: &Rc<ActionHandle>,
        (label, icon, handler): (&str, &str, ActionRowAction),
    ) -> gtk::Button {
        let image = if handler == ActionRowAction::Delete {
            assets::danger_icon(icon, assets::CHROME_ICON_PX)
        } else {
            assets::text_icon(icon, assets::CHROME_ICON_PX)
        };
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        let label = crate::i18n::tr(label);
        let button = gtk::Button::builder()
            .child(&image)
            .tooltip_text(&label)
            .build();
        crate::ui::accessibility::set_label(&button, &label);
        button.add_css_class("settings-action-button");
        button.add_css_class("settings-action-icon-button");
        if handler == ActionRowAction::Delete {
            button.add_css_class("danger");
        }
        let state = self.clone();
        let action = action.clone();
        button.connect_clicked(move |button| match handler {
            ActionRowAction::Edit => state.edit(button, action.clone()),
            ActionRowAction::Duplicate => state.duplicate(button, &action),
            ActionRowAction::Export => state.export(button, &action),
            ActionRowAction::Delete => state.delete(button, &action),
        });
        button
    }
    fn set_enabled(self: &Rc<Self>, id: &str, enabled: bool, anchor: &gtk::Box) {
        let failed = {
            let anchor = anchor.clone();
            move |message: &str| {
                show_error_dialog(
                    &anchor,
                    &crate::i18n::tr("Unable to save the action"),
                    message,
                );
            }
        };
        let saved = self.registry.set_enabled(id.to_owned(), enabled);
        let state = self.clone();
        glib::MainContext::default().spawn_local(async move {
            match saved.await {
                Ok(()) => PageState::render(&state),
                Err(error) => failed(&error.user_message()),
            }
        });
    }

    fn save(
        self: &Rc<Self>,
        definition: ActionDefinition,
        script: Option<ActionScript>,
        done: impl FnOnce(Result<(), String>) + 'static,
    ) {
        let saved = self
            .registry
            .write(ActionWriteRequest { definition, script });
        let state = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let saved = saved.await.map_err(|error| error.user_message());
            if saved.is_ok() {
                PageState::render(&state);
            }
            done(saved);
        });
    }

    fn confirm_editor(
        self: &Rc<Self>,
        mode: EditorMode,
        definition: ActionDefinition,
        script: Option<ActionScript>,
        done: impl FnOnce(Result<(), String>) + 'static,
    ) {
        match mode {
            EditorMode::Create => {
                let created = self
                    .registry
                    .create(ActionWriteRequest { definition, script });
                glib::MainContext::default().spawn_local(async move {
                    done(created.await.map_err(|error| error.user_message()));
                });
            }
            EditorMode::Edit => self.save(definition, script, done),
        }
    }

    fn edit(self: &Rc<Self>, button: &gtk::Button, action: Rc<ActionHandle>) {
        let read = self.registry.read(action.id().to_owned());
        let state = self.clone();
        let button = button.clone();
        glib::MainContext::default().spawn_local(async move {
            match read.await {
                Ok(stored) => {
                    let action = Rc::new(ActionHandle {
                        definition: stored.definition,
                        ..(*action).clone()
                    });
                    state.open_editor(
                        &button,
                        EditorMode::Edit,
                        EditorDraft {
                            action,
                            script: stored.script,
                        },
                    );
                }
                Err(error) => show_error_dialog(
                    &button,
                    &crate::i18n::tr("Unable to read the action"),
                    &error.user_message(),
                ),
            }
        });
    }

    fn start_new(self: &Rc<Self>, button: &gtk::Button) {
        let action = Rc::new(ActionHandle {
            directory: std::path::PathBuf::new(),
            definition: draft_definition(ActionRuntime::Python, ExecutionMode::WholeSelection),
            availability: crate::services::ActionAvailability::Unavailable {
                reason: crate::i18n::tr("This action has not been saved yet"),
            },
        });
        let script = ActionScript {
            file_name: "main.py".to_owned(),
            contents: python_template(),
        };
        self.open_editor(
            button,
            EditorMode::Create,
            EditorDraft {
                action,
                script: Some(script),
            },
        );
    }

    fn open_editor(self: &Rc<Self>, anchor: &gtk::Button, mode: EditorMode, draft: EditorDraft) {
        let Some(host) = ModalHost::blurred_for(anchor) else {
            return;
        };
        let layout = editor_layout(mode, &draft.action);
        let form = Rc::new(EditorForm::new(mode, &draft.action, draft.script));
        layout.body.append(&form.root);
        layout.actions.prepend(&form.error);
        let content = layout.content;
        let layer = modal_layer(
            &content,
            &host.overlay,
            host.blurred_root.clone(),
            Some(Rc::new(|| true)),
        );
        let dismiss = dismisser(&layer, &host);
        for button in [&layout.cancel, &layout.close] {
            let dismiss = dismiss.clone();
            button.connect_clicked(move |_| dismiss());
        }
        let escape_dismiss = dismiss.clone();
        close_on_escape(&layer, move || escape_dismiss());
        crate::ui::modal::submit_on_enter(&layout.body, &layout.confirm);
        // The library's search entry must not inherit the modal's save binding.
        for button in &form.examples {
            examples::install(button, Rc::downgrade(&form));
        }
        self.save_on_confirm(
            &layout.confirm,
            EditorSession {
                mode,
                form: form.clone(),
                dismiss,
            },
        );
        host.overlay.add_overlay(&layer);
        form.focus_first();
    }

    fn save_on_confirm(self: &Rc<Self>, confirm: &gtk::Button, session: EditorSession) {
        let state = self.clone();
        confirm.connect_clicked(move |confirm| {
            let (definition, script) = match session.form.read() {
                Ok(parsed) => parsed,
                Err(message) => {
                    session.form.show_error(&message);
                    return;
                }
            };
            confirm.set_sensitive(false);
            let confirm = confirm.clone();
            let session = session.clone();
            state.confirm_editor(session.mode, definition, script, move |saved| {
                confirm.set_sensitive(true);
                match saved {
                    Ok(()) => (session.dismiss)(),
                    Err(message) => session.form.show_error(&message),
                }
            });
        });
    }

    fn duplicate(self: &Rc<Self>, button: &gtk::Button, action: &Rc<ActionHandle>) {
        let duplicated = self
            .registry
            .duplicate(action.id().to_owned(), copy_name(action.name()));
        let button = button.clone();
        glib::MainContext::default().spawn_local(async move {
            if let Err(error) = duplicated.await {
                show_error_dialog(
                    &button,
                    &crate::i18n::tr("Unable to duplicate the action"),
                    &error.user_message(),
                );
            }
        });
    }

    fn import(self: &Rc<Self>, button: &gtk::Button) {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title(crate::i18n::tr("Import an action folder"))
            .modal(true)
            .build();
        let state = self.clone();
        let anchor = button.clone();
        dialog.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
            let Ok(folder) = result else {
                return;
            };
            let Some(path) = folder.path() else {
                return;
            };
            let imported = state.registry.import(path);
            let state = state.clone();
            let anchor = anchor.clone();
            glib::MainContext::default().spawn_local(async move {
                match imported.await {
                    Ok(id) => {
                        PageState::render(&state);
                        tracing::info!(action = %id, "imported a custom action");
                        show_notice(
                            &anchor,
                            &crate::i18n::tr("Action imported"),
                            &rust_i18n::t!(
                                "“%{id}” was imported disabled. Review its script, then enable it.",
                                id = id
                            ),
                        );
                    }
                    Err(error) => {
                        show_error_dialog(
                            &anchor,
                            &crate::i18n::tr("Unable to import the action"),
                            &error.user_message(),
                        );
                    }
                }
            });
        });
    }

    fn export(self: &Rc<Self>, button: &gtk::Button, action: &Rc<ActionHandle>) {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title(crate::i18n::tr("Export this action to a folder"))
            .modal(true)
            .build();
        let state = self.clone();
        let anchor = button.clone();
        let id = action.id().to_owned();
        dialog.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
            let Ok(folder) = result else {
                return;
            };
            let Some(path) = folder.path() else {
                return;
            };
            state.export_to(&anchor, &id, &path);
        });
    }

    fn export_to(&self, anchor: &gtk::Button, id: &str, folder: &std::path::Path) {
        let exported = self.registry.export(id.to_owned(), folder.to_path_buf());
        let anchor = anchor.clone();
        glib::MainContext::default().spawn_local(async move {
            match exported.await {
                Ok(target) => show_notice(
                    &anchor,
                    &crate::i18n::tr("Action exported"),
                    &target.display().to_string(),
                ),
                Err(error) => show_error_dialog(
                    &anchor,
                    &crate::i18n::tr("Unable to export the action"),
                    &error.user_message(),
                ),
            }
        });
    }

    fn delete(self: &Rc<Self>, button: &gtk::Button, action: &Rc<ActionHandle>) {
        let Some(host) = ModalHost::blurred_for(button) else {
            return;
        };
        let layout = message_dialog_layout(
            crate::assets::icons::TRASH,
            &crate::i18n::tr("Delete this action?"),
            action.name(),
            &crate::i18n::tr("Delete"),
            ModalTone::Danger,
        );
        layout
            .body
            .append(&message_dialog_description(&crate::i18n::tr(
                "The action folder and its script are removed from your actions directory.",
            )));
        let content = layout.content;
        let layer = modal_layer(&content, &host.overlay, host.blurred_root.clone(), None);
        for button in [&layout.close, &layout.cancel] {
            let weak_layer = layer.downgrade();
            let overlay = host.overlay.clone();
            let root = host.blurred_root.clone();
            button.connect_clicked(move |_| {
                if let Some(layer) = weak_layer.upgrade() {
                    dismiss_modal_layer(&layer, &overlay, root.as_ref());
                }
            });
        }
        let state = self.clone();
        let weak_layer = layer.downgrade();
        let overlay = host.overlay.clone();
        let root = host.blurred_root.clone();
        let id = action.id().to_owned();
        let failure_anchor = button.clone();
        layout.confirm.connect_clicked(move |_| {
            let deleted = state.registry.delete(id.clone());
            if let Some(layer) = weak_layer.upgrade() {
                dismiss_modal_layer(&layer, &overlay, root.as_ref());
            }
            let state = state.clone();
            let failure_anchor = failure_anchor.clone();
            glib::MainContext::default().spawn_local(async move {
                match deleted.await {
                    Ok(()) => PageState::render(&state),
                    Err(error) => show_error_dialog(
                        &failure_anchor,
                        &crate::i18n::tr("Unable to delete the action"),
                        &error.user_message(),
                    ),
                }
            });
        });
        host.overlay.add_overlay(&layer);
        layout.confirm.grab_focus();
    }
}

/// The action an editor opens on and its script, if it has one.
struct EditorDraft {
    action: Rc<ActionHandle>,
    script: Option<ActionScript>,
}

/// An open editor: what saving does and how its dialog closes.
#[derive(Clone)]
struct EditorSession {
    mode: EditorMode,
    form: Rc<EditorForm>,
    dismiss: Rc<dyn Fn()>,
}

fn editor_layout(mode: EditorMode, action: &ActionHandle) -> ModalLayout {
    let (title, confirm) = match mode {
        EditorMode::Create => ("New action", "Create action"),
        EditorMode::Edit => ("Edit action", "Save changes"),
    };
    let layout = modal_layout(
        action_icon(action.definition.icon.as_deref()),
        &crate::i18n::tr(title),
        &crate::i18n::tr("Saved as action.toml under ~/.config/strata/actions/<id>"),
        &crate::i18n::tr(confirm),
    );
    layout.content.add_css_class("settings-action-dialog");
    if let Some(icon) = layout.close.child() {
        icon.set_halign(gtk::Align::Center);
        icon.set_valign(gtk::Align::Center);
    }
    layout
}

/// Closes the editor's layer while it is still shown.
fn dismisser(layer: &gtk::Box, host: &ModalHost) -> Rc<dyn Fn()> {
    let weak_layer = layer.downgrade();
    let overlay = host.overlay.clone();
    let root = host.blurred_root.clone();
    Rc::new(move || {
        if let Some(layer) = weak_layer.upgrade() {
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
        }
    })
}

fn action_copy(action: &ActionHandle) -> gtk::Box {
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(action.name()));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class("settings-option-title");
    let detail = gtk::Label::new(Some(&summary(action)));
    detail.set_xalign(0.0);
    detail.set_wrap(true);
    detail.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    detail.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&detail);
    copy
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionRowAction {
    Edit,
    Duplicate,
    Export,
    Delete,
}

fn problem_row(failure: &ActionLoadFailure) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("settings-option");
    let title = gtk::Label::new(Some(&rust_i18n::t!(
        "%{name}: %{error}",
        name = failure.directory,
        error = failure.error.user_message()
    )));
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.add_css_class("settings-option-title");
    title.add_css_class("settings-action-problem");
    row.append(&title);
    row
}

fn show_notice(anchor: &impl IsA<gtk::Widget>, title: &str, detail: &str) {
    let Some(host) = ModalHost::blurred_for(anchor) else {
        return;
    };
    let layout = message_dialog_layout(
        crate::assets::icons::CHECK,
        title,
        detail,
        &crate::i18n::tr("Close"),
        ModalTone::Accent,
    );
    let content = layout.content;
    let layer = modal_layer(&content, &host.overlay, host.blurred_root.clone(), None);
    for button in [&layout.close, &layout.confirm] {
        let weak_layer = layer.downgrade();
        let overlay = host.overlay.clone();
        let root = host.blurred_root.clone();
        button.connect_clicked(move |_| {
            if let Some(layer) = weak_layer.upgrade() {
                dismiss_modal_layer(&layer, &overlay, root.as_ref());
            }
        });
    }
    host.overlay.add_overlay(&layer);
    layout.close.grab_focus();
}

fn summary(action: &ActionHandle) -> String {
    let tr = crate::i18n::tr;
    let mut parts = vec![action.definition.run.runtime.localized_label()];
    parts.push(match action.definition.run.mode {
        ExecutionMode::PerItem => tr("per item"),
        ExecutionMode::WholeSelection => tr("whole selection"),
    });
    parts.push(match action.definition.when.kinds.as_slice() {
        [InputKind::File] => tr("files"),
        [InputKind::Folder] => tr("folders"),
        _ => tr("files and folders"),
    });
    if !action.definition.when.extensions.is_empty() {
        parts.push(crate::i18n::list(
            action.definition.when.extensions.iter().cloned(),
        ));
    }
    if !action.definition.enabled {
        parts.push(tr("disabled"));
    }
    if let Some(reason) = action.unavailable_reason() {
        // A capitalized standalone reason continues the summary in lower case where the
        // language does so after a separator.
        parts.push(crate::services::error_detail(reason.to_owned()));
    }
    parts.join(" · ")
}

fn copy_name(name: &str) -> String {
    let candidate = rust_i18n::t!("%{name} copy", name = name);
    candidate
        .chars()
        .take(crate::model::MAX_ACTION_NAME_CHARS)
        .collect()
}

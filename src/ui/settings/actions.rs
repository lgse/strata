// SPDX-License-Identifier: MIT

//! Settings → Actions: the manager for custom actions.
//!
//! The page edits the same `action.toml` files users can edit by hand, so
//! everything goes through [`ActionRegistry`]: no separate GUI-only format, no
//! writes outside the action directory, and every change reloads the catalog the
//! context menus read.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::model::{
    ACTION_SCHEMA_VERSION, ActionConditions, ActionDefinition, ActionRuntime, ErrorPolicy,
    ExecutionMode, InputKind, MenuPlacement, RunSpec, WorkingDirectory, suggest_id,
};
use crate::services::{
    ActionHandle, ActionLoadFailure, ActionRegistry, ActionScript, ActionWriteRequest,
};
use crate::ui::{
    actions::{ACTION_ICON_CHOICES, action_icon, is_known_action_icon},
    controls::{
        ModalTone, form_entry, form_error_label, menu_option, message_dialog_description,
        message_dialog_layout, modal_layout,
    },
    modal::{ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog},
};

use super::{append_heading, page_content, scrollable_page, search};

#[cfg(test)]
mod tests;

const MAX_EXTENSION_FIELD_CHARS: usize = 256;
const MAX_ARGUMENTS_FIELD_CHARS: usize = 2048;
const SCRIPT_EDITOR_HEIGHT: i32 = 140;

pub(super) fn actions_page() -> gtk::Widget {
    let content = page_content();
    content.add_css_class("settings-actions-page");
    append_heading(&content, "CUSTOM ACTIONS");
    let description = gtk::Label::new(Some(
        "Add your own scripts to the file and folder context menus. \
         Actions run with your permissions, so only enable scripts you trust.",
    ));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-section-description");
    content.append(&description);

    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    toolbar.add_css_class("settings-integration-actions");
    toolbar.set_margin_top(8);
    toolbar.set_margin_bottom(8);
    let new_action = gtk::Button::with_label("New action…");
    new_action.add_css_class("settings-action-button");
    search::tag(&new_action, "New custom action");
    let import = gtk::Button::with_label("Import…");
    import.add_css_class("settings-action-button");
    search::tag(&import, "Import custom action");
    toolbar.append(&new_action);
    toolbar.append(&import);
    content.append(&toolbar);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("settings-group");
    list.set_overflow(gtk::Overflow::Hidden);
    content.append(&list);
    let problems = gtk::Box::new(gtk::Orientation::Vertical, 0);
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
    // Another window, or a hand edit that was reloaded, refreshes this page.
    // The subscription ends with the page widget.
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
            let empty = gtk::Label::new(Some(
                "No custom actions yet. Create one to add it to the context menu.",
            ));
            empty.set_xalign(0.0);
            empty.set_wrap(true);
            empty.add_css_class("settings-option-description");
            // Rows inside a settings group supply their own padding; a lone
            // message needs the same inset so it cannot touch the group border.
            empty.add_css_class("settings-actions-empty");
            self.list.append(&empty);
        }
        for action in catalog.actions() {
            self.list.append(&self.row(action));
        }
        if !catalog.failures().is_empty() {
            append_heading(&self.problems, "PROBLEMS");
            let note = gtk::Label::new(Some(
                "These files could not be loaded. Fix them in the actions folder, or remove them.",
            ));
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
        let icon = crate::assets::primary_icon(action_icon(action.definition.icon.as_deref()), 20);
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);

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
        row.append(&copy);

        let toggle = gtk::Switch::builder()
            .active(action.definition.enabled)
            .valign(gtk::Align::Center)
            .tooltip_text("Enable this action in the context menu")
            .build();
        toggle.update_property(&[gtk::accessible::Property::Label("Enabled")]);
        let state = self.clone();
        let action_id = action.id().to_owned();
        let anchor = row.clone();
        toggle.connect_state_set(move |_, enabled| {
            if let Err(message) = state.set_enabled(&action_id, enabled) {
                show_error_dialog(&anchor, "Unable to save the action", &message);
            }
            glib::Propagation::Proceed
        });
        row.append(&toggle);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        buttons.set_valign(gtk::Align::Center);
        for (label, handler) in [
            ("Edit", ActionRowAction::Edit),
            ("Duplicate", ActionRowAction::Duplicate),
            ("Export…", ActionRowAction::Export),
            ("Delete", ActionRowAction::Delete),
        ] {
            let button = gtk::Button::with_label(label);
            button.add_css_class("settings-action-button");
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
            buttons.append(&button);
        }
        row.append(&buttons);
        row
    }

    fn set_enabled(self: &Rc<Self>, id: &str, enabled: bool) -> Result<(), String> {
        let Some(action) = self.registry.catalog().get(id).cloned() else {
            return Err(format!("The action “{id}” no longer exists"));
        };
        let mut definition = action.definition.clone();
        definition.enabled = enabled;
        Self::save(self, &definition, None)
    }

    fn save(
        self: &Rc<Self>,
        definition: &ActionDefinition,
        script: Option<ActionScript>,
    ) -> Result<(), String> {
        self.registry
            .write(&ActionWriteRequest {
                definition: definition.clone(),
                script,
            })
            .map_err(|error| error.to_string())?;
        PageState::render(self);
        Ok(())
    }

    fn edit(self: &Rc<Self>, button: &gtk::Button, action: Rc<ActionHandle>) {
        let script = match self.registry.read_script(action.id()) {
            Ok(script) => script,
            Err(error) => {
                show_error_dialog(button, "Unable to read the action", &error.to_string());
                return;
            }
        };
        self.open_editor(button, EditorMode::Edit, action, script);
    }

    fn start_new(self: &Rc<Self>, button: &gtk::Button) {
        let action = Rc::new(ActionHandle {
            directory: std::path::PathBuf::new(),
            definition: draft_definition(ActionRuntime::Python, ExecutionMode::WholeSelection),
            availability: crate::services::ActionAvailability::Unavailable {
                reason: "This action has not been saved yet".to_owned(),
            },
        });
        let script = ActionScript {
            file_name: "main.py".to_owned(),
            contents: python_template(),
        };
        self.open_editor(button, EditorMode::Create, action, Some(script));
    }

    fn open_editor(
        self: &Rc<Self>,
        anchor: &gtk::Button,
        mode: EditorMode,
        action: Rc<ActionHandle>,
        script: Option<ActionScript>,
    ) {
        let Some(host) = ModalHost::blurred_for(anchor) else {
            return;
        };
        let title = match mode {
            EditorMode::Create => "New action",
            EditorMode::Edit => "Edit action",
        };
        let layout = modal_layout(
            action_icon(action.definition.icon.as_deref()),
            title,
            "Saved as action.toml under ~/.config/strata/actions",
            "Save",
        );
        let form = EditorForm::new(mode, &action, script);
        layout.body.append(&form.root);
        let content = layout.content;
        let layer = modal_layer(&content, &host.overlay, host.blurred_root.clone(), None);
        let overlay = host.overlay.clone();
        let blurred_root = host.blurred_root.clone();
        let dismiss = {
            let weak_layer = layer.downgrade();
            let overlay = overlay.clone();
            let root = blurred_root.clone();
            Rc::new(move || {
                if let Some(layer) = weak_layer.upgrade() {
                    dismiss_modal_layer(&layer, &overlay, root.as_ref());
                }
            })
        };
        for button in [&layout.cancel, &layout.close] {
            let dismiss = dismiss.clone();
            button.connect_clicked(move |_| dismiss());
        }
        let escape = gtk::EventControllerKey::new();
        let dismiss_for_escape = dismiss.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_for_escape();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        crate::ui::modal::submit_on_enter(&layout.body, &layout.confirm);

        let state = self.clone();
        let form_for_save = form.clone();
        let weak_layer = layer.downgrade();
        let overlay_for_save = overlay.clone();
        let root_for_save = blurred_root.clone();
        layout.confirm.connect_clicked(move |_| {
            let (definition, script) = match form_for_save.read() {
                Ok(parsed) => parsed,
                Err(message) => {
                    form_for_save.show_error(&message);
                    return;
                }
            };
            match state.save(&definition, script) {
                Ok(()) => {
                    if let Some(layer) = weak_layer.upgrade() {
                        dismiss_modal_layer(&layer, &overlay_for_save, root_for_save.as_ref());
                    }
                }
                Err(message) => form_for_save.show_error(&message),
            }
        });
        overlay.add_overlay(&layer);
        form.focus_first();
    }

    fn duplicate(self: &Rc<Self>, button: &gtk::Button, action: &Rc<ActionHandle>) {
        let script = match self.registry.read_script(action.id()) {
            Ok(script) => script,
            Err(error) => {
                show_error_dialog(button, "Unable to duplicate the action", &error.to_string());
                return;
            }
        };
        let mut definition = action.definition.clone();
        definition.id = unique_copy_id(&self.registry, &definition.id);
        definition.name = copy_name(&definition.name);
        if let Err(message) = self.save(&definition, script) {
            show_error_dialog(button, "Unable to duplicate the action", &message);
        }
    }

    fn import(self: &Rc<Self>, button: &gtk::Button) {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Import an action folder")
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
            match state.registry.import(&path) {
                Ok(id) => {
                    PageState::render(&state);
                    tracing::info!(action = %id, "imported a custom action");
                    show_notice(
                        &anchor,
                        "Action imported",
                        &format!(
                            "“{id}” was imported disabled. Review its script, then enable it."
                        ),
                    );
                }
                Err(error) => {
                    show_error_dialog(&anchor, "Unable to import the action", &error.to_string());
                }
            }
        });
    }

    fn export(self: &Rc<Self>, button: &gtk::Button, action: &Rc<ActionHandle>) {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Export this action to a folder")
            .modal(true)
            .build();
        let registry = self.registry.clone();
        let anchor = button.clone();
        let id = action.id().to_owned();
        dialog.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
            let Ok(folder) = result else {
                return;
            };
            let Some(path) = folder.path() else {
                return;
            };
            match registry.export(&id, &path) {
                Ok(target) => {
                    show_notice(&anchor, "Action exported", &target.display().to_string())
                }
                Err(error) => {
                    show_error_dialog(&anchor, "Unable to export the action", &error.to_string())
                }
            }
        });
    }

    fn delete(self: &Rc<Self>, button: &gtk::Button, action: &Rc<ActionHandle>) {
        let Some(host) = ModalHost::blurred_for(button) else {
            return;
        };
        let layout = message_dialog_layout(
            crate::assets::icons::TRASH,
            "Delete this action?",
            action.name(),
            "Delete",
            ModalTone::Danger,
        );
        layout.body.append(&message_dialog_description(
            "The action folder and its script are removed from your actions directory.",
        ));
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
            let result = state.registry.delete(&id);
            if let Some(layer) = weak_layer.upgrade() {
                dismiss_modal_layer(&layer, &overlay, root.as_ref());
            }
            match result {
                Ok(()) => PageState::render(&state),
                Err(error) => show_error_dialog(
                    &failure_anchor,
                    "Unable to delete the action",
                    &error.to_string(),
                ),
            }
        });
        host.overlay.add_overlay(&layer);
        layout.confirm.grab_focus();
    }
}

/// Which button of an action row was clicked.
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
    let title = gtk::Label::new(Some(&format!("{}: {}", failure.directory, failure.error)));
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.add_css_class("settings-option-title");
    title.add_css_class("settings-action-problem");
    row.append(&title);
    row
}

/// A plain informational dialog, used for import/export results.
fn show_notice(anchor: &impl IsA<gtk::Widget>, title: &str, detail: &str) {
    let Some(host) = ModalHost::blurred_for(anchor) else {
        return;
    };
    let layout = message_dialog_layout(
        crate::assets::icons::CHECK,
        title,
        detail,
        "Close",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditorMode {
    Create,
    Edit,
}

/// The editor form. Fields mirror the manifest; validation runs before a write,
/// so an invalid definition never reaches the store.
#[derive(Clone)]
struct EditorForm {
    mode: EditorMode,
    root: gtk::Box,
    error: gtk::Label,
    name: gtk::Entry,
    id: gtk::Entry,
    description: gtk::Entry,
    entrypoint: gtk::Entry,
    script: gtk::TextView,
    program: gtk::Entry,
    arguments: gtk::Entry,
    confirm: gtk::Switch,
    enabled: gtk::Switch,
    files: gtk::CheckButton,
    folders: gtk::CheckButton,
    extensions: gtk::Entry,
    max_items: gtk::Entry,
    entrypoint_field: gtk::Box,
    script_field: gtk::Box,
    program_field: gtk::Box,
    arguments_field: gtk::Box,
    selected_icon: Rc<std::cell::RefCell<Option<String>>>,
    selected_runtime: Rc<std::cell::Cell<ActionRuntime>>,
    selected_mode: Rc<std::cell::Cell<ExecutionMode>>,
    selected_policy: Rc<std::cell::Cell<ErrorPolicy>>,
    selected_directory: Rc<std::cell::Cell<WorkingDirectory>>,
    selected_placement: Rc<std::cell::Cell<MenuPlacement>>,
}

impl EditorForm {
    fn new(mode: EditorMode, action: &ActionHandle, script: Option<ActionScript>) -> Self {
        let definition = &action.definition;
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("settings-action-editor");
        let error = form_error_label();

        let name = form_entry();
        name.set_text(&definition.name);
        let id = form_entry();
        id.set_text(&definition.id);
        // Renaming an id would create a second action rather than renaming this
        // one, so an existing action keeps its identity.
        id.set_editable(mode == EditorMode::Create);
        id.set_sensitive(mode == EditorMode::Create);
        let description = form_entry();
        description.set_text(definition.description.as_deref().unwrap_or(""));
        description.set_placeholder_text(Some("Shown as the menu tooltip (optional)"));

        let selected_icon = Rc::new(std::cell::RefCell::new(definition.icon.clone()));
        let icon = icon_chooser(&selected_icon);

        let selected_runtime = Rc::new(std::cell::Cell::new(definition.run.runtime));
        let runtime = segmented(
            &selected_runtime,
            &[
                ("Python", ActionRuntime::Python),
                ("Bash", ActionRuntime::Bash),
                ("Command", ActionRuntime::Command),
            ],
        );

        let entrypoint = form_entry();
        entrypoint.set_text(definition.run.entrypoint.as_deref().unwrap_or("main.py"));
        let script_buffer = gtk::TextBuffer::new(None);
        if let Some(script) = script.as_ref() {
            script_buffer.set_text(&script.contents);
        }
        let script_view = gtk::TextView::builder()
            .buffer(&script_buffer)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::None)
            .height_request(SCRIPT_EDITOR_HEIGHT)
            .build();
        let script_scroll = gtk::ScrolledWindow::builder()
            .child(&script_view)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        script_scroll.add_css_class("settings-action-script");

        let program = form_entry();
        program.set_text(definition.run.program.as_deref().unwrap_or(""));
        program.set_placeholder_text(Some("Installed program, for example make"));
        let arguments = form_entry();
        arguments.set_text(&definition.run.args.join("\n"));
        arguments.set_placeholder_text(Some("One per line; {path}, {paths}, or {parent}"));

        let selected_mode = Rc::new(std::cell::Cell::new(definition.run.mode));
        let mode_control = segmented(
            &selected_mode,
            &[
                ("Whole selection", ExecutionMode::WholeSelection),
                ("Per item", ExecutionMode::PerItem),
            ],
        );
        let selected_policy = Rc::new(std::cell::Cell::new(definition.run.on_error));
        let on_error = segmented(
            &selected_policy,
            &[
                ("Continue", ErrorPolicy::Continue),
                ("Stop", ErrorPolicy::Stop),
            ],
        );
        let selected_directory = Rc::new(std::cell::Cell::new(definition.run.working_directory));
        let working_directory = segmented(
            &selected_directory,
            &[
                ("Invoking folder", WorkingDirectory::Parent),
                ("Home", WorkingDirectory::Home),
                ("Action folder", WorkingDirectory::Action),
            ],
        );
        let selected_placement = Rc::new(std::cell::Cell::new(definition.menu));
        let placement = segmented(
            &selected_placement,
            &[
                ("Menu item", MenuPlacement::Top),
                ("Actions submenu", MenuPlacement::Submenu),
            ],
        );

        // A GtkSwitch fills whatever allocation it is given, which turns the
        // editor's stacked fields into full-width ovals. Keep it at its natural
        // size under its label instead.
        let confirm = gtk::Switch::builder()
            .active(definition.run.confirm)
            .halign(gtk::Align::Start)
            .build();
        let enabled = gtk::Switch::builder()
            .active(definition.enabled)
            .halign(gtk::Align::Start)
            .build();
        let files = gtk::CheckButton::with_label("Files");
        let folders = gtk::CheckButton::with_label("Folders");
        if definition.when.kinds.is_empty() {
            files.set_active(true);
            folders.set_active(true);
        } else {
            files.set_active(definition.when.kinds.contains(&InputKind::File));
            folders.set_active(definition.when.kinds.contains(&InputKind::Folder));
        }
        let extensions = form_entry();
        extensions.set_text(&definition.when.extensions.join(", "));
        extensions.set_placeholder_text(Some("png, jpg (empty means every extension)"));
        let max_items = form_entry();
        max_items.set_text(
            &definition
                .when
                .max_items
                .map(|value| value.to_string())
                .unwrap_or_default(),
        );
        max_items.set_placeholder_text(Some("No limit"));

        let id_hint = if mode == EditorMode::Create {
            "Folder name under the actions directory: lowercase letters, digits, dashes"
        } else {
            "The action id cannot change; duplicate it to create a variant"
        };
        let kind_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        kind_row.append(&files);
        kind_row.append(&folders);
        let entrypoint_field = field(
            "Script file",
            "A plain file name beside action.toml",
            &entrypoint,
        );
        let script_field = field("Script", "Runs with your permissions", &script_scroll);
        let program_field = field(
            "Program",
            "Required for command actions instead of a script",
            &program,
        );
        let arguments_field = field(
            "Arguments",
            "One per line: {path} is one item, {paths} the whole selection, {parent} the folder",
            &arguments,
        );

        root.append(&error);
        root.append(&field("Name", "Shown in the context menu", &name));
        root.append(&field("Id", id_hint, &id));
        root.append(&field("Description", "Optional tooltip", &description));
        root.append(&field("Icon", "Bundled Lucide icon", &icon));
        root.append(&field("Runtime", "How the action is started", &runtime));
        root.append(&entrypoint_field);
        root.append(&script_field);
        root.append(&program_field);
        root.append(&arguments_field);
        root.append(&field(
            "Run",
            "Once, or once per selected item",
            &mode_control,
        ));
        root.append(&field(
            "On failure",
            "Only used when running per item",
            &on_error,
        ));
        root.append(&field(
            "Working folder",
            "Where the process starts",
            &working_directory,
        ));
        root.append(&field("Placement", "Where the action appears", &placement));
        root.append(&field(
            "Applies to",
            "Which selected entries offer this action",
            &kind_row,
        ));
        root.append(&field(
            "Extensions",
            "Comma separated, without dots",
            &extensions,
        ));
        root.append(&field(
            "Maximum items",
            "Optional selection limit",
            &max_items,
        ));
        root.append(&field(
            "Confirm",
            "Ask before running this action",
            &confirm,
        ));
        root.append(&field("Enabled", "Show this action in menus", &enabled));

        let form = Self {
            mode,
            root,
            error,
            name,
            id,
            description,
            entrypoint,
            script: script_view,
            program,
            arguments,
            confirm,
            enabled,
            files,
            folders,
            extensions,
            max_items,
            entrypoint_field,
            script_field,
            program_field,
            arguments_field,
            selected_icon,
            selected_runtime,
            selected_mode,
            selected_policy,
            selected_directory,
            selected_placement,
        };
        // Script and command rows swap when the runtime changes.
        let mut child = runtime.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            let form = form.clone();
            if let Some(toggle) = widget.downcast_ref::<gtk::ToggleButton>() {
                toggle.connect_toggled(move |_| {
                    if form.entrypoint.is_sensitive() || form.program.is_sensitive() {
                        form.sync_runtime_rows();
                    }
                });
            }
        }
        form.sync_runtime_rows();
        form
    }

    fn focus_first(&self) {
        match self.mode {
            EditorMode::Create => {
                self.name.grab_focus();
            }
            EditorMode::Edit => {
                self.description.grab_focus();
            }
        }
    }

    fn show_error(&self, message: &str) {
        self.error.set_text(message);
        self.error.set_visible(true);
    }

    /// Shows only the fields the selected runtime uses.
    fn sync_runtime_rows(&self) {
        let runtime = self.selected_runtime.get();
        let is_script = runtime != ActionRuntime::Command;
        self.entrypoint_field.set_visible(is_script);
        self.script_field.set_visible(is_script);
        self.program_field.set_visible(!is_script);
        self.arguments_field.set_visible(!is_script);
        if is_script {
            let suggested = default_entrypoint(runtime);
            let current = self.entrypoint.text();
            let current = current.trim();
            if current.is_empty() || default_entrypoint_for_any(current).is_some() {
                self.entrypoint.set_text(suggested);
            }
        }
    }

    /// Builds the definition and script the store will write, or explains why it
    /// cannot.
    fn read(&self) -> Result<(ActionDefinition, Option<ActionScript>), String> {
        let name = self.name.text().trim().to_owned();
        if name.is_empty() {
            return Err("Enter a name".to_owned());
        }
        let id = match self.mode {
            EditorMode::Create => {
                let typed = self.id.text().trim().to_owned();
                if typed.is_empty() {
                    suggest_id(&name)
                } else {
                    typed
                }
            }
            EditorMode::Edit => self.id.text().trim().to_owned(),
        };
        if self.extensions.text().chars().count() > MAX_EXTENSION_FIELD_CHARS {
            return Err("That is too many extensions".to_owned());
        }
        if self.arguments.text().chars().count() > MAX_ARGUMENTS_FIELD_CHARS {
            return Err("That is too many arguments".to_owned());
        }
        let mut kinds = Vec::new();
        if self.files.is_active() {
            kinds.push(InputKind::File);
        }
        if self.folders.is_active() {
            kinds.push(InputKind::Folder);
        }
        let max_items = match self.max_items.text().trim() {
            "" => None,
            value => Some(
                value
                    .parse::<usize>()
                    .map_err(|_| "The maximum item count must be a number".to_owned())?,
            ),
        };
        let runtime = self.selected_runtime.get();
        let is_script = runtime != ActionRuntime::Command;
        // Report the empty field the user can see, rather than the manifest
        // vocabulary the model would use for a hand-written file.
        if !is_script && self.program.text().trim().is_empty() {
            return Err("Enter the program to run".to_owned());
        }
        if is_script && self.entrypoint.text().trim().is_empty() {
            return Err("Enter a script file name".to_owned());
        }
        let definition = ActionDefinition {
            schema_version: ACTION_SCHEMA_VERSION,
            id,
            name,
            description: {
                let description = self.description.text().trim().to_owned();
                (!description.is_empty()).then_some(description)
            },
            icon: self
                .selected_icon
                .borrow()
                .clone()
                .filter(|icon| is_known_action_icon(icon)),
            enabled: self.enabled.is_active(),
            menu: self.selected_placement.get(),
            when: ActionConditions {
                kinds,
                extensions: self
                    .extensions
                    .text()
                    .split(',')
                    .map(|extension| {
                        extension
                            .trim()
                            .trim_start_matches('.')
                            .to_ascii_lowercase()
                    })
                    .filter(|extension| !extension.is_empty())
                    .collect(),
                mime_types: Vec::new(),
                min_items: 1,
                max_items,
            },
            run: RunSpec {
                runtime,
                entrypoint: is_script.then(|| self.entrypoint.text().trim().to_owned()),
                program: (!is_script).then(|| self.program.text().trim().to_owned()),
                args: self
                    .arguments
                    .text()
                    .lines()
                    .map(|line| line.trim().to_owned())
                    .filter(|line| !line.is_empty())
                    .collect(),
                mode: self.selected_mode.get(),
                on_error: self.selected_policy.get(),
                working_directory: self.selected_directory.get(),
                confirm: self.confirm.is_active(),
            },
        };
        // Validate here so the dialog reports a precise problem before writing.
        definition.validate().map_err(|error| error.to_string())?;
        let script = match definition.run.script_entrypoint() {
            Some(entrypoint) => {
                let buffer = self.script.buffer();
                let contents = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string();
                Some(ActionScript {
                    file_name: entrypoint.to_owned(),
                    contents,
                })
            }
            None => None,
        };
        Ok((definition, script))
    }
}

fn default_entrypoint(runtime: ActionRuntime) -> &'static str {
    match runtime {
        ActionRuntime::Python => "main.py",
        _ => "run.sh",
    }
}

fn default_entrypoint_for_any(name: &str) -> Option<ActionRuntime> {
    match name {
        "main.py" => Some(ActionRuntime::Python),
        "run.sh" => Some(ActionRuntime::Bash),
        _ => None,
    }
}

fn field(title: &str, description: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    row.add_css_class("settings-action-field");
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.add_css_class("settings-option-title");
    row.append(&label);
    if !description.is_empty() {
        let hint = gtk::Label::new(Some(description));
        hint.set_xalign(0.0);
        hint.set_wrap(true);
        hint.add_css_class("settings-option-description");
        row.append(&hint);
    }
    row.append(control);
    row
}

/// A row of mutually exclusive buttons backed by a shared cell.
fn segmented<T: Copy + PartialEq + 'static>(
    selected: &Rc<std::cell::Cell<T>>,
    choices: &[(&str, T)],
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("settings-action-segmented");
    for (label, value) in choices {
        let button = gtk::ToggleButton::with_label(label);
        button.add_css_class("settings-action-choice");
        button.set_active(selected.get() == *value);
        let selected = selected.clone();
        let value = *value;
        let clicked = button.clone();
        button.connect_clicked(move |_| {
            selected.set(value);
            if let Some(parent) = clicked.parent().and_downcast::<gtk::Box>() {
                let mut child = parent.first_child();
                while let Some(widget) = child {
                    if let Some(sibling) = widget.downcast_ref::<gtk::ToggleButton>() {
                        sibling.set_active(sibling == &clicked);
                    }
                    child = widget.next_sibling();
                }
            }
        });
        row.append(&button);
    }
    row
}

fn icon_chooser(selected: &Rc<std::cell::RefCell<Option<String>>>) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    let current = selected.borrow().clone();
    button.set_child(Some(&icon_preview(current.as_deref())));
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    menu.add_css_class("column-menu");
    let popover = gtk::Popover::builder()
        .position(gtk::PositionType::Bottom)
        .has_arrow(false)
        .build();
    popover.set_child(Some(&menu));
    for (slug, _) in ACTION_ICON_CHOICES {
        let (row, _) = menu_option(slug, selected.borrow().as_deref() == Some(*slug));
        row.add_css_class("settings-action-icon-choice");
        let selected = selected.clone();
        let slug = (*slug).to_owned();
        let weak_popover = popover.downgrade();
        let weak_button = button.downgrade();
        row.connect_clicked(move |_| {
            selected.replace(Some(slug.clone()));
            if let Some(button) = weak_button.upgrade() {
                button.set_child(Some(&icon_preview(Some(&slug))));
            }
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
        });
        menu.append(&row);
    }
    button.set_popover(Some(&popover));
    button
}

fn icon_preview(slug: Option<&str>) -> gtk::Box {
    let preview = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    preview.append(&crate::assets::primary_icon(action_icon(slug), 16));
    let label = gtk::Label::new(Some(slug.unwrap_or("default")));
    preview.append(&label);
    preview
}

/// A blank definition for a new action, before the editor fills it in.
fn draft_definition(runtime: ActionRuntime, mode: ExecutionMode) -> ActionDefinition {
    ActionDefinition {
        schema_version: ACTION_SCHEMA_VERSION,
        id: String::new(),
        name: String::new(),
        description: None,
        icon: None,
        enabled: true,
        menu: MenuPlacement::Submenu,
        when: ActionConditions {
            kinds: Vec::new(),
            extensions: Vec::new(),
            mime_types: Vec::new(),
            min_items: 1,
            max_items: None,
        },
        run: RunSpec {
            runtime,
            entrypoint: Some(default_entrypoint(runtime).to_owned()),
            program: None,
            args: Vec::new(),
            mode,
            on_error: ErrorPolicy::Continue,
            working_directory: WorkingDirectory::Parent,
            confirm: false,
        },
    }
}

fn python_template() -> String {
    "#!/usr/bin/env python3\nfrom strata_actions import context\n\nctx = context()\nfor path in ctx.paths:\n    ctx.log(f\"Processing {path}\")\n"
        .to_owned()
}

fn summary(action: &ActionHandle) -> String {
    let mut parts = vec![action.definition.run.runtime.label().to_owned()];
    parts.push(match action.definition.run.mode {
        ExecutionMode::PerItem => "per item".to_owned(),
        ExecutionMode::WholeSelection => "whole selection".to_owned(),
    });
    match action.definition.when.kinds.as_slice() {
        [InputKind::File] => parts.push("files".to_owned()),
        [InputKind::Folder] => parts.push("folders".to_owned()),
        _ => parts.push("files and folders".to_owned()),
    }
    if !action.definition.when.extensions.is_empty() {
        parts.push(action.definition.when.extensions.join(", "));
    }
    if !action.definition.enabled {
        parts.push("disabled".to_owned());
    }
    if let Some(reason) = action.unavailable_reason() {
        parts.push(reason.to_owned());
    }
    parts.join(" · ")
}

fn copy_name(name: &str) -> String {
    let candidate = format!("{name} copy");
    candidate
        .chars()
        .take(crate::model::MAX_ACTION_NAME_CHARS)
        .collect()
}

fn unique_copy_id(registry: &ActionRegistry, id: &str) -> String {
    for suffix in 0..1000 {
        let candidate = if suffix == 0 {
            format!("{id}-copy")
        } else {
            format!("{id}-copy-{suffix}")
        };
        let candidate: String = candidate
            .chars()
            .take(crate::model::MAX_ACTION_ID_CHARS)
            .collect();
        if registry.catalog().get(&candidate).is_none() {
            return candidate;
        }
    }
    id.to_owned()
}

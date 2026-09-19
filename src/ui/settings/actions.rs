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
use sourceview5::prelude::*;

use crate::model::{
    ACTION_SCHEMA_VERSION, ActionConditions, ActionDefinition, ActionError, ActionRuntime,
    ErrorPolicy, ExecutionMode, InputKind, MenuPlacement, RunSpec, WorkingDirectory, suggest_id,
};
use crate::services::{
    ActionHandle, ActionLoadFailure, ActionRegistry, ActionScript, ActionWriteRequest,
};
use crate::ui::{
    actions::{ACTION_ICON_CHOICES, action_icon, is_known_action_icon},
    controls::{
        ModalTone, form_check_button, form_entry, form_error_label, message_dialog_description,
        message_dialog_layout, modal_layout, segmented_control,
    },
    modal::{ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog},
};

use super::{append_heading, page_content, scrollable_page, search};

#[cfg(test)]
mod tests;

const MAX_EXTENSION_FIELD_CHARS: usize = 256;
const MAX_ARGUMENTS_FIELD_CHARS: usize = 2048;
const GENERAL_TAB: u32 = 0;
const SCRIPT_TAB: u32 = 1;
const BEHAVIOR_TAB: u32 = 2;

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
            "Saved as action.toml under ~/.config/strata/actions/<id>",
            match mode {
                EditorMode::Create => "Create action",
                EditorMode::Edit => "Save changes",
            },
        );
        layout.content.add_css_class("settings-action-dialog");
        if let Some(icon) = layout.close.child() {
            icon.set_halign(gtk::Align::Center);
            icon.set_valign(gtk::Align::Center);
        }
        let form = EditorForm::new(mode, &action, script);
        layout.body.append(&form.root);
        layout.actions.prepend(&form.error);
        let content = layout.content;
        let layer = modal_layer(
            &content,
            &host.overlay,
            host.blurred_root.clone(),
            Some(Rc::new(|| true)),
        );
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
    tabs: gtk::Notebook,
    name: gtk::Entry,
    id: gtk::Entry,
    description: gtk::Entry,
    entrypoint: gtk::Entry,
    script: sourceview5::View,
    program: gtk::Entry,
    arguments: gtk::TextView,
    confirm: gtk::Switch,
    enabled: gtk::Switch,
    files: gtk::CheckButton,
    folders: gtk::CheckButton,
    extensions: gtk::Entry,
    max_items: gtk::Entry,
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
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("settings-action-editor");
        let error = form_error_label();
        error.set_wrap(true);
        error.set_max_width_chars(44);
        error.set_hexpand(true);
        error.set_valign(gtk::Align::Center);
        let tabs = gtk::Notebook::builder()
            .show_border(false)
            .height_request(520)
            .build();
        tabs.add_css_class("settings-action-tabs");
        root.append(&tabs);
        let general = editor_page(&tabs, "General");
        let script_page = editor_page(&tabs, "Script");
        let behavior = editor_page(&tabs, "Behavior");
        behavior.set_spacing(0);

        let name = form_entry();
        name.set_text(&definition.name);
        name.set_placeholder_text(Some("Batch rename"));
        let id = form_entry();
        id.set_text(&definition.id);
        id.set_placeholder_text(Some("batch-rename"));
        if mode == EditorMode::Create {
            let weak_id = id.downgrade();
            name.connect_changed(move |name| {
                if let Some(id) = weak_id.upgrade() {
                    let suggestion = suggest_id(name.text().trim());
                    id.set_placeholder_text(Some(&suggestion));
                }
            });
        }
        // Renaming an id would create a second action rather than renaming this
        // one, so an existing action keeps its identity.
        id.set_editable(mode == EditorMode::Create);
        id.set_sensitive(mode == EditorMode::Create);
        let description = form_entry();
        description.set_text(definition.description.as_deref().unwrap_or(""));
        description.set_placeholder_text(Some("Shown as the menu tooltip"));

        let selected_icon = Rc::new(std::cell::RefCell::new(definition.icon.clone()));
        let icon = icon_chooser(&selected_icon);

        let selected_runtime = Rc::new(std::cell::Cell::new(definition.run.runtime));
        let (runtime, runtime_buttons) = segmented(
            &selected_runtime,
            &[
                ("Python", ActionRuntime::Python),
                ("Bash", ActionRuntime::Bash),
                ("Command", ActionRuntime::Command),
            ],
        );

        let entrypoint = form_entry();
        entrypoint.set_text(definition.run.entrypoint.as_deref().unwrap_or("main.py"));
        let python = script_buffer("python3", &python_template());
        let bash = script_buffer("sh", bash_template());
        if let Some(script) = script.as_ref() {
            match definition.run.runtime {
                ActionRuntime::Bash => bash.set_text(&script.contents),
                _ => python.set_text(&script.contents),
            }
        }
        let script_view = sourceview5::View::builder()
            .monospace(true)
            .show_line_numbers(true)
            .tab_width(4)
            .insert_spaces_instead_of_tabs(true)
            .auto_indent(true)
            .left_margin(12)
            .right_margin(12)
            .top_margin(10)
            .bottom_margin(10)
            .wrap_mode(gtk::WrapMode::None)
            .build();
        let script_scroll = editor_scroll(&script_view);
        script_scroll.set_min_content_height(280);
        script_scroll.set_vexpand(true);

        let program = form_entry();
        program.set_text(definition.run.program.as_deref().unwrap_or(""));
        program.set_placeholder_text(Some("Installed program, for example make"));
        let arguments = gtk::TextView::builder()
            .monospace(true)
            .accepts_tab(false)
            .left_margin(12)
            .right_margin(12)
            .top_margin(10)
            .bottom_margin(10)
            .build();
        arguments.buffer().set_text(&definition.run.args.join("\n"));
        let arguments_scroll = editor_scroll(&arguments);
        arguments_scroll.set_min_content_height(160);
        arguments_scroll.set_vexpand(true);

        let selected_mode = Rc::new(std::cell::Cell::new(definition.run.mode));
        let (mode_control, mode_buttons) = segmented(
            &selected_mode,
            &[
                ("Whole selection", ExecutionMode::WholeSelection),
                ("Per item", ExecutionMode::PerItem),
            ],
        );
        let selected_policy = Rc::new(std::cell::Cell::new(definition.run.on_error));
        let (on_error, policy_buttons) = segmented(
            &selected_policy,
            &[
                ("Continue", ErrorPolicy::Continue),
                ("Stop", ErrorPolicy::Stop),
            ],
        );
        let selected_directory = Rc::new(std::cell::Cell::new(definition.run.working_directory));
        let (working_directory, _) = segmented(
            &selected_directory,
            &[
                ("Invoking", WorkingDirectory::Parent),
                ("Home", WorkingDirectory::Home),
                ("Action", WorkingDirectory::Action),
            ],
        );
        let selected_placement = Rc::new(std::cell::Cell::new(definition.menu));
        let (placement, _) = segmented(
            &selected_placement,
            &[
                ("Menu item", MenuPlacement::Top),
                ("Actions submenu", MenuPlacement::Submenu),
            ],
        );

        let behavior_controls = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        for control in [&mode_control, &on_error, &working_directory, &placement] {
            behavior_controls.add_widget(control);
        }

        let (confirm_row, confirm) = super::settings_option(
            "Confirm",
            "Ask before running this action",
            definition.run.confirm,
        );
        confirm_row.add_css_class("settings-action-option");
        let (enabled_row, enabled) =
            super::settings_option("Enabled", "Show this action in menus", definition.enabled);
        enabled_row.add_css_class("settings-action-option");
        enabled_row.add_css_class("settings-action-enabled");
        let files = form_check_button("Files");
        let folders = form_check_button("Folders");
        if definition.when.kinds.is_empty() {
            files.set_active(true);
            folders.set_active(true);
        } else {
            files.set_active(definition.when.kinds.contains(&InputKind::File));
            folders.set_active(definition.when.kinds.contains(&InputKind::Folder));
        }
        let extensions = form_entry();
        extensions.set_text(&definition.when.extensions.join(", "));
        extensions.set_placeholder_text(Some("Any"));
        extensions.set_width_chars(18);
        let max_items = form_entry();
        max_items.set_text(
            &definition
                .when
                .max_items
                .map(|value| value.to_string())
                .unwrap_or_default(),
        );
        max_items.set_placeholder_text(Some("No limit"));
        max_items.set_width_chars(10);
        max_items.set_input_purpose(gtk::InputPurpose::Digits);

        id.set_tooltip_text(Some(if mode == EditorMode::Create {
            "Folder name: lowercase letters, digits, and dashes. Leave blank to use the name."
        } else {
            "The action id cannot change; duplicate it to create a variant."
        }));
        let kind_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        kind_row.append(&files);
        kind_row.append(&folders);
        let entrypoint_field = field("Script file", "Saved beside action.toml", &entrypoint);
        let script_field = field("Script", "Runs with your permissions", &script_scroll);
        let program_field = field("Program", "Installed executable", &program);
        let arguments_field = field(
            "Arguments",
            "One per line · {path}, {paths}, or {parent}",
            &arguments_scroll,
        );

        let identity = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        let name_field = field("Name", "Shown in the context menu", &name);
        name_field.set_hexpand(true);
        identity.append(&name_field);
        id.set_width_chars(28);
        let id_field = field("Id", "Folder name", &id);
        id_field.set_hexpand(false);
        identity.append(&id_field);
        general.append(&identity);
        general.append(&field("Description", "Optional tooltip", &description));
        general.append(&field("Icon", "Bundled Lucide icon", &icon));
        general.append(&enabled_row);

        let launch = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        let launch_controls = gtk::SizeGroup::new(gtk::SizeGroupMode::Vertical);
        launch_controls.add_widget(&runtime);
        launch_controls.add_widget(&entrypoint);
        launch_controls.add_widget(&program);
        let runtime_field = field("Runtime", "How it starts", &runtime);
        runtime_field.set_hexpand(false);
        launch.append(&runtime_field);
        entrypoint_field.set_hexpand(true);
        program_field.set_hexpand(true);
        launch.append(&entrypoint_field);
        launch.append(&program_field);
        script_page.append(&launch);
        script_field.set_vexpand(true);
        arguments_field.set_vexpand(true);
        script_page.append(&script_field);
        script_page.append(&arguments_field);
        label_control(&script_view, "Script", "Runs with your permissions");
        label_control(
            &arguments,
            "Arguments",
            "One argument per line; {path}, {paths}, or {parent}",
        );

        behavior.append(&option_row(
            "Run",
            "Once, or once per selected item",
            &mode_control,
        ));
        let failure_row = option_row("On failure", "Only used when running per item", &on_error);
        failure_row.set_sensitive(selected_mode.get() == ExecutionMode::PerItem);
        for button in &policy_buttons {
            button.set_sensitive(selected_mode.get() == ExecutionMode::PerItem);
        }
        for button in mode_buttons {
            let row = failure_row.downgrade();
            let mode = selected_mode.clone();
            let policy_buttons = policy_buttons.clone();
            button.connect_toggled(move |_| {
                let per_item = mode.get() == ExecutionMode::PerItem;
                if let Some(row) = row.upgrade() {
                    row.set_sensitive(per_item);
                }
                for button in &policy_buttons {
                    button.set_sensitive(per_item);
                }
            });
        }
        behavior.append(&failure_row);
        behavior.append(&option_row(
            "Working folder",
            "Where the process starts",
            &working_directory,
        ));
        behavior.append(&option_row(
            "Placement",
            "Where the action appears",
            &placement,
        ));
        behavior.append(&option_row(
            "Applies to",
            "Which selected entries offer this action",
            &kind_row,
        ));
        behavior.append(&option_row(
            "Extensions",
            "Comma separated, without dots",
            &extensions,
        ));
        behavior.append(&option_row(
            "Maximum items",
            "Optional selection limit",
            &max_items,
        ));
        behavior.append(&confirm_row);

        let sync_runtime: Rc<dyn Fn()> = Rc::new({
            let selected = selected_runtime.clone();
            let entrypoint = entrypoint.clone();
            let view = script_view.clone();
            move || {
                let runtime = selected.get();
                let is_script = runtime != ActionRuntime::Command;
                entrypoint_field.set_visible(is_script);
                script_field.set_visible(is_script);
                program_field.set_visible(!is_script);
                arguments_field.set_visible(!is_script);
                if is_script {
                    view.set_buffer(Some(match runtime {
                        ActionRuntime::Bash => &bash,
                        _ => &python,
                    }));
                    let current = entrypoint.text();
                    if current.trim().is_empty()
                        || default_entrypoint_for_any(current.trim()).is_some()
                    {
                        entrypoint.set_text(default_entrypoint(runtime));
                    }
                }
            }
        });
        sync_runtime();
        for button in runtime_buttons {
            let sync_runtime = sync_runtime.clone();
            button.connect_toggled(move |button| {
                if button.is_active() {
                    sync_runtime();
                }
            });
        }

        Self {
            mode,
            root,
            error,
            tabs,
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
            selected_icon,
            selected_runtime,
            selected_mode,
            selected_policy,
            selected_directory,
            selected_placement,
        }
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

    fn invalid_field(
        &self,
        tab: u32,
        field: &impl IsA<gtk::Widget>,
        message: impl Into<String>,
    ) -> String {
        self.tabs.set_current_page(Some(tab));
        field.grab_focus();
        message.into()
    }

    /// Builds the definition and script the store will write, or explains why it
    /// cannot.
    fn read(&self) -> Result<(ActionDefinition, Option<ActionScript>), String> {
        let name = self.name.text().trim().to_owned();
        if name.is_empty() {
            return Err(self.invalid_field(GENERAL_TAB, &self.name, "Enter a name"));
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
            return Err(self.invalid_field(
                BEHAVIOR_TAB,
                &self.extensions,
                "That is too many extensions",
            ));
        }
        let arguments = text_contents(&self.arguments);
        if self.selected_runtime.get() == ActionRuntime::Command
            && arguments.chars().count() > MAX_ARGUMENTS_FIELD_CHARS
        {
            return Err(self.invalid_field(
                SCRIPT_TAB,
                &self.arguments,
                "That is too many arguments",
            ));
        }
        let mut kinds = Vec::new();
        if self.files.is_active() {
            kinds.push(InputKind::File);
        }
        if self.folders.is_active() {
            kinds.push(InputKind::Folder);
        }
        if kinds.is_empty() {
            return Err(self.invalid_field(
                BEHAVIOR_TAB,
                &self.files,
                "Choose Files, Folders, or both",
            ));
        }
        let max_items = match self.max_items.text().trim() {
            "" => None,
            value => Some(value.parse::<usize>().map_err(|_| {
                self.invalid_field(
                    BEHAVIOR_TAB,
                    &self.max_items,
                    "The maximum item count must be a number",
                )
            })?),
        };
        let runtime = self.selected_runtime.get();
        let is_script = runtime != ActionRuntime::Command;
        // Report the empty field the user can see, rather than the manifest
        // vocabulary the model would use for a hand-written file.
        if !is_script && self.program.text().trim().is_empty() {
            return Err(self.invalid_field(SCRIPT_TAB, &self.program, "Enter the program to run"));
        }
        if is_script && self.entrypoint.text().trim().is_empty() {
            return Err(self.invalid_field(
                SCRIPT_TAB,
                &self.entrypoint,
                "Enter a script file name",
            ));
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
                args: if is_script {
                    Vec::new()
                } else {
                    arguments
                        .lines()
                        .map(|line| line.trim().to_owned())
                        .filter(|line| !line.is_empty())
                        .collect()
                },
                mode: self.selected_mode.get(),
                on_error: self.selected_policy.get(),
                working_directory: self.selected_directory.get(),
                confirm: self.confirm.is_active(),
            },
        };
        // Validate here so the dialog reports a precise problem before writing.
        definition.validate().map_err(|error| {
            let (tab, field): (_, &gtk::Widget) = match &error {
                ActionError::InvalidId(_) => (GENERAL_TAB, self.id.upcast_ref()),
                ActionError::InvalidName => (GENERAL_TAB, self.name.upcast_ref()),
                ActionError::InvalidDescription => (GENERAL_TAB, self.description.upcast_ref()),
                ActionError::InvalidExtension(_) | ActionError::TooManyConditions(_) => {
                    (BEHAVIOR_TAB, self.extensions.upcast_ref())
                }
                ActionError::InvalidItemRange => (BEHAVIOR_TAB, self.max_items.upcast_ref()),
                ActionError::MissingEntrypoint | ActionError::InvalidEntrypoint(_) => {
                    (SCRIPT_TAB, self.entrypoint.upcast_ref())
                }
                ActionError::MissingProgram | ActionError::InvalidProgram(_) => {
                    (SCRIPT_TAB, self.program.upcast_ref())
                }
                _ => (SCRIPT_TAB, self.arguments.upcast_ref()),
            };
            self.invalid_field(tab, field, error.to_string())
        })?;
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

fn editor_page(tabs: &gtk::Notebook, title: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 22);
    page.add_css_class("settings-action-page");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&page)
        .build();
    scroll.add_css_class("settings-content-scroll");
    let label = gtk::Label::new(Some(title));
    tabs.append_page(&scroll, Some(&label));
    page
}

fn label_control(control: &impl IsA<gtk::Widget>, title: &str, description: &str) {
    control.as_ref().update_property(&[
        gtk::accessible::Property::Label(title),
        gtk::accessible::Property::Description(description),
    ]);
}

fn field(title: &str, description: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
    row.add_css_class("settings-action-field");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.add_css_class("settings-option-title");
    heading.append(&label);
    let hint = gtk::Label::new(Some(description));
    hint.set_xalign(1.0);
    hint.add_css_class("settings-option-description");
    heading.append(&hint);
    row.append(&heading);
    row.append(control);
    label_control(control, title, description);
    row
}

fn option_row(title: &str, description: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let (row, toggle) = super::settings_option(title, description, false);
    row.remove(&toggle);
    row.add_css_class("settings-action-option");
    control.set_valign(gtk::Align::Center);
    control.set_halign(gtk::Align::End);
    label_control(control, title, description);
    row.append(control);
    row
}

/// Bind the shared control before callers connect dependent state updates.
fn segmented<T: Copy + PartialEq + 'static>(
    selected: &Rc<std::cell::Cell<T>>,
    choices: &[(&str, T)],
) -> (gtk::Box, Vec<gtk::ToggleButton>) {
    let labels: Vec<_> = choices.iter().map(|(label, _)| *label).collect();
    let active = choices
        .iter()
        .position(|(_, value)| *value == selected.get())
        .unwrap_or(0);
    let (control, buttons) = segmented_control(&labels, active);
    control.set_homogeneous(false);
    control.set_hexpand(false);
    for (button, (_, value)) in buttons.iter().zip(choices) {
        let selected = selected.clone();
        let value = *value;
        button.connect_toggled(move |button| {
            if button.is_active() {
                selected.set(value);
            }
        });
    }
    (control, buttons)
}

fn icon_chooser(selected: &Rc<std::cell::RefCell<Option<String>>>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let quick = [
        "play",
        "terminal",
        "file-code",
        "scissors",
        "copy",
        "file-archive",
        "image",
        "refresh",
    ];
    let more = gtk::MenuButton::new();
    more.set_child(Some(&picker_icon(crate::assets::icons::PLUS)));
    more.add_css_class("settings-action-more-icons");
    more.set_tooltip_text(Some("More icons"));
    label_control(&more, "More icons", "Choose another bundled Lucide icon");
    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(6)
        .build();
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .child(&grid)
        .build();
    popover.add_css_class("column-popover");
    more.set_popover(Some(&popover));
    let mut first: Option<gtk::ToggleButton> = None;
    let current = selected.borrow().clone();
    let current = current
        .as_deref()
        .filter(|slug| is_known_action_icon(slug))
        .unwrap_or("play");
    let mut remaining = 0;
    for slug in quick.iter().copied().chain(
        ACTION_ICON_CHOICES
            .iter()
            .map(|(slug, _)| *slug)
            .filter(|slug| !quick.contains(slug)),
    ) {
        let button = gtk::ToggleButton::new();
        button.add_css_class("settings-action-icon-choice");
        button.set_child(Some(&picker_icon(action_icon(Some(slug)))));
        button.set_tooltip_text(Some(slug));
        label_control(&button, &format!("{slug} icon"), "");
        if let Some(first) = first.as_ref() {
            button.set_group(Some(first));
        } else {
            first = Some(button.clone());
        }
        button.set_active(current == slug);
        let is_more = !quick.contains(&slug);
        if is_more {
            grid.attach(&button, remaining % 4, remaining / 4, 1, 1);
            remaining += 1;
        } else {
            row.append(&button);
        }
        let selected = selected.clone();
        let weak_more = more.downgrade();
        let weak_popover = popover.downgrade();
        button.connect_toggled(move |button| {
            if button.is_active() {
                selected.replace(Some(slug.to_owned()));
                if let Some(more) = weak_more.upgrade() {
                    more.set_child(Some(&picker_icon(if is_more {
                        action_icon(Some(slug))
                    } else {
                        crate::assets::icons::PLUS
                    })));
                    more.set_tooltip_text(Some(if is_more { slug } else { "More icons" }));
                    if is_more {
                        more.add_css_class("selected");
                    } else {
                        more.remove_css_class("selected");
                    }
                }
                if let Some(popover) = weak_popover.upgrade() {
                    popover.popdown();
                }
            }
        });
    }
    if !quick.contains(&current) {
        more.set_child(Some(&picker_icon(action_icon(Some(current)))));
        more.add_css_class("selected");
        more.set_tooltip_text(Some(current));
    }
    row.append(&more);
    row
}

fn picker_icon(name: &str) -> gtk::Image {
    let icon = crate::assets::primary_icon(name, 18);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon
}

fn script_buffer(language: &str, contents: &str) -> sourceview5::Buffer {
    let buffer = sourceview5::Buffer::new(None);
    crate::ui::theme::register_source_buffer(&buffer);
    buffer.set_language(
        sourceview5::LanguageManager::default()
            .language(language)
            .as_ref(),
    );
    buffer.set_text(contents);
    buffer
}

fn editor_scroll(view: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::builder()
        .child(view)
        .overflow(gtk::Overflow::Hidden)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scroll.add_css_class("settings-action-script");
    scroll
}

fn text_contents(view: &impl IsA<gtk::TextView>) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn bash_template() -> &'static str {
    "#!/usr/bin/env bash\nwhile IFS= read -r -d '' path; do\n    printf 'Processing %s\\n' \"$path\"\ndone < \"$STRATA_ACTION_PATHS\"\n"
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

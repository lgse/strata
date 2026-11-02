// SPDX-License-Identifier: MIT

//! The action editor form: its tabs and fields, filled from an action and read
//! back into a validated definition and script.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::prelude::*;
use sourceview5::prelude::*;

use super::{
    BEHAVIOR_TAB, GENERAL_TAB, MAX_ARGUMENTS_FIELD_CHARS, MAX_EXTENSION_FIELD_CHARS, SCRIPT_TAB,
    examples, icon_chooser::icon_chooser, readiness,
};
use crate::model::action::examples::{ACTION_EXAMPLES, ActionExample};
use crate::model::{
    ACTION_SCHEMA_VERSION, ActionConditions, ActionDefinition, ActionError, ActionRuntime,
    ErrorPolicy, ExecutionMode, InputKind, MenuPlacement, RunSpec, WorkingDirectory, suggest_id,
};
use crate::services::{ActionHandle, ActionScript};
use crate::ui::{
    actions::is_known_action_icon,
    controls::{form_check_button, form_entry, form_error_label, segmented_control},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EditorMode {
    Create,
    Edit,
}

#[derive(Clone)]
pub(super) struct EditorForm {
    pub(super) mode: EditorMode,
    pub(super) original_conditions: ActionConditions,
    pub(super) original_arguments: Vec<String>,
    pub(super) root: gtk::Box,
    pub(super) error: gtk::Label,
    pub(super) tabs: gtk::Notebook,
    pub(super) name: gtk::Entry,
    pub(super) id: gtk::Entry,
    pub(super) description: gtk::Entry,
    pub(super) entrypoint: gtk::Entry,
    pub(super) script: sourceview5::View,
    pub(super) examples: Vec<gtk::MenuButton>,
    pub(super) python_buffer: sourceview5::Buffer,
    pub(super) bash_buffer: sourceview5::Buffer,
    pub(super) runtime_choices: Vec<gtk::ToggleButton>,
    pub(super) mode_choices: Vec<gtk::ToggleButton>,
    pub(super) last_example: Rc<std::cell::Cell<Option<&'static ActionExample>>>,
    pub(super) program: gtk::Entry,
    pub(super) arguments: gtk::TextView,
    pub(super) confirm: gtk::Switch,
    pub(super) enabled: gtk::Switch,
    pub(super) files: gtk::CheckButton,
    pub(super) folders: gtk::CheckButton,
    pub(super) extensions: gtk::Entry,
    pub(super) max_items: gtk::Entry,
    pub(super) selected_icon: Rc<std::cell::RefCell<Option<String>>>,
    pub(super) selected_runtime: Rc<std::cell::Cell<ActionRuntime>>,
    pub(super) selected_mode: Rc<std::cell::Cell<ExecutionMode>>,
    pub(super) selected_policy: Rc<std::cell::Cell<ErrorPolicy>>,
    pub(super) selected_directory: Rc<std::cell::Cell<WorkingDirectory>>,
    pub(super) selected_placement: Rc<std::cell::Cell<MenuPlacement>>,
}

/// The dialog's frame: the error line and a notebook with one page per tab.
struct EditorShell {
    root: gtk::Box,
    error: gtk::Label,
    tabs: gtk::Notebook,
    general: gtk::Box,
    script: gtk::Box,
    behavior: gtk::Box,
}

impl EditorShell {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("settings-action-editor");
        let error = form_error_label();
        error.set_wrap(true);
        crate::ui::controls::keep_words_whole(&error);
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
        let script = editor_page(&tabs, "Script");
        let behavior = editor_page(&tabs, "Behavior");
        behavior.set_spacing(0);
        Self {
            root,
            error,
            tabs,
            general,
            script,
            behavior,
        }
    }
}

/// The General tab's name, id, description and icon.
struct IdentityControls {
    name: gtk::Entry,
    id: gtk::Entry,
    description: gtk::Entry,
    selected_icon: Rc<RefCell<Option<String>>>,
    icon: gtk::Box,
}

impl IdentityControls {
    fn new(mode: EditorMode, definition: &ActionDefinition) -> Self {
        let name = form_entry();
        name.set_text(&definition.name);
        name.set_placeholder_text(Some(&crate::i18n::tr("Batch rename")));
        let id = form_entry();
        id.set_text(&definition.id);
        id.set_placeholder_text(Some("batch-rename"));
        if mode == EditorMode::Create {
            suggest_id_from_name(&name, &id);
        }
        // Changing the id would create another directory, not rename the action.
        id.set_editable(mode == EditorMode::Create);
        id.set_sensitive(mode == EditorMode::Create);
        let description = form_entry();
        description.set_text(definition.description.as_deref().unwrap_or(""));
        description.set_placeholder_text(Some(&crate::i18n::tr(
            "Describes this action to assistive technology",
        )));
        let selected_icon = Rc::new(RefCell::new(definition.icon.clone()));
        let icon = icon_chooser(&selected_icon);
        Self {
            name,
            id,
            description,
            selected_icon,
            icon,
        }
    }
}

fn suggest_id_from_name(name: &gtk::Entry, id: &gtk::Entry) {
    let weak_id = id.downgrade();
    name.connect_changed(move |name| {
        if let Some(id) = weak_id.upgrade() {
            let suggestion = suggest_id(name.text().trim());
            id.set_placeholder_text(Some(&suggestion));
        }
    });
}

/// The Script tab's runtime choice, script buffers and command line.
struct LaunchControls {
    selected_runtime: Rc<Cell<ActionRuntime>>,
    runtime: gtk::Box,
    runtime_buttons: Vec<gtk::ToggleButton>,
    entrypoint: gtk::Entry,
    python: sourceview5::Buffer,
    bash: sourceview5::Buffer,
    script_view: sourceview5::View,
    script_scroll: gtk::ScrolledWindow,
    program: gtk::Entry,
    arguments: gtk::TextView,
    arguments_scroll: gtk::ScrolledWindow,
}

impl LaunchControls {
    fn new(definition: &ActionDefinition, script: Option<&ActionScript>) -> Self {
        let selected_runtime = Rc::new(Cell::new(definition.run.runtime));
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
        if let Some(script) = script {
            match definition.run.runtime {
                ActionRuntime::Bash => bash.set_text(&script.contents),
                _ => python.set_text(&script.contents),
            }
        }
        let script_view = script_view();
        let script_scroll = editor_scroll(&script_view);
        script_scroll.set_min_content_height(240);
        script_scroll.set_vexpand(true);
        let (program, arguments, arguments_scroll) = command_controls(definition);
        Self {
            selected_runtime,
            runtime,
            runtime_buttons,
            entrypoint,
            python,
            bash,
            script_view,
            script_scroll,
            program,
            arguments,
            arguments_scroll,
        }
    }
}

fn script_view() -> sourceview5::View {
    sourceview5::View::builder()
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
        .build()
}

fn command_controls(
    definition: &ActionDefinition,
) -> (gtk::Entry, gtk::TextView, gtk::ScrolledWindow) {
    let program = form_entry();
    program.set_text(definition.run.program.as_deref().unwrap_or(""));
    program.set_placeholder_text(Some(&crate::i18n::tr(
        "Installed program, for example make",
    )));
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
    (program, arguments, arguments_scroll)
}

/// The Behavior tab's run choices.
struct RunChoices {
    selected_mode: Rc<Cell<ExecutionMode>>,
    mode_control: gtk::Box,
    mode_buttons: Vec<gtk::ToggleButton>,
    selected_policy: Rc<Cell<ErrorPolicy>>,
    on_error: gtk::Box,
    policy_buttons: Vec<gtk::ToggleButton>,
    selected_directory: Rc<Cell<WorkingDirectory>>,
    working_directory: gtk::Box,
    selected_placement: Rc<Cell<MenuPlacement>>,
    placement: gtk::Box,
}

impl RunChoices {
    fn new(definition: &ActionDefinition) -> Self {
        let selected_mode = Rc::new(Cell::new(definition.run.mode));
        let (mode_control, mode_buttons) = segmented(
            &selected_mode,
            &[
                ("Whole selection", ExecutionMode::WholeSelection),
                ("Per item", ExecutionMode::PerItem),
            ],
        );
        let selected_policy = Rc::new(Cell::new(definition.run.on_error));
        let (on_error, policy_buttons) = segmented(
            &selected_policy,
            &[
                ("Continue", ErrorPolicy::Continue),
                ("Stop", ErrorPolicy::Stop),
            ],
        );
        let selected_directory = Rc::new(Cell::new(definition.run.working_directory));
        let (working_directory, _) = segmented(
            &selected_directory,
            &[
                ("Invoking", WorkingDirectory::Parent),
                ("Home", WorkingDirectory::Home),
                ("Action", WorkingDirectory::Action),
            ],
        );
        let selected_placement = Rc::new(Cell::new(definition.menu));
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
        Self {
            selected_mode,
            mode_control,
            mode_buttons,
            selected_policy,
            on_error,
            policy_buttons,
            selected_directory,
            working_directory,
            selected_placement,
            placement,
        }
    }
}

/// The switches and conditions that decide when the action is offered.
struct Conditions {
    confirm_row: gtk::Box,
    confirm: gtk::Switch,
    enabled_row: gtk::Box,
    enabled: gtk::Switch,
    files: gtk::CheckButton,
    folders: gtk::CheckButton,
    extensions: gtk::Entry,
    max_items: gtk::Entry,
}

impl Conditions {
    fn new(definition: &ActionDefinition) -> Self {
        let (confirm_row, confirm) = super::super::settings_option(
            "Confirm",
            "Ask before running this action",
            definition.run.confirm,
        );
        confirm_row.add_css_class("settings-action-option");
        let (enabled_row, enabled) = super::super::settings_option(
            "Enabled",
            "Show this action in menus",
            definition.enabled,
        );
        enabled_row.add_css_class("settings-action-option");
        enabled_row.add_css_class("settings-action-enabled");
        let (files, folders) = kind_choices(&definition.when.kinds);
        let (extensions, max_items) = condition_entries(&definition.when);
        Self {
            confirm_row,
            confirm,
            enabled_row,
            enabled,
            files,
            folders,
            extensions,
            max_items,
        }
    }
}

/// An action with no kinds applies to both.
fn kind_choices(kinds: &[InputKind]) -> (gtk::CheckButton, gtk::CheckButton) {
    let files = form_check_button(&crate::i18n::tr("Files"));
    let folders = form_check_button(&crate::i18n::tr("Folders"));
    if kinds.is_empty() {
        files.set_active(true);
        folders.set_active(true);
    } else {
        files.set_active(kinds.contains(&InputKind::File));
        folders.set_active(kinds.contains(&InputKind::Folder));
    }
    (files, folders)
}

fn condition_entries(when: &ActionConditions) -> (gtk::Entry, gtk::Entry) {
    let extensions = form_entry();
    extensions.set_text(&when.extensions.join(", "));
    extensions.set_placeholder_text(Some(&crate::i18n::tr("Any")));
    extensions.set_width_chars(18);
    let max_items = form_entry();
    max_items.set_text(
        &when
            .max_items
            .map(|value| value.to_string())
            .unwrap_or_default(),
    );
    max_items.set_placeholder_text(Some(&crate::i18n::tr("No limit")));
    max_items.set_width_chars(18);
    max_items.set_input_purpose(gtk::InputPurpose::Digits);
    let entry_widths = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
    entry_widths.add_widget(&extensions);
    entry_widths.add_widget(&max_items);
    (extensions, max_items)
}

/// The Script tab's labelled fields, which the runtime choice shows or hides.
struct LaunchFields {
    entrypoint: gtk::Box,
    script: gtk::Box,
    program: gtk::Box,
    arguments: gtk::Box,
    examples: Vec<gtk::MenuButton>,
}

impl LaunchFields {
    fn new(launch: &LaunchControls) -> Self {
        let entrypoint = field(
            "Script file",
            "Saved beside action.toml",
            &launch.entrypoint,
        );
        let script = field("Script", "", &launch.script_scroll);
        let program = field("Program", "Installed executable", &launch.program);
        let arguments = field(
            "Arguments",
            "One per line · {path}, {paths}, or {parent}",
            &launch.arguments_scroll,
        );
        if let Some(heading) = script.first_child().and_downcast::<gtk::Box>() {
            heading.append(&readiness::indicator(
                launch.selected_runtime.clone(),
                &launch.runtime_buttons,
                &launch.python,
                &launch.bash,
            ));
        }
        let examples = [&script, &arguments]
            .map(|field| {
                let button = examples::button();
                if let Some(heading) = field.first_child().and_downcast::<gtk::Box>() {
                    heading.append(&button);
                }
                button
            })
            .to_vec();
        Self {
            entrypoint,
            script,
            program,
            arguments,
            examples,
        }
    }
}

fn describe_id(id: &gtk::Entry, mode: EditorMode) {
    crate::ui::accessibility::set_description(
        id,
        Some(&crate::i18n::tr(if mode == EditorMode::Create {
            "Folder name: lowercase letters, digits, and dashes. Leave blank to use the name."
        } else {
            "The action id cannot change; duplicate it to create a variant."
        })),
    );
}

fn lay_out_general(page: &gtk::Box, identity: &IdentityControls, enabled_row: &gtk::Box) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    let name_field = field("Name", "Shown in the context menu", &identity.name);
    name_field.set_hexpand(true);
    row.append(&name_field);
    identity.id.set_width_chars(28);
    let id_field = field("Id", "Folder name", &identity.id);
    id_field.set_hexpand(false);
    row.append(&id_field);
    page.append(&row);
    page.append(&field(
        "Description",
        "Optional accessible description",
        &identity.description,
    ));
    page.append(&field("Icon", "Bundled Lucide icon", &identity.icon));
    page.append(enabled_row);
}

fn lay_out_script(page: &gtk::Box, launch: &LaunchControls, fields: &LaunchFields) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    let launch_controls = gtk::SizeGroup::new(gtk::SizeGroupMode::Vertical);
    launch_controls.add_widget(&launch.runtime);
    launch_controls.add_widget(&launch.entrypoint);
    launch_controls.add_widget(&launch.program);
    let runtime_field = field("Runtime", "How it starts", &launch.runtime);
    runtime_field.set_hexpand(false);
    row.append(&runtime_field);
    fields.entrypoint.set_hexpand(true);
    fields.program.set_hexpand(true);
    row.append(&fields.entrypoint);
    row.append(&fields.program);
    page.append(&row);
    fields.script.set_vexpand(true);
    fields.arguments.set_vexpand(true);
    page.append(&fields.script);
    page.append(&fields.arguments);
    label_control(&launch.script_view, "Script", "");
    label_control(
        &launch.arguments,
        "Arguments",
        "One argument per line; {path}, {paths}, or {parent}",
    );
}

fn lay_out_behavior(
    page: &gtk::Box,
    run: &RunChoices,
    conditions: &Conditions,
    kind_row: &gtk::Box,
) {
    page.append(&option_row(
        "Run",
        "Once, or once per selected item",
        &run.mode_control,
    ));
    page.append(&failure_row(run));
    page.append(&option_row(
        "Working folder",
        "Where the process starts",
        &run.working_directory,
    ));
    page.append(&option_row(
        "Placement",
        "Where the action appears",
        &run.placement,
    ));
    page.append(&option_row(
        "Applies to",
        "Which selected entries offer this action",
        kind_row,
    ));
    page.append(&option_row(
        "Extensions",
        "Comma separated, without dots",
        &conditions.extensions,
    ));
    page.append(&option_row(
        "Maximum items",
        "Optional selection limit",
        &conditions.max_items,
    ));
    page.append(&conditions.confirm_row);
}

/// The failure policy applies only per item, so it follows the run mode.
fn failure_row(run: &RunChoices) -> gtk::Box {
    let row = option_row(
        "On failure",
        "Only used when running per item",
        &run.on_error,
    );
    row.set_sensitive(run.selected_mode.get() == ExecutionMode::PerItem);
    for button in &run.policy_buttons {
        button.set_sensitive(run.selected_mode.get() == ExecutionMode::PerItem);
    }
    for button in &run.mode_buttons {
        let row = row.downgrade();
        let mode = run.selected_mode.clone();
        let policy_buttons = run.policy_buttons.clone();
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
    row
}

/// Shows the fields the selected runtime uses and its script, and swaps a
/// default script file name for the runtime's own.
fn follow_runtime(launch: &LaunchControls, fields: &LaunchFields) {
    let view = Rc::new(RuntimeView {
        selected: launch.selected_runtime.clone(),
        python: launch.python.clone(),
        bash: launch.bash.clone(),
        entrypoint: launch.entrypoint.clone(),
        script_view: launch.script_view.clone(),
        entrypoint_field: fields.entrypoint.clone(),
        script_field: fields.script.clone(),
        program_field: fields.program.clone(),
        arguments_field: fields.arguments.clone(),
    });
    view.sync();
    for button in &launch.runtime_buttons {
        let view = view.clone();
        button.connect_toggled(move |button| {
            if button.is_active() {
                view.sync();
            }
        });
    }
}

/// What the runtime choice shows, hides or rewrites.
struct RuntimeView {
    selected: Rc<Cell<ActionRuntime>>,
    python: sourceview5::Buffer,
    bash: sourceview5::Buffer,
    entrypoint: gtk::Entry,
    script_view: sourceview5::View,
    entrypoint_field: gtk::Box,
    script_field: gtk::Box,
    program_field: gtk::Box,
    arguments_field: gtk::Box,
}

impl RuntimeView {
    fn sync(&self) {
        let runtime = self.selected.get();
        let is_script = runtime != ActionRuntime::Command;
        self.entrypoint_field.set_visible(is_script);
        self.script_field.set_visible(is_script);
        self.program_field.set_visible(!is_script);
        self.arguments_field.set_visible(!is_script);
        if !is_script {
            return;
        }
        self.script_view.set_buffer(Some(match runtime {
            ActionRuntime::Bash => &self.bash,
            _ => &self.python,
        }));
        replace_default_entrypoint(&self.entrypoint, runtime);
    }
}
fn replace_default_entrypoint(entrypoint: &gtk::Entry, runtime: ActionRuntime) {
    let current = entrypoint.text();
    let default_name =
        current.trim().is_empty() || default_entrypoint_for_any(current.trim()).is_some();
    if default_name {
        entrypoint.set_text(default_entrypoint(runtime));
    }
}

impl EditorForm {
    pub(super) fn new(
        mode: EditorMode,
        action: &ActionHandle,
        script: Option<ActionScript>,
    ) -> Self {
        let definition = &action.definition;
        let shell = EditorShell::new();
        let identity = IdentityControls::new(mode, definition);
        let launch = LaunchControls::new(definition, script.as_ref());
        let run = RunChoices::new(definition);
        let conditions = Conditions::new(definition);
        describe_id(&identity.id, mode);
        let kind_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        kind_row.append(&conditions.files);
        kind_row.append(&conditions.folders);
        let fields = LaunchFields::new(&launch);
        lay_out_general(&shell.general, &identity, &conditions.enabled_row);
        lay_out_script(&shell.script, &launch, &fields);
        lay_out_behavior(&shell.behavior, &run, &conditions, &kind_row);
        follow_runtime(&launch, &fields);
        Self {
            mode,
            original_conditions: definition.when.clone(),
            original_arguments: definition.run.args.clone(),
            root: shell.root,
            error: shell.error,
            tabs: shell.tabs,
            name: identity.name,
            id: identity.id,
            description: identity.description,
            entrypoint: launch.entrypoint,
            script: launch.script_view,
            examples: fields.examples,
            python_buffer: launch.python,
            bash_buffer: launch.bash,
            runtime_choices: launch.runtime_buttons,
            mode_choices: run.mode_buttons,
            last_example: Rc::new(Cell::new(None)),
            program: launch.program,
            arguments: launch.arguments,
            confirm: conditions.confirm,
            enabled: conditions.enabled,
            files: conditions.files,
            folders: conditions.folders,
            extensions: conditions.extensions,
            max_items: conditions.max_items,
            selected_icon: identity.selected_icon,
            selected_runtime: launch.selected_runtime,
            selected_mode: run.selected_mode,
            selected_policy: run.selected_policy,
            selected_directory: run.selected_directory,
            selected_placement: run.selected_placement,
        }
    }

    pub(super) fn recipe_buffer(&self, runtime: ActionRuntime) -> &sourceview5::Buffer {
        match runtime {
            ActionRuntime::Bash => &self.bash_buffer,
            _ => &self.python_buffer,
        }
    }

    pub(super) fn replaces_draft(&self, runtime: ActionRuntime) -> bool {
        let buffer = self.recipe_buffer(runtime);
        let contents = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let starter = match runtime {
            ActionRuntime::Bash => bash_template().to_owned(),
            _ => python_template(),
        };
        !contents.trim().is_empty() && contents.as_str() != starter
    }

    pub(super) fn apply_example(&self, example: &'static ActionExample) {
        let previous = self.last_example.get();
        let tr = crate::i18n::tr;
        if self.name.text().is_empty()
            || previous.is_some_and(|previous| self.name.text() == tr(previous.name))
        {
            self.name.set_text(&tr(example.name));
        }
        if self.description.text().is_empty()
            || previous.is_some_and(|previous| self.description.text() == tr(previous.description))
        {
            self.description.set_text(&tr(example.description));
        }
        let runtime = example.runtime();
        let changed_runtime = self.selected_runtime.get() != runtime;
        self.runtime_choices[usize::from(runtime == ActionRuntime::Bash)].set_active(true);
        if changed_runtime {
            self.entrypoint.set_text(default_entrypoint(runtime));
        }
        let buffer = self.recipe_buffer(runtime);
        buffer.begin_user_action();
        buffer.delete(&mut buffer.start_iter(), &mut buffer.end_iter());
        buffer.insert(&mut buffer.start_iter(), &example.script());
        buffer.end_user_action();
        self.mode_choices[usize::from(example.mode == ExecutionMode::PerItem)].set_active(true);
        self.files.set_active(true);
        self.folders.set_active(example.folders);
        self.extensions.set_text(&example.extensions.join(", "));
        self.last_example.set(Some(example));
        self.error.set_visible(false);
        self.tabs.set_current_page(Some(SCRIPT_TAB));
        self.script.grab_focus();
    }

    pub(super) fn focus_first(&self) {
        self.name.grab_focus_without_selecting();
        self.name.set_position(-1);
    }

    pub(super) fn show_error(&self, message: &str) {
        self.error.set_text(message);
        self.error.set_visible(true);
    }

    pub(super) fn invalid_field(
        &self,
        tab: u32,
        field: &impl IsA<gtk::Widget>,
        message: impl Into<String>,
    ) -> String {
        self.tabs.set_current_page(Some(tab));
        field.grab_focus();
        message.into()
    }

    pub(super) fn read(&self) -> Result<(ActionDefinition, Option<ActionScript>), String> {
        let name = self.read_name()?;
        let id = self.read_id(&name);
        self.check_extensions_length()?;
        let arguments = text_contents(&self.arguments);
        self.check_arguments_length(&arguments)?;
        let kinds = self.read_kinds()?;
        let max_items = self.read_max_items()?;
        let runtime = self.selected_runtime.get();
        let is_script = runtime != ActionRuntime::Command;
        self.check_launch(is_script)?;
        let definition = ActionDefinition {
            schema_version: ACTION_SCHEMA_VERSION,
            id,
            name,
            description: self.read_description(),
            icon: self
                .selected_icon
                .borrow()
                .clone()
                .filter(|icon| is_known_action_icon(icon)),
            enabled: self.enabled.is_active(),
            menu: self.selected_placement.get(),
            when: ActionConditions {
                kinds,
                extensions: self.read_extensions(),
                mime_types: self.original_conditions.mime_types.clone(),
                min_items: self.original_conditions.min_items,
                max_items,
            },
            run: self.read_run(runtime, &arguments),
        };
        definition
            .validate()
            .map_err(|error| self.validation_error(&error))?;
        let script = self.read_script(&definition);
        Ok((definition, script))
    }

    fn read_name(&self) -> Result<String, String> {
        let name = self.name.text().trim().to_owned();
        if name.is_empty() {
            return Err(self.invalid_field(
                GENERAL_TAB,
                &self.name,
                crate::i18n::tr("Enter a name"),
            ));
        }
        Ok(name)
    }

    /// A new action left without an id takes one from its name.
    fn read_id(&self, name: &str) -> String {
        let typed = self.id.text().trim().to_owned();
        if self.mode == EditorMode::Create && typed.is_empty() {
            suggest_id(name)
        } else {
            typed
        }
    }

    fn check_extensions_length(&self) -> Result<(), String> {
        if self.extensions.text().chars().count() > MAX_EXTENSION_FIELD_CHARS {
            return Err(self.invalid_field(
                BEHAVIOR_TAB,
                &self.extensions,
                crate::i18n::tr("That is too many extensions"),
            ));
        }
        Ok(())
    }

    fn check_arguments_length(&self, arguments: &str) -> Result<(), String> {
        let command = self.selected_runtime.get() == ActionRuntime::Command;
        if command && arguments.chars().count() > MAX_ARGUMENTS_FIELD_CHARS {
            return Err(self.invalid_field(
                SCRIPT_TAB,
                &self.arguments,
                crate::i18n::tr("That is too many arguments"),
            ));
        }
        Ok(())
    }

    fn read_kinds(&self) -> Result<Vec<InputKind>, String> {
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
                crate::i18n::tr("Choose Files, Folders, or both"),
            ));
        }
        Ok(kinds)
    }

    fn read_max_items(&self) -> Result<Option<usize>, String> {
        match self.max_items.text().trim() {
            "" => Ok(None),
            value => value.parse::<usize>().map(Some).map_err(|_| {
                self.invalid_field(
                    BEHAVIOR_TAB,
                    &self.max_items,
                    crate::i18n::tr("The maximum item count must be a number"),
                )
            }),
        }
    }

    /// A command needs its program; a script needs its file name.
    fn check_launch(&self, is_script: bool) -> Result<(), String> {
        if !is_script && self.program.text().trim().is_empty() {
            return Err(self.invalid_field(
                SCRIPT_TAB,
                &self.program,
                crate::i18n::tr("Enter the program to run"),
            ));
        }
        if is_script && self.entrypoint.text().trim().is_empty() {
            return Err(self.invalid_field(
                SCRIPT_TAB,
                &self.entrypoint,
                crate::i18n::tr("Enter a script file name"),
            ));
        }
        Ok(())
    }

    fn read_description(&self) -> Option<String> {
        let description = self.description.text().trim().to_owned();
        (!description.is_empty()).then_some(description)
    }

    fn read_extensions(&self) -> Vec<String> {
        self.extensions
            .text()
            .split(',')
            .map(|extension| {
                extension
                    .trim()
                    .trim_start_matches('.')
                    .to_ascii_lowercase()
            })
            .filter(|extension| !extension.is_empty())
            .collect()
    }

    fn read_run(&self, runtime: ActionRuntime, arguments: &str) -> RunSpec {
        let is_script = runtime != ActionRuntime::Command;
        RunSpec {
            runtime,
            entrypoint: is_script.then(|| self.entrypoint.text().trim().to_owned()),
            program: (!is_script).then(|| self.program.text().trim().to_owned()),
            args: if is_script {
                Vec::new()
            } else {
                self.read_arguments(arguments)
            },
            mode: self.selected_mode.get(),
            on_error: self.selected_policy.get(),
            working_directory: self.selected_directory.get(),
            confirm: self.confirm.is_active(),
        }
    }

    /// Unedited arguments keep their saved form.
    fn read_arguments(&self, arguments: &str) -> Vec<String> {
        if arguments == self.original_arguments.join("\n") {
            self.original_arguments.clone()
        } else if arguments.is_empty() {
            Vec::new()
        } else {
            arguments.split('\n').map(str::to_owned).collect()
        }
    }

    /// Shows the tab and field a validation error is about.
    fn validation_error(&self, error: &ActionError) -> String {
        let (tab, field): (_, &gtk::Widget) = match error {
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
        self.invalid_field(tab, field, error.user_message())
    }

    fn read_script(&self, definition: &ActionDefinition) -> Option<ActionScript> {
        let entrypoint = definition.run.script_entrypoint()?;
        let buffer = self.script.buffer();
        let contents = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        Some(ActionScript {
            file_name: entrypoint.to_owned(),
            contents,
        })
    }
}

pub(super) fn default_entrypoint(runtime: ActionRuntime) -> &'static str {
    match runtime {
        ActionRuntime::Python => "main.py",
        _ => "run.sh",
    }
}

pub(super) fn default_entrypoint_for_any(name: &str) -> Option<ActionRuntime> {
    match name {
        "main.py" => Some(ActionRuntime::Python),
        "run.sh" => Some(ActionRuntime::Bash),
        _ => None,
    }
}

pub(super) fn editor_page(tabs: &gtk::Notebook, title: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 22);
    page.add_css_class("settings-action-page");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&page)
        .build();
    scroll.add_css_class("settings-content-scroll");
    let label = gtk::Label::new(Some(&crate::i18n::tr(title)));
    label.add_css_class("settings-action-tab-label");
    tabs.append_page(&scroll, Some(&label));
    page
}

/// Takes English source text and translates it.
pub(super) fn label_control(control: &impl IsA<gtk::Widget>, title: &str, description: &str) {
    crate::ui::accessibility::set_own_label(control, &crate::i18n::tr(title));
    crate::ui::accessibility::set_description(
        control.upcast_ref::<gtk::Widget>(),
        Some(&translate_optional(description)),
    );
}

pub(super) fn translate_optional(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        crate::i18n::tr(text)
    }
}

pub(super) fn field(title: &str, description: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
    row.add_css_class("settings-action-field");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(&crate::i18n::tr(title)));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.add_css_class("settings-option-title");
    heading.append(&label);
    if !description.is_empty() {
        let hint = gtk::Label::new(Some(&crate::i18n::tr(description)));
        hint.set_xalign(1.0);
        hint.add_css_class("settings-option-description");
        heading.append(&hint);
    }
    row.append(&heading);
    row.append(control);
    label_control(control, title, description);
    row
}

pub(super) fn option_row(
    title: &str,
    description: &str,
    control: &impl IsA<gtk::Widget>,
) -> gtk::Box {
    let (row, toggle) = super::super::settings_option(title, description, false);
    row.remove(&toggle);
    row.add_css_class("settings-action-option");
    control.set_valign(gtk::Align::Center);
    control.set_halign(gtk::Align::End);
    label_control(control, title, description);
    row.append(control);
    row
}

/// Bind the shared control before callers connect dependent state updates.
pub(super) fn segmented<T: Copy + PartialEq + 'static>(
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

pub(super) fn script_buffer(language: &str, contents: &str) -> sourceview5::Buffer {
    let buffer = crate::ui::theme::new_source_buffer();
    buffer.set_language(
        crate::ui::theme::source_languages()
            .language(language)
            .as_ref(),
    );
    buffer.set_text(contents);
    buffer
}

pub(super) fn editor_scroll(view: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::builder()
        .child(view)
        .overflow(gtk::Overflow::Hidden)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scroll.add_css_class("settings-action-script");
    scroll
}

pub(super) fn text_contents(view: &impl IsA<gtk::TextView>) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

pub(super) fn bash_template() -> &'static str {
    "#!/usr/bin/env bash\nwhile IFS= read -r -d '' path; do\n    printf 'Processing %s\\n' \"$path\"\ndone < \"$STRATA_ACTION_PATHS\"\n"
}

pub(super) fn draft_definition(runtime: ActionRuntime, mode: ExecutionMode) -> ActionDefinition {
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

pub(super) fn python_template() -> String {
    ACTION_EXAMPLES[0].script()
}

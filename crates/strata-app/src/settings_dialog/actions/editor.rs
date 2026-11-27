// SPDX-License-Identifier: MIT

use rune::{TextEdit, Tx};
use strata_components::{
    code_view::{editor::EditorState, highlight::Highlighter},
    controls::tabbed_form::TabbedFormState,
    scroll::Bar,
};
use strata_core::{
    i18n::tr,
    model::{
        ACTION_SCHEMA_VERSION, ActionConditions, ActionDefinition, ActionError, ActionRuntime,
        ErrorPolicy, ExecutionMode, InputKind, MenuPlacement, RunSpec, WorkingDirectory,
        action::{
            MAX_ARGUMENT_CHARS, MAX_ARGUMENTS,
            examples::{ACTION_EXAMPLES, ActionExample},
        },
        suggest_id,
    },
};
use strata_fs::services::{ActionHandle, ActionScript};

use super::readiness::{ReadinessState, RefreshRequest};
use crate::{
    App, custom_actions::is_known_action_icon, settings_dialog::controls::settings_bar,
    slots::WeakLater,
};

pub(super) const GENERAL_TAB: usize = 0;
pub(super) const SCRIPT_TAB: usize = 1;
pub(super) const BEHAVIOR_TAB: usize = 2;

pub(super) const MAX_EXTENSION_FIELD_CHARS: usize = 256;
pub(super) const MAX_ARGUMENTS_FIELD_CHARS: usize = MAX_ARGUMENTS * (MAX_ARGUMENT_CHARS + 1);

pub(super) const EDITOR_NAME_KEY: &str = "settings-action-editor-name";
pub(super) const EDITOR_ID_KEY: &str = "settings-action-editor-id";
pub(super) const EDITOR_DESCRIPTION_KEY: &str = "settings-action-editor-description";
pub(super) const EDITOR_ENTRYPOINT_KEY: &str = "settings-action-editor-entrypoint";
pub(super) const EDITOR_PROGRAM_KEY: &str = "settings-action-editor-program";
pub(super) const EDITOR_ARGUMENTS_KEY: &str = "settings-action-editor-arguments";
pub(super) const EDITOR_FILES_KEY: &str = "settings-action-editor-files";
pub(super) const EDITOR_EXTENSIONS_KEY: &str = "settings-action-editor-extensions";
pub(super) const EDITOR_MAX_ITEMS_KEY: &str = "settings-action-editor-max-items";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EditorMode {
    Create,
    Edit,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ValidationError {
    pub(super) tab: usize,
    pub(super) field_key: &'static str,
    pub(super) message: String,
}

pub(super) struct EditorSession {
    pub(super) mode: EditorMode,
    pub(super) generation: u64,
    pub(super) tabs: TabbedFormState,
    pub(super) original_conditions: ActionConditions,
    pub(super) original_arguments: Vec<String>,
    pub(super) name: TextEdit,
    pub(super) id: TextEdit,
    pub(super) description: TextEdit,
    pub(super) selected_icon: Option<String>,
    pub(super) more_icons_open: bool,
    pub(super) enabled: bool,
    pub(super) runtime: ActionRuntime,
    pub(super) entrypoint: TextEdit,
    pub(super) python_editor: EditorState,
    pub(super) bash_editor: EditorState,
    pub(super) program: TextEdit,
    pub(super) arguments: TextEdit,
    pub(super) mode_choice: ExecutionMode,
    pub(super) policy: ErrorPolicy,
    pub(super) working_directory: WorkingDirectory,
    pub(super) placement: MenuPlacement,
    pub(super) files: bool,
    pub(super) folders: bool,
    pub(super) extensions: TextEdit,
    pub(super) max_items: TextEdit,
    pub(super) confirm: bool,
    pub(super) last_example: Option<&'static ActionExample>,
    pub(super) examples_open: bool,
    pub(super) examples_query: TextEdit,
    pub(super) examples_category: &'static str,
    pub(super) pending_example: Option<&'static ActionExample>,
    pub(super) editor_bars: [Bar; 3],
    pub(super) examples_bar: Bar,
    pub(super) readiness: ReadinessState,
    pub(super) error: Option<String>,
    pub(super) error_tab: Option<usize>,
    pub(super) saving: bool,
}

pub(super) struct EditorDraft<'a> {
    pub(super) mode: EditorMode,
    pub(super) generation: u64,
    pub(super) action: &'a ActionHandle,
    pub(super) script: Option<ActionScript>,
}

impl EditorSession {
    pub(super) fn new(
        draft: EditorDraft<'_>,
        highlighter: &Highlighter,
        later: WeakLater<App>,
    ) -> Self {
        let def = &draft.action.definition;
        let (python_text, bash_text) = initial_scripts(def.run.runtime, draft.script.as_ref());
        let python_editor = EditorState::new(python_text, highlighter, Some("main.py"), Some("py"))
            .key("settings-action-script-python");
        let bash_editor = EditorState::new(bash_text, highlighter, Some("run.sh"), Some("bash"))
            .key("settings-action-script-bash");
        let (files, folders) = initial_kinds(&def.when.kinds);
        let mut session = Self {
            mode: draft.mode,
            generation: draft.generation,
            tabs: TabbedFormState::new(),
            original_conditions: def.when.clone(),
            original_arguments: def.run.args.clone(),
            name: TextEdit::new(&def.name),
            id: TextEdit::new(&def.id),
            description: TextEdit::new(def.description.as_deref().unwrap_or("")),
            selected_icon: def.icon.clone(),
            more_icons_open: false,
            enabled: def.enabled,
            runtime: def.run.runtime,
            entrypoint: TextEdit::new(
                def.run
                    .entrypoint
                    .as_deref()
                    .unwrap_or_else(|| default_entrypoint(def.run.runtime)),
            ),
            python_editor,
            bash_editor,
            program: TextEdit::new(def.run.program.as_deref().unwrap_or("")),
            arguments: TextEdit::new(def.run.args.join("\n")),
            mode_choice: def.run.mode,
            policy: def.run.on_error,
            working_directory: def.run.working_directory,
            placement: def.menu,
            files,
            folders,
            extensions: TextEdit::new(def.when.extensions.join(", ")),
            max_items: TextEdit::new(
                def.when
                    .max_items
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            ),
            confirm: def.run.confirm,
            last_example: None,
            examples_open: false,
            examples_query: TextEdit::default(),
            examples_category: "All",
            pending_example: None,
            editor_bars: std::array::from_fn(|_| settings_bar()),
            examples_bar: settings_bar(),
            readiness: ReadinessState::default(),
            error: None,
            error_tab: None,
            saving: false,
        };
        session.refresh_readiness(later, true);
        session
    }

    pub(super) fn id_placeholder(&self) -> String {
        if self.mode == EditorMode::Create {
            let suggestion = suggest_id(self.name.text.trim());
            if !suggestion.is_empty() {
                return suggestion;
            }
        }
        "batch-rename".to_owned()
    }

    pub(super) fn active_script_editor(&self) -> &EditorState {
        match self.runtime {
            ActionRuntime::Bash => &self.bash_editor,
            _ => &self.python_editor,
        }
    }

    pub(super) fn active_script_editor_mut(&mut self) -> &mut EditorState {
        match self.runtime {
            ActionRuntime::Bash => &mut self.bash_editor,
            _ => &mut self.python_editor,
        }
    }

    pub(super) fn recipe_editor(&self, runtime: ActionRuntime) -> &EditorState {
        match runtime {
            ActionRuntime::Bash => &self.bash_editor,
            _ => &self.python_editor,
        }
    }

    pub(super) fn recipe_editor_mut(&mut self, runtime: ActionRuntime) -> &mut EditorState {
        match runtime {
            ActionRuntime::Bash => &mut self.bash_editor,
            _ => &mut self.python_editor,
        }
    }

    pub(super) fn popover_open(&self) -> bool {
        self.more_icons_open || self.examples_open
    }

    pub(super) fn release_bars(&self) {
        for bar in &self.editor_bars {
            bar.release();
        }
        self.examples_bar.release();
        self.python_editor.release_bars();
        self.bash_editor.release_bars();
    }

    pub(super) fn has_focused_field(&self) -> bool {
        self.name.focused
            || self.id.focused
            || self.description.focused
            || self.entrypoint.focused
            || self.python_editor.edit.focused
            || self.bash_editor.edit.focused
            || self.program.focused
            || self.arguments.focused
            || self.extensions.focused
            || self.max_items.focused
            || self.examples_query.focused
    }

    pub(super) fn poll(&mut self, tx: &mut Tx) -> bool {
        let mut changed = self.tabs.tick(tx);
        changed |= self.poll_highlight();
        changed
    }

    pub(super) fn poll_highlight(&mut self) -> bool {
        let py = self.python_editor.poll();
        let sh = self.bash_editor.poll();
        py | sh
    }

    pub(super) fn set_runtime(&mut self, runtime: ActionRuntime, later: WeakLater<App>) {
        if self.runtime == runtime {
            return;
        }
        self.python_editor.release_bars();
        self.bash_editor.release_bars();
        self.runtime = runtime;
        if runtime != ActionRuntime::Command {
            replace_default_entrypoint(&mut self.entrypoint, runtime);
            self.refresh_readiness(later, false);
        }
    }

    pub(super) fn refresh_readiness(&mut self, later: WeakLater<App>, force: bool) {
        if self.runtime == ActionRuntime::Command {
            return;
        }
        let source = self.active_script_editor().text().to_owned();
        let runtime = self.runtime;
        let generation = self.generation;
        self.readiness.refresh(RefreshRequest {
            runtime,
            source: &source,
            later,
            editor_gen: generation,
            force,
        });
    }

    pub(super) fn replaces_draft(&self, runtime: ActionRuntime) -> bool {
        let contents = self.recipe_editor(runtime).text();
        let starter = match runtime {
            ActionRuntime::Bash => bash_template().to_owned(),
            _ => python_template(),
        };
        !contents.trim().is_empty() && contents != starter
    }

    pub(super) fn apply_example(&mut self, example: &'static ActionExample, later: WeakLater<App>) {
        let previous = self.last_example;
        if self.name.text.is_empty() || previous.is_some_and(|prev| self.name.text == tr(prev.name))
        {
            self.name.replace_text(tr(example.name));
        }
        if self.description.text.is_empty()
            || previous.is_some_and(|prev| self.description.text == tr(prev.description))
        {
            self.description.replace_text(tr(example.description));
        }
        let runtime = example.runtime();
        let changed_runtime = self.runtime != runtime;
        self.runtime = runtime;
        if changed_runtime {
            self.entrypoint.replace_text(default_entrypoint(runtime));
        }
        self.recipe_editor_mut(runtime).set_text(&example.script());
        self.mode_choice = example.mode;
        self.files = true;
        self.folders = example.folders;
        self.extensions.replace_text(example.extensions.join(", "));
        self.last_example = Some(example);
        self.error = None;
        self.error_tab = None;
        self.examples_open = false;
        self.release_bars();
        self.examples_query.focused = false;
        self.pending_example = None;
        self.tabs.select(SCRIPT_TAB);
        self.refresh_readiness(later, false);
    }

    pub(super) fn show_error(&mut self, message: impl Into<String>, tab: Option<usize>) {
        self.error = Some(message.into());
        self.error_tab = tab;
    }

    pub(super) fn read(&self) -> Result<(ActionDefinition, Option<ActionScript>), ValidationError> {
        let name = self.read_name()?;
        let id = self.read_id(&name);
        self.check_extensions_length()?;
        self.check_arguments_length()?;
        let kinds = self.read_kinds()?;
        let max_items = self.read_max_items()?;
        let is_script = self.runtime != ActionRuntime::Command;
        self.check_launch(is_script)?;
        let definition = ActionDefinition {
            schema_version: ACTION_SCHEMA_VERSION,
            id,
            name,
            description: self.read_description(),
            icon: self
                .selected_icon
                .clone()
                .filter(|icon| is_known_action_icon(icon)),
            enabled: self.enabled,
            menu: self.placement,
            when: ActionConditions {
                kinds,
                extensions: self.read_extensions(),
                mime_types: self.original_conditions.mime_types.clone(),
                min_items: self.original_conditions.min_items,
                max_items,
            },
            run: self.read_run(),
        };
        definition
            .validate()
            .map_err(|error| self.validation_error(&error))?;
        let script = self.read_script(&definition);
        Ok((definition, script))
    }

    fn read_name(&self) -> Result<String, ValidationError> {
        let name = self.name.text.trim().to_owned();
        if name.is_empty() {
            return Err(ValidationError {
                tab: GENERAL_TAB,
                field_key: EDITOR_NAME_KEY,
                message: tr("Enter a name"),
            });
        }
        Ok(name)
    }

    /// A new action left without an id takes one from its name.
    fn read_id(&self, name: &str) -> String {
        let typed = self.id.text.trim().to_owned();
        if self.mode == EditorMode::Create && typed.is_empty() {
            suggest_id(name)
        } else {
            typed
        }
    }

    fn check_extensions_length(&self) -> Result<(), ValidationError> {
        if self.extensions.text.chars().count() > MAX_EXTENSION_FIELD_CHARS {
            return Err(ValidationError {
                tab: BEHAVIOR_TAB,
                field_key: EDITOR_EXTENSIONS_KEY,
                message: tr("That is too many extensions"),
            });
        }
        Ok(())
    }

    fn check_arguments_length(&self) -> Result<(), ValidationError> {
        let command = self.runtime == ActionRuntime::Command;
        if command && self.arguments.text.chars().count() > MAX_ARGUMENTS_FIELD_CHARS {
            return Err(ValidationError {
                tab: SCRIPT_TAB,
                field_key: EDITOR_ARGUMENTS_KEY,
                message: tr("That is too many arguments"),
            });
        }
        Ok(())
    }

    fn read_kinds(&self) -> Result<Vec<InputKind>, ValidationError> {
        let mut kinds = Vec::new();
        if self.files {
            kinds.push(InputKind::File);
        }
        if self.folders {
            kinds.push(InputKind::Folder);
        }
        if kinds.is_empty() {
            return Err(ValidationError {
                tab: BEHAVIOR_TAB,
                field_key: EDITOR_FILES_KEY,
                message: tr("Choose Files, Folders, or both"),
            });
        }
        Ok(kinds)
    }

    fn read_max_items(&self) -> Result<Option<usize>, ValidationError> {
        match self.max_items.text.trim() {
            "" => Ok(None),
            value => value
                .parse::<usize>()
                .map(Some)
                .map_err(|_| ValidationError {
                    tab: BEHAVIOR_TAB,
                    field_key: EDITOR_MAX_ITEMS_KEY,
                    message: tr("The maximum item count must be a number"),
                }),
        }
    }

    fn check_launch(&self, is_script: bool) -> Result<(), ValidationError> {
        if !is_script && self.program.text.trim().is_empty() {
            return Err(ValidationError {
                tab: SCRIPT_TAB,
                field_key: EDITOR_PROGRAM_KEY,
                message: tr("Enter the program to run"),
            });
        }
        if is_script && self.entrypoint.text.trim().is_empty() {
            return Err(ValidationError {
                tab: SCRIPT_TAB,
                field_key: EDITOR_ENTRYPOINT_KEY,
                message: tr("Enter a script file name"),
            });
        }
        Ok(())
    }

    fn read_description(&self) -> Option<String> {
        let description = self.description.text.trim().to_owned();
        (!description.is_empty()).then_some(description)
    }

    fn read_extensions(&self) -> Vec<String> {
        self.extensions
            .text
            .split(',')
            .map(|ext| ext.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|ext| !ext.is_empty())
            .collect()
    }

    fn read_run(&self) -> RunSpec {
        let is_script = self.runtime != ActionRuntime::Command;
        RunSpec {
            runtime: self.runtime,
            entrypoint: is_script.then(|| self.entrypoint.text.trim().to_owned()),
            program: (!is_script).then(|| self.program.text.trim().to_owned()),
            args: if is_script {
                Vec::new()
            } else {
                self.read_arguments()
            },
            mode: self.mode_choice,
            on_error: self.policy,
            working_directory: self.working_directory,
            confirm: self.confirm,
        }
    }

    /// Unedited arguments keep their saved form.
    fn read_arguments(&self) -> Vec<String> {
        let arguments = self.arguments.text.as_str();
        if arguments == self.original_arguments.join("\n") {
            self.original_arguments.clone()
        } else if arguments.is_empty() {
            Vec::new()
        } else {
            arguments.split('\n').map(str::to_owned).collect()
        }
    }

    fn validation_error(&self, error: &ActionError) -> ValidationError {
        let (tab, field_key) = match error {
            ActionError::InvalidId(_) => (GENERAL_TAB, EDITOR_ID_KEY),
            ActionError::InvalidName => (GENERAL_TAB, EDITOR_NAME_KEY),
            ActionError::InvalidDescription => (GENERAL_TAB, EDITOR_DESCRIPTION_KEY),
            ActionError::InvalidExtension(_) | ActionError::TooManyConditions(_) => {
                (BEHAVIOR_TAB, EDITOR_EXTENSIONS_KEY)
            }
            ActionError::InvalidItemRange => (BEHAVIOR_TAB, EDITOR_MAX_ITEMS_KEY),
            ActionError::MissingEntrypoint | ActionError::InvalidEntrypoint(_) => {
                (SCRIPT_TAB, EDITOR_ENTRYPOINT_KEY)
            }
            ActionError::MissingProgram | ActionError::InvalidProgram(_) => {
                (SCRIPT_TAB, EDITOR_PROGRAM_KEY)
            }
            _ => (SCRIPT_TAB, EDITOR_ARGUMENTS_KEY),
        };
        ValidationError {
            tab,
            field_key,
            message: error.user_message(),
        }
    }

    fn read_script(&self, definition: &ActionDefinition) -> Option<ActionScript> {
        let entrypoint = definition.run.script_entrypoint()?;
        Some(ActionScript {
            file_name: entrypoint.to_owned(),
            contents: self.active_script_editor().text().to_owned(),
        })
    }
}

fn initial_scripts(runtime: ActionRuntime, script: Option<&ActionScript>) -> (String, String) {
    let mut python = python_template();
    let mut bash = bash_template().to_owned();
    if let Some(script) = script {
        match runtime {
            ActionRuntime::Bash => bash = script.contents.clone(),
            _ => python = script.contents.clone(),
        }
    }
    (python, bash)
}

fn initial_kinds(kinds: &[InputKind]) -> (bool, bool) {
    if kinds.is_empty() {
        (true, true)
    } else {
        (
            kinds.contains(&InputKind::File),
            kinds.contains(&InputKind::Folder),
        )
    }
}

fn replace_default_entrypoint(entrypoint: &mut TextEdit, runtime: ActionRuntime) {
    let current = entrypoint.text.trim();
    let is_default = current.is_empty() || default_entrypoint_for_any(current).is_some();
    if is_default {
        entrypoint.replace_text(default_entrypoint(runtime));
    }
}

pub(super) fn default_entrypoint(runtime: ActionRuntime) -> &'static str {
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

pub(super) fn bash_template() -> &'static str {
    "#!/usr/bin/env bash\nwhile IFS= read -r -d '' path; do\n    printf 'Processing %s\\n' \"$path\"\ndone < \"$STRATA_ACTION_PATHS\"\n"
}

pub(super) fn python_template() -> String {
    ACTION_EXAMPLES[0].script()
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

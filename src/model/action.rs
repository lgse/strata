// SPDX-License-Identifier: MIT

//! Custom action definitions: the portable, user-authored `action.toml` contract.
//!
//! This module is deliberately pure. It parses, validates, and matches action
//! definitions, but never touches the filesystem, the process table, PATH, or
//! widgets. Filesystem discovery, interpreter resolution, and process execution
//! live in `adapters::local_actions` and `adapters::local_jobs`.
//!
//! A value of [`ActionDefinition`] can only be produced through [`ActionDefinition::parse`],
//! so an existing definition is always schema-valid. Filesystem-dependent checks
//! (does the entrypoint exist? is the interpreter installed?) are applied by the
//! store once it re-reads the script's shebang.

use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub(crate) mod examples;

/// Version of the `action.toml` schema this build understands.
pub const ACTION_SCHEMA_VERSION: u32 = 1;

pub const MAX_ACTION_ID_CHARS: usize = 64;
pub const MAX_ACTION_NAME_CHARS: usize = 64;
pub const MAX_ACTION_DESCRIPTION_CHARS: usize = 240;
pub const MAX_ICON_CHARS: usize = 64;
pub const MAX_ENTRYPOINT_CHARS: usize = 128;
pub const MAX_PROGRAM_CHARS: usize = 512;
pub const MAX_EXTENSIONS: usize = 64;
pub const MAX_MIME_TYPES: usize = 64;
pub const MAX_ARGUMENTS: usize = 32;
pub const MAX_ARGUMENT_CHARS: usize = 512;
pub const MAX_ITEMS_PER_ACTION: usize = 10_000;
/// Longest script prefix inspected for a shebang line.
pub const MAX_SHEBANG_BYTES: usize = 256;

/// Marker appended to a generated manifest so hand-edits keep their bearings.
pub const MANIFEST_HEADER: &str = "\
# Strata custom action. See docs/custom-actions.md.
# Edited by Settings → Actions, but hand-editing is supported.";

/// A validated custom action definition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionDefinition {
    pub schema_version: u32,
    /// Directory name under `$XDG_CONFIG_HOME/strata/actions` and stable identity.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Bundled Lucide icon slug, without the `strata-` prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub menu: MenuPlacement,
    pub when: ActionConditions,
    pub run: RunSpec,
}

/// Whether the action appears inline or inside the actions submenu.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MenuPlacement {
    #[default]
    Submenu,
    Top,
}

/// Which interpreter or program runs the action.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionRuntime {
    #[default]
    Python,
    Bash,
    /// An existing executable plus explicit argument tokens.
    Command,
}

impl ActionRuntime {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Python => "Python",
            Self::Bash => "Bash",
            Self::Command => "Command",
        }
    }

    /// The default interpreter looked up on `PATH` when a script has no usable shebang.
    pub const fn default_interpreter(self) -> Option<&'static str> {
        match self {
            Self::Python => Some("python3"),
            Self::Bash => Some("bash"),
            Self::Command => None,
        }
    }
}

/// Whether the action runs once per input or once for the whole selection.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionMode {
    /// One invocation carrying every selected path.
    #[default]
    WholeSelection,
    /// One invocation per selected path, with per-item progress and failure policy.
    PerItem,
}

/// What to do after a failed invocation in [`ExecutionMode::PerItem`].
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorPolicy {
    #[default]
    Continue,
    Stop,
}

/// Directory an invocation starts in.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkingDirectory {
    /// The folder the action was invoked from.
    #[default]
    Parent,
    Home,
    /// The action's own directory.
    Action,
}

/// The kind of file-system entry an action can act on.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputKind {
    File,
    Folder,
}

/// Declarative applicability rules. Evaluating these never runs user code.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionConditions {
    /// Empty means every kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<InputKind>,
    /// Lowercase extensions without the leading dot. Empty means every extension.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// `image/*` or `image/png` patterns. Empty means every content type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mime_types: Vec<String>,
    #[serde(default = "one_item")]
    pub min_items: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,
}

impl Default for ActionConditions {
    fn default() -> Self {
        Self {
            kinds: Vec::new(),
            extensions: Vec::new(),
            mime_types: Vec::new(),
            min_items: 1,
            max_items: None,
        }
    }
}

/// How an invocation is started.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    pub runtime: ActionRuntime,
    /// Script file name beside `action.toml`. Required for Python and Bash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    /// Executable for [`ActionRuntime::Command`], absolute or resolved on `PATH`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default)]
    pub mode: ExecutionMode,
    /// Ignored by [`ExecutionMode::WholeSelection`], which has a single invocation.
    #[serde(default)]
    pub on_error: ErrorPolicy,
    #[serde(default)]
    pub working_directory: WorkingDirectory,
    /// Ask before each invocation.
    #[serde(default)]
    pub confirm: bool,
}

/// One expanded argument for an [`ActionRuntime::Command`] invocation.
///
/// Paths reach the program as separate, absolute argv entries. Nothing is ever
/// interpolated into shell source, and no token expands a bare file name, so a
/// selected file cannot inject an option such as `-rf`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArgumentToken {
    Literal(String),
    /// The single input, valid in [`ExecutionMode::PerItem`].
    Path,
    /// Every input, valid in [`ExecutionMode::WholeSelection`].
    Paths,
    /// The folder the action was invoked from.
    Parent,
}

impl ArgumentToken {
    pub const TOKENS: [(&'static str, Self); 3] = [
        ("{path}", Self::Path),
        ("{paths}", Self::Paths),
        ("{parent}", Self::Parent),
    ];
}

/// An interpreter declared by a script shebang.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Interpreter {
    pub program: String,
    pub arguments: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterpreterFamily {
    Python,
    Shell,
    Other,
}

/// Why a definition could not be loaded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionError {
    Toml(String),
    Serialize(String),
    UnsupportedSchema {
        found: u32,
    },
    InvalidId(String),
    InvalidName,
    InvalidDescription,
    InvalidIcon,
    MissingEntrypoint,
    UnexpectedEntrypoint,
    MissingProgram,
    UnexpectedProgram,
    InvalidEntrypoint(String),
    InvalidProgram(String),
    TooManyArguments,
    ArgumentTooLong,
    InvalidArgument(String),
    PathTokenInPerItem,
    PathsTokenInWholeSelection,
    InvalidExtension(String),
    InvalidMimeType(String),
    TooManyConditions(&'static str),
    InvalidItemRange,
    InvalidShebang,
    ShebangMismatch {
        declared: String,
        runtime: ActionRuntime,
    },
}

impl fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(message) => {
                write!(formatter, "The action file is not valid TOML: {message}")
            }
            Self::Serialize(message) => {
                write!(formatter, "Unable to write the action file: {message}")
            }
            Self::UnsupportedSchema { found } => write!(
                formatter,
                "This action uses schema version {found}, but this Strata build supports {ACTION_SCHEMA_VERSION}"
            ),
            Self::InvalidId(id) => write!(
                formatter,
                "The action id “{id}” must start with a lowercase letter or digit and use only lowercase letters, digits, dots, dashes, or underscores"
            ),
            Self::InvalidName => write!(
                formatter,
                "Enter a name of 1–{MAX_ACTION_NAME_CHARS} characters without control characters"
            ),
            Self::InvalidDescription => write!(
                formatter,
                "Descriptions are limited to {MAX_ACTION_DESCRIPTION_CHARS} characters"
            ),
            Self::InvalidIcon => write!(
                formatter,
                "Icons must be a Lucide icon name of lowercase letters, digits, and dashes"
            ),
            Self::MissingEntrypoint => {
                write!(
                    formatter,
                    "A Python or Bash action needs an entrypoint script"
                )
            }
            Self::UnexpectedEntrypoint => {
                write!(
                    formatter,
                    "Command actions must not declare an entrypoint script"
                )
            }
            Self::MissingProgram => write!(formatter, "A command action needs a program"),
            Self::UnexpectedProgram => write!(
                formatter,
                "Python and Bash actions must not declare a program; the interpreter comes from the script"
            ),
            Self::InvalidEntrypoint(name) => write!(
                formatter,
                "“{name}” must be a plain file name beside action.toml without a path or leading dash"
            ),
            Self::InvalidProgram(name) => write!(
                formatter,
                "“{name}” is not a usable program name: it cannot be empty, start with a dash, or contain a NUL byte"
            ),
            Self::TooManyArguments => {
                write!(
                    formatter,
                    "Commands accept at most {MAX_ARGUMENTS} arguments"
                )
            }
            Self::ArgumentTooLong => write!(
                formatter,
                "Arguments are limited to {MAX_ARGUMENT_CHARS} characters"
            ),
            Self::InvalidArgument(argument) => write!(
                formatter,
                "“{argument}” is not a usable argument. Write literal text, or one of {{path}}, {{paths}}, or {{parent}} on its own"
            ),
            Self::PathTokenInPerItem => write!(
                formatter,
                "{{path}} is for per-item actions; use {{paths}} for a whole selection"
            ),
            Self::PathsTokenInWholeSelection => write!(
                formatter,
                "{{paths}} is for whole-selection actions; use {{path}} for a per-item action"
            ),
            Self::InvalidExtension(extension) => write!(
                formatter,
                "“{extension}” is not a usable extension: write it without a dot, for example “png”"
            ),
            Self::InvalidMimeType(mime) => write!(
                formatter,
                "“{mime}” is not a usable content type: write “image/png” or “image/*”"
            ),
            Self::TooManyConditions(what) => {
                write!(formatter, "At most {MAX_EXTENSIONS} {what} can be listed")
            }
            Self::InvalidItemRange => write!(
                formatter,
                "Item counts must be at least 1, at most {MAX_ITEMS_PER_ACTION}, and max_items cannot be smaller than min_items"
            ),
            Self::InvalidShebang => write!(
                formatter,
                "The script starts with “#!” but does not name a program, so the interpreter is unknown"
            ),
            Self::ShebangMismatch { declared, runtime } => write!(
                formatter,
                "The script declares “{declared}”, which does not match the {} runtime",
                runtime.label()
            ),
        }
    }
}

impl std::error::Error for ActionError {}

/// One selected entry offered to action matching.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionInput {
    pub kind: InputKind,
    /// Native name, kept byte-exact so unusual names still match correctly.
    pub name: std::ffi::OsString,
    /// Content type guessed by the caller; `None` when unknown.
    pub content_type: Option<String>,
}

/// Content type every folder reports to matching rules.
pub const FOLDER_CONTENT_TYPE: &str = "inode/directory";

impl ActionInput {
    #[cfg(test)]
    pub fn file(name: impl Into<std::ffi::OsString>, content_type: Option<&str>) -> Self {
        Self {
            kind: InputKind::File,
            name: name.into(),
            content_type: content_type.map(str::to_owned),
        }
    }

    pub fn folder(name: impl Into<std::ffi::OsString>) -> Self {
        Self {
            kind: InputKind::Folder,
            name: name.into(),
            content_type: Some(FOLDER_CONTENT_TYPE.to_owned()),
        }
    }

    /// Lowercase extension without the dot, taken byte-exactly from the native name.
    #[cfg(test)]
    pub fn extension(&self) -> Option<String> {
        let extension = extension_bytes(&self.name)?;
        std::str::from_utf8(extension)
            .ok()
            .map(|extension| extension.to_ascii_lowercase())
    }
}

fn enabled_by_default() -> bool {
    true
}

fn one_item() -> usize {
    1
}

fn extension_bytes(name: &OsStr) -> Option<&[u8]> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = name.as_bytes();
    let dot = bytes.iter().rposition(|byte| *byte == b'.')?;
    // A leading dot alone is a hidden file name, not an extension.
    let extension = bytes.get(dot + 1..)?;
    (!extension.is_empty() && dot > 0).then_some(extension)
}

fn ascii_eq_ignore_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

impl ActionDefinition {
    /// Parses and validates a manifest. The returned value is always schema-valid.
    pub fn parse(source: &str) -> Result<Self, ActionError> {
        let definition: Self = toml::from_str(source)
            .map_err(|error| ActionError::Toml(error.message().to_owned()))?;
        definition.validate()?;
        Ok(definition)
    }

    /// Serializes a manifest for `action.toml`, including the generated-file header.
    pub fn to_manifest(&self) -> Result<String, ActionError> {
        let body = self.validate().and_then(|()| {
            toml::to_string_pretty(self).map_err(|error| ActionError::Serialize(error.to_string()))
        })?;
        Ok(format!("{MANIFEST_HEADER}\n\n{body}"))
    }

    pub fn validate(&self) -> Result<(), ActionError> {
        if self.schema_version != ACTION_SCHEMA_VERSION {
            return Err(ActionError::UnsupportedSchema {
                found: self.schema_version,
            });
        }
        validate_id(&self.id)?;
        validate_text(&self.name, MAX_ACTION_NAME_CHARS)
            .then_some(())
            .ok_or(ActionError::InvalidName)?;
        if let Some(description) = &self.description
            && (!validate_text(description, MAX_ACTION_DESCRIPTION_CHARS) || description.is_empty())
        {
            return Err(ActionError::InvalidDescription);
        }
        if let Some(icon) = &self.icon
            && !valid_icon(icon)
        {
            return Err(ActionError::InvalidIcon);
        }
        self.when.validate()?;
        self.run.validate()?;
        Ok(())
    }

    /// Resolves the interpreter a script declares, if the manifest is consistent with it.
    ///
    /// `Ok(None)` means the script has no shebang and the runtime default applies.
    pub fn interpreter_for_source(&self, source: &str) -> Result<Option<Interpreter>, ActionError> {
        self.run.interpreter_for_source(source)
    }
}

fn validate_id(id: &str) -> Result<(), ActionError> {
    if valid_action_id(id) {
        Ok(())
    } else {
        Err(ActionError::InvalidId(id.to_owned()))
    }
}

/// Whether `id` is a safe single directory component for an action.
///
/// The store relies on this to keep writes and deletions confined to one child
/// of the actions directory.
pub fn valid_action_id(id: &str) -> bool {
    let mut characters = id.chars();
    id.chars().count() <= MAX_ACTION_ID_CHARS
        && characters
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && id.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || "._-".contains(character)
        })
}

fn validate_text(text: &str, max_chars: usize) -> bool {
    !text.is_empty()
        && !text.starts_with(' ')
        && !text.ends_with(' ')
        && text.chars().count() <= max_chars
        && !text.chars().any(char::is_control)
}

fn valid_icon(icon: &str) -> bool {
    !icon.is_empty()
        && icon.chars().count() <= MAX_ICON_CHARS
        && icon.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

impl ActionConditions {
    /// Whether every input satisfies the conditions, so a mixed selection never
    /// silently drops entries an author did not intend to skip.
    pub fn matches(&self, inputs: &[ActionInput]) -> bool {
        if inputs.is_empty() || inputs.len() < self.min_items {
            return false;
        }
        if let Some(max_items) = self.max_items
            && inputs.len() > max_items
        {
            return false;
        }
        inputs.iter().all(|input| self.matches_input(input))
    }

    pub fn matches_input(&self, input: &ActionInput) -> bool {
        if !self.kinds.is_empty() && !self.kinds.contains(&input.kind) {
            return false;
        }
        if !self.extensions.is_empty() {
            let Some(extension) = extension_bytes(&input.name) else {
                return false;
            };
            if !self
                .extensions
                .iter()
                .any(|candidate| ascii_eq_ignore_case(candidate.as_bytes(), extension))
            {
                return false;
            }
        }
        if !self.mime_types.is_empty() {
            let Some(content_type) = input.content_type.as_deref() else {
                return false;
            };
            if !self
                .mime_types
                .iter()
                .any(|pattern| mime_matches(pattern, content_type))
            {
                return false;
            }
        }
        true
    }

    fn validate(&self) -> Result<(), ActionError> {
        if self.extensions.len() > MAX_EXTENSIONS {
            return Err(ActionError::TooManyConditions("extensions"));
        }
        if self.mime_types.len() > MAX_MIME_TYPES {
            return Err(ActionError::TooManyConditions("content types"));
        }
        for extension in &self.extensions {
            if !valid_extension(extension) {
                return Err(ActionError::InvalidExtension(extension.clone()));
            }
        }
        for mime in &self.mime_types {
            if !valid_mime_type(mime) {
                return Err(ActionError::InvalidMimeType(mime.clone()));
            }
        }
        if self.min_items < 1 || self.min_items > MAX_ITEMS_PER_ACTION {
            return Err(ActionError::InvalidItemRange);
        }
        if let Some(max_items) = self.max_items
            && (max_items < self.min_items || max_items > MAX_ITEMS_PER_ACTION)
        {
            return Err(ActionError::InvalidItemRange);
        }
        Ok(())
    }
}

fn valid_extension(extension: &str) -> bool {
    !extension.is_empty()
        && !extension.starts_with('.')
        && !extension.contains('/')
        && !extension.contains('\\')
        && !extension.contains('*')
        && extension.chars().count() <= 24
        && extension
            .chars()
            .all(|character| !character.is_control() && !character.is_whitespace())
}

fn valid_mime_type(mime: &str) -> bool {
    if mime.chars().count() > 128 || mime.chars().any(char::is_control) {
        return false;
    }
    let Some((category, specific)) = mime.split_once('/') else {
        return false;
    };
    let valid_part = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "+.-_".contains(character))
    };
    valid_part(category) && (specific == "*" || valid_part(specific))
}

fn mime_matches(pattern: &str, content_type: &str) -> bool {
    match pattern.split_once('/') {
        Some((category, "*")) => content_type
            .split_once('/')
            .is_some_and(|(other, _)| category.eq_ignore_ascii_case(other)),
        _ => pattern.eq_ignore_ascii_case(content_type),
    }
}

impl RunSpec {
    pub fn script_entrypoint(&self) -> Option<&str> {
        matches!(self.runtime, ActionRuntime::Python | ActionRuntime::Bash)
            .then(|| self.entrypoint.as_deref())
            .flatten()
    }

    /// Argument tokens for [`ActionRuntime::Command`]; empty for scripts.
    pub fn argument_tokens(&self) -> Result<Vec<ArgumentToken>, ActionError> {
        if self.runtime != ActionRuntime::Command {
            return Ok(Vec::new());
        }
        parse_argument_tokens(&self.args, self.mode)
    }

    /// Resolves the interpreter a script declares, if the manifest is consistent with it.
    ///
    /// Command actions never inspect the entrypoint, because their program is not a script.
    pub fn interpreter_for_source(&self, source: &str) -> Result<Option<Interpreter>, ActionError> {
        if self.runtime == ActionRuntime::Command {
            return Ok(None);
        }
        if !has_shebang(source) {
            return Ok(None);
        }
        let interpreter = interpreter_from_source(source).ok_or(ActionError::InvalidShebang)?;
        let family = interpreter_family(&interpreter.program);
        let expected = match self.runtime {
            ActionRuntime::Python => InterpreterFamily::Python,
            ActionRuntime::Bash => InterpreterFamily::Shell,
            ActionRuntime::Command => InterpreterFamily::Other,
        };
        if family != expected {
            return Err(ActionError::ShebangMismatch {
                declared: format!("#!{}", interpreter.program),
                runtime: self.runtime,
            });
        }
        Ok(Some(interpreter))
    }

    fn validate(&self) -> Result<(), ActionError> {
        match self.runtime {
            ActionRuntime::Python | ActionRuntime::Bash => {
                if self.program.is_some() {
                    return Err(ActionError::UnexpectedProgram);
                }
                if !self.args.is_empty() {
                    return Err(ActionError::InvalidArgument(self.args[0].clone()));
                }
                let Some(entrypoint) = self.entrypoint.as_deref() else {
                    return Err(ActionError::MissingEntrypoint);
                };
                if !valid_entrypoint(entrypoint) {
                    return Err(ActionError::InvalidEntrypoint(entrypoint.to_owned()));
                }
            }
            ActionRuntime::Command => {
                if self.entrypoint.is_some() {
                    return Err(ActionError::UnexpectedEntrypoint);
                }
                let Some(program) = self.program.as_deref() else {
                    return Err(ActionError::MissingProgram);
                };
                if !valid_program(program) {
                    return Err(ActionError::InvalidProgram(program.to_owned()));
                }
                self.argument_tokens()?;
            }
        }
        Ok(())
    }
}

fn valid_entrypoint(entrypoint: &str) -> bool {
    if entrypoint.is_empty() || entrypoint.chars().count() > MAX_ENTRYPOINT_CHARS {
        return false;
    }
    let path = Path::new(entrypoint);
    let mut components = path.components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
        && !entrypoint.starts_with('-')
        && !entrypoint.contains('\0')
}

fn valid_program(program: &str) -> bool {
    !program.is_empty()
        && program.chars().count() <= MAX_PROGRAM_CHARS
        && !program.starts_with('-')
        && !program.contains('\0')
        && !program.chars().any(char::is_control)
}

fn parse_argument_tokens(
    arguments: &[String],
    mode: ExecutionMode,
) -> Result<Vec<ArgumentToken>, ActionError> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(ActionError::TooManyArguments);
    }
    let mut tokens = Vec::with_capacity(arguments.len());
    for argument in arguments {
        if argument.chars().count() > MAX_ARGUMENT_CHARS {
            return Err(ActionError::ArgumentTooLong);
        }
        if argument.contains(['{', '}']) {
            let Some((_, token)) = ArgumentToken::TOKENS
                .iter()
                .find(|(placeholder, _)| *placeholder == argument.as_str())
            else {
                return Err(ActionError::InvalidArgument(argument.clone()));
            };
            match token {
                ArgumentToken::Path if mode == ExecutionMode::WholeSelection => {
                    return Err(ActionError::PathTokenInPerItem);
                }
                ArgumentToken::Paths if mode == ExecutionMode::PerItem => {
                    return Err(ActionError::PathsTokenInWholeSelection);
                }
                _ => {}
            }
            tokens.push(token.clone());
        } else if argument.is_empty() {
            // An empty literal argument is legitimate, if unusual.
            tokens.push(ArgumentToken::Literal(String::new()));
        } else {
            tokens.push(ArgumentToken::Literal(argument.clone()));
        }
    }
    Ok(tokens)
}

/// Expands argument tokens for one invocation.
///
/// Paths must be absolute so a selected file can never be read as a program
/// option; a relative input is rejected rather than quietly passed through.
pub fn expand_arguments(
    tokens: &[ArgumentToken],
    inputs: &[PathBuf],
    parent: &Path,
) -> Result<Vec<std::ffi::OsString>, ArgumentTokenError> {
    let mut arguments = Vec::new();
    for token in tokens {
        match token {
            ArgumentToken::Literal(literal) => arguments.push(literal.into()),
            ArgumentToken::Path => {
                let [input] = inputs else {
                    return Err(ArgumentTokenError::PathCount(inputs.len()));
                };
                push_absolute(&mut arguments, input)?;
            }
            ArgumentToken::Paths => {
                for input in inputs {
                    push_absolute(&mut arguments, input)?;
                }
            }
            ArgumentToken::Parent => push_absolute(&mut arguments, parent)?,
        }
    }
    Ok(arguments)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArgumentTokenError {
    PathCount(usize),
    RelativePath(PathBuf),
}

impl fmt::Display for ArgumentTokenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathCount(count) => write!(
                formatter,
                "{{path}} needs exactly one selected item, but {count} were offered"
            ),
            Self::RelativePath(path) => write!(
                formatter,
                "“{}” is not an absolute path, so it cannot be passed safely",
                path.display()
            ),
        }
    }
}

fn push_absolute(
    arguments: &mut Vec<std::ffi::OsString>,
    path: &Path,
) -> Result<(), ArgumentTokenError> {
    if !path.is_absolute() {
        return Err(ArgumentTokenError::RelativePath(path.to_path_buf()));
    }
    arguments.push(path.as_os_str().to_owned());
    Ok(())
}

/// Whether `source` opens with a `#!` line.
pub fn has_shebang(source: &str) -> bool {
    first_line(source).is_some_and(|line| line.starts_with("#!"))
}

/// Parses the shebang of `source`, resolving `/usr/bin/env` indirection.
///
/// Returns `None` when there is no shebang or the line does not name a program.
pub fn interpreter_from_source(source: &str) -> Option<Interpreter> {
    let line = first_line(source)?;
    let rest = line.strip_prefix("#!")?.trim();
    let mut words = rest.split_ascii_whitespace();
    let program = words.next()?;
    if program.is_empty() || file_name(program).is_empty() {
        return None;
    }
    let mut arguments: Vec<String> = words.map(str::to_owned).collect();
    if file_name(program) == "env" {
        // `env` may carry flags (`-S`, `-u NAME`, `--`) and `NAME=value`
        // assignments before the real interpreter.
        let index = arguments
            .iter()
            .position(|word| !word.starts_with('-') && !word.contains('='))?;
        let program = arguments.remove(index);
        arguments.drain(..index);
        if program.is_empty() || file_name(&program).is_empty() {
            return None;
        }
        return Some(Interpreter { program, arguments });
    }
    Some(Interpreter {
        program: program.to_owned(),
        arguments,
    })
}

fn first_line(source: &str) -> Option<&str> {
    let mut end = source.len().min(MAX_SHEBANG_BYTES);
    while end > 0 && !source.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = source.get(..end)?;
    let line = prefix.split('\n').next()?;
    Some(line.trim_end_matches('\r'))
}

fn file_name(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

/// Classifies an interpreter program name into a family.
pub fn interpreter_family(program: &str) -> InterpreterFamily {
    let name = file_name(program);
    let lowered = name.to_ascii_lowercase();
    let stem = lowered.split('.').next().unwrap_or(&lowered);
    if stem.starts_with("python") || lowered.starts_with("python") {
        InterpreterFamily::Python
    } else if matches!(
        stem,
        "sh" | "bash" | "dash" | "zsh" | "ksh" | "mksh" | "ash" | "yash" | "oksh"
    ) {
        InterpreterFamily::Shell
    } else {
        InterpreterFamily::Other
    }
}

/// Slug for a new action id derived from a display name.
pub fn suggest_id(name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() || !slug.starts_with(|character: char| character.is_ascii_alphanumeric()) {
        return "action".to_owned();
    }
    slug.chars().take(MAX_ACTION_ID_CHARS).collect()
}

#[cfg(test)]
mod tests;

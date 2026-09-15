// SPDX-License-Identifier: MIT

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

use glib::user_config_dir;
use serde::Deserialize;

const ACTIONS_FILE: &str = "actions.toml";
const PATH_TOKEN: &str = "{path}";
const PATHS_TOKEN: &str = "{paths}";

static LOADED: OnceLock<Vec<FileAction>> = OnceLock::new();

/// User-defined context-menu command from `$XDG_CONFIG_HOME/strata/actions.toml`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileAction {
    pub id: String,
    pub label: String,
    pub icon: String,
    command: Vec<String>,
    targets: ActionTargets,
    requires: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ActionTargets {
    Files,
    Folders,
    #[default]
    Any,
}

#[derive(Debug, Deserialize)]
struct ActionsFile {
    #[serde(default)]
    actions: Vec<RawAction>,
}

#[derive(Debug, Deserialize)]
struct RawAction {
    id: String,
    label: String,
    #[serde(default)]
    icon: Option<String>,
    command: Vec<String>,
    #[serde(default)]
    targets: ActionTargets,
    #[serde(default)]
    requires: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    File,
    Folder,
}

#[derive(Debug, Eq, PartialEq)]
pub enum LaunchError {
    EmptyCommand,
    MissingBinary(String),
    Spawn(String),
}

impl FileAction {
    pub fn matches(&self, kinds: &[ActionKind], path_count: usize) -> bool {
        if path_count == 0 || !self.requires_available() {
            return false;
        }
        if uses_single_path_token(&self.command) && path_count != 1 {
            return false;
        }
        kinds.iter().all(|kind| self.targets.allows(*kind))
    }

    pub fn argv_for(&self, paths: &[&Path]) -> Option<Vec<OsString>> {
        build_argv(&self.command, paths)
    }

    fn requires_available(&self) -> bool {
        self.requires.iter().all(|name| command_exists(name))
            && self
                .command
                .first()
                .is_some_and(|program| command_exists(program))
    }
}

impl ActionTargets {
    fn allows(self, kind: ActionKind) -> bool {
        matches!(
            (self, kind),
            (Self::Any, _) | (Self::Files, ActionKind::File) | (Self::Folders, ActionKind::Folder)
        )
    }
}

/// Process-wide actions from the user config file. Missing or invalid files
/// yield an empty list so a bad TOML file cannot prevent Strata from starting.
pub fn configured_actions() -> &'static [FileAction] {
    LOADED.get_or_init(load_user_actions)
}

pub fn load_actions_from(path: &Path) -> Vec<FileAction> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "unable to read file actions"
            );
            return Vec::new();
        }
    };
    parse_actions(&contents, path)
}

pub fn launch(action: &FileAction, paths: &[&Path]) -> Result<(), LaunchError> {
    let argv = action.argv_for(paths).ok_or(LaunchError::EmptyCommand)?;
    let program = argv.first().ok_or(LaunchError::EmptyCommand)?;
    let mut command = Command::new(program);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map(|_| ()).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LaunchError::MissingBinary(program.to_string_lossy().into_owned())
        } else {
            LaunchError::Spawn(error.to_string())
        }
    })
}

pub fn user_actions_path() -> PathBuf {
    user_config_dir().join("strata").join(ACTIONS_FILE)
}

fn load_user_actions() -> Vec<FileAction> {
    load_actions_from(&user_actions_path())
}

fn parse_actions(contents: &str, path: &Path) -> Vec<FileAction> {
    let parsed = match toml::from_str::<ActionsFile>(contents) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "ignoring invalid file actions"
            );
            return Vec::new();
        }
    };
    let mut actions = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for raw in parsed.actions {
        if raw.id.is_empty() || raw.label.trim().is_empty() || raw.command.is_empty() {
            tracing::warn!(
                path = %path.display(),
                id = %raw.id,
                "skipping incomplete file action"
            );
            continue;
        }
        if !seen.insert(raw.id.clone()) {
            tracing::warn!(
                path = %path.display(),
                id = %raw.id,
                "skipping duplicate file action id"
            );
            continue;
        }
        actions.push(FileAction {
            icon: raw
                .icon
                .filter(|name| crate::assets::icons::is_bundled_action_icon(name))
                .unwrap_or_else(|| crate::assets::icons::EXTERNAL_LINK.to_owned()),
            id: raw.id,
            label: raw.label,
            command: raw.command,
            targets: raw.targets,
            requires: raw.requires,
        });
    }
    actions
}

fn uses_single_path_token(command: &[String]) -> bool {
    command.iter().any(|arg| arg == PATH_TOKEN) && command.iter().all(|arg| arg != PATHS_TOKEN)
}

fn build_argv(command: &[String], paths: &[&Path]) -> Option<Vec<OsString>> {
    if command.is_empty() || paths.is_empty() {
        return None;
    }
    if uses_single_path_token(command) && paths.len() != 1 {
        return None;
    }
    let mut argv = Vec::new();
    let mut saw_paths_token = false;
    for arg in command {
        if arg == PATH_TOKEN {
            argv.push(paths[0].as_os_str().to_os_string());
        } else if arg == PATHS_TOKEN {
            saw_paths_token = true;
            argv.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
        } else {
            argv.push(OsString::from(arg));
        }
    }
    if !saw_paths_token && !command.iter().any(|arg| arg == PATH_TOKEN) {
        argv.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    }
    Some(argv)
}

fn command_exists(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let path = Path::new(name);
    if path.components().count() > 1 {
        return path.is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|directory| directory.join(name).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests;

impl LaunchError {
    pub fn dialog_detail(&self) -> String {
        match self {
            Self::EmptyCommand => "The action has no command".to_owned(),
            Self::MissingBinary(name) => format!("“{name}” is not installed"),
            Self::Spawn(error) => error.clone(),
        }
    }
}

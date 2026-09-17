// SPDX-License-Identifier: MIT

//! Custom action registry: the capability boundary between stored `action.toml`
//! definitions and the surfaces that show or run them.
//!
//! `ActionStore` is the contract implemented by `adapters::local_actions`; the
//! registry owns the cached catalog, delegates edits, and notifies observers.
//! Matching is a pure function over the cached catalog, so opening a context menu
//! never touches the filesystem or runs user code.

use std::{
    cell::RefCell,
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::model::{
    ActionDefinition, ActionError, ActionInput, ArgumentToken, InterpreterFamily, MenuPlacement,
};

use super::listeners::{ListenerGuard, Listeners};

#[cfg(test)]
mod tests;

/// How an action can be started, resolved once at load time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionProgram {
    /// A script file beside `action.toml` plus the interpreter that runs it.
    Script {
        interpreter: OsString,
        interpreter_arguments: Vec<OsString>,
        script: PathBuf,
        family: InterpreterFamily,
    },
    /// An executable plus already-validated argument tokens.
    Command {
        program: OsString,
        arguments: Vec<ArgumentToken>,
    },
}

/// Whether the action can run on this machine right now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionAvailability {
    Available(ActionProgram),
    /// A missing interpreter or program, explained for the person who can fix it.
    Unavailable {
        reason: String,
    },
}

/// A definition together with its directory and resolved program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionHandle {
    pub definition: ActionDefinition,
    pub directory: PathBuf,
    pub availability: ActionAvailability,
}

impl ActionHandle {
    pub fn id(&self) -> &str {
        &self.definition.id
    }

    pub fn name(&self) -> &str {
        &self.definition.name
    }

    pub fn program(&self) -> Option<&ActionProgram> {
        match &self.availability {
            ActionAvailability::Available(program) => Some(program),
            ActionAvailability::Unavailable { .. } => None,
        }
    }

    pub fn is_available(&self) -> bool {
        self.program().is_some()
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        match &self.availability {
            ActionAvailability::Unavailable { reason } => Some(reason),
            ActionAvailability::Available(_) => None,
        }
    }
}

/// A stored definition that could not be loaded, reported in Settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionLoadFailure {
    /// Directory name, which may not be a valid action id.
    pub directory: String,
    pub error: ActionStoreError,
}

/// A definition that applies to the current selection.
#[derive(Clone, Debug)]
pub struct MatchedAction {
    pub action: Rc<ActionHandle>,
    pub placement: MenuPlacement,
}

/// The cached catalog plus the definitions that failed to load.
#[derive(Clone, Debug, Default)]
pub struct ActionCatalog {
    actions: Vec<Rc<ActionHandle>>,
    failures: Vec<ActionLoadFailure>,
}

impl ActionCatalog {
    pub fn new(actions: Vec<Rc<ActionHandle>>, failures: Vec<ActionLoadFailure>) -> Self {
        Self { actions, failures }
    }

    pub fn actions(&self) -> &[Rc<ActionHandle>] {
        &self.actions
    }

    pub fn failures(&self) -> &[ActionLoadFailure] {
        &self.failures
    }

    pub fn get(&self, id: &str) -> Option<&Rc<ActionHandle>> {
        self.actions.iter().find(|action| action.id() == id)
    }

    /// Enabled definitions whose declarative rules accept every input.
    pub fn matches(&self, inputs: &[ActionInput]) -> Vec<MatchedAction> {
        let mut matched: Vec<_> = self
            .actions
            .iter()
            .filter(|action| action.definition.enabled && action.definition.when.matches(inputs))
            .map(|action| MatchedAction {
                action: action.clone(),
                placement: action.definition.menu,
            })
            .collect();
        matched.sort_by(|left, right| {
            left.action
                .name()
                .to_lowercase()
                .cmp(&right.action.name().to_lowercase())
                .then_with(|| left.action.id().cmp(right.action.id()))
        });
        matched
    }
}

/// A script file saved beside `action.toml`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionScript {
    pub file_name: String,
    pub contents: String,
}

/// Everything needed to create or replace one action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionWriteRequest {
    pub definition: ActionDefinition,
    /// Absent for [`crate::model::ActionRuntime::Command`].
    pub script: Option<ActionScript>,
}

/// Why a store operation failed. Every variant is meant to be readable in Settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionStoreError {
    Invalid(ActionError),
    Io(String),
    AlreadyExists(String),
    NotFound(String),
    /// The path is not a single normal file beside the manifest.
    NotARegularFile(String),
    /// The directory name and the manifest's `id` disagree.
    IdMismatch {
        declared: String,
        directory: String,
    },
    MissingEntrypoint(String),
    MissingProgram(String),
    MissingInterpreter(String),
    /// A directory that is not an importable action.
    NotAnActionDirectory(String),
}

impl fmt::Display for ActionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(formatter),
            Self::Io(message) => write!(formatter, "{message}"),
            Self::AlreadyExists(name) => {
                write!(formatter, "An action named “{name}” already exists")
            }
            Self::NotFound(id) => write!(formatter, "The action “{id}” no longer exists"),
            Self::IdMismatch {
                declared,
                directory,
            } => write!(
                formatter,
                "The manifest declares id \u{201c}{declared}\u{201d}, but its folder is named \u{201c}{directory}\u{201d}; they must match"
            ),
            Self::NotARegularFile(name) => write!(
                formatter,
                "“{name}” must be a regular file, not a link or directory"
            ),
            Self::MissingEntrypoint(name) => {
                write!(
                    formatter,
                    "The script “{name}” is missing beside action.toml"
                )
            }
            Self::MissingProgram(program) => {
                write!(formatter, "“{program}” was not found on your PATH")
            }
            Self::MissingInterpreter(program) => write!(
                formatter,
                "The interpreter “{program}” was not found. Install it, or point the action at an interpreter that exists"
            ),
            Self::NotAnActionDirectory(path) => {
                write!(formatter, "“{path}” does not contain an action.toml file")
            }
        }
    }
}

impl std::error::Error for ActionStoreError {}

impl From<ActionError> for ActionStoreError {
    fn from(error: ActionError) -> Self {
        Self::Invalid(error)
    }
}

/// Storage for custom actions. Implemented by `adapters::local_actions`.
pub trait ActionStore {
    /// Reads every stored action. Never fails as a whole: a broken definition is
    /// reported as a [`ActionLoadFailure`] so one bad file cannot hide the rest.
    fn load(&self) -> ActionCatalog;

    fn write(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError>;

    fn delete(&self, id: &str) -> Result<(), ActionStoreError>;

    /// Returns the stored script for editing, or `None` for command actions.
    fn read_script(&self, id: &str) -> Result<Option<ActionScript>, ActionStoreError>;

    /// Copies a manifest and its entrypoint out of an existing action directory.
    fn import(&self, source: &Path) -> Result<String, ActionStoreError>;

    /// Copies a stored action's manifest and entrypoint into `destination`.
    fn export(&self, id: &str, destination: &Path) -> Result<PathBuf, ActionStoreError>;
}

/// Reads the store once and keeps the catalog cached until something changes it.
pub struct ActionRegistry {
    store: Rc<dyn ActionStore>,
    catalog: RefCell<Rc<ActionCatalog>>,
    listeners: Rc<Listeners<Rc<dyn Fn()>>>,
}

impl ActionRegistry {
    pub fn new(store: Rc<dyn ActionStore>) -> Self {
        let catalog = Rc::new(store.load());
        Self {
            store,
            catalog: RefCell::new(catalog),
            listeners: Rc::new(Listeners::new()),
        }
    }

    pub fn catalog(&self) -> Rc<ActionCatalog> {
        self.catalog.borrow().clone()
    }

    /// Re-reads the store and notifies observers only when the result changed.
    pub fn reload(&self) -> Rc<ActionCatalog> {
        let reloaded = Rc::new(self.store.load());
        if catalog_equivalent(&self.catalog.borrow(), &reloaded) {
            return self.catalog.borrow().clone();
        }
        self.catalog.replace(reloaded.clone());
        self.notify();
        reloaded
    }

    pub fn write(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError> {
        self.store.write(request)?;
        self.reload();
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), ActionStoreError> {
        self.store.delete(id)?;
        self.reload();
        Ok(())
    }

    /// Reads the stored script so the editor can show what is actually there.
    pub fn read_script(&self, id: &str) -> Result<Option<ActionScript>, ActionStoreError> {
        self.store.read_script(id)
    }

    pub fn import(&self, source: &Path) -> Result<String, ActionStoreError> {
        let id = self.store.import(source)?;
        self.reload();
        Ok(id)
    }

    pub fn export(&self, id: &str, destination: &Path) -> Result<PathBuf, ActionStoreError> {
        self.store.export(id, destination)
    }

    /// Registers a change callback. The subscription ends when the returned
    /// observer drops, so a window or Settings page can own its own lifetime.
    pub(crate) fn observe(&self, callback: Rc<dyn Fn()>) -> ListenerGuard<Rc<dyn Fn()>> {
        self.listeners.add(callback)
    }

    fn notify(&self) {
        self.listeners.notify(|callback| callback());
    }
}

fn catalog_equivalent(left: &ActionCatalog, right: &ActionCatalog) -> bool {
    left.actions.len() == right.actions.len()
        && left
            .actions
            .iter()
            .zip(&right.actions)
            .all(|(left, right)| left == right)
        && left.failures == right.failures
}

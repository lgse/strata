// SPDX-License-Identifier: MIT

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionProgram {
    Script {
        interpreter: OsString,
        interpreter_arguments: Vec<OsString>,
        script: PathBuf,
        family: InterpreterFamily,
    },
    Command {
        program: OsString,
        arguments: Vec<ArgumentToken>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionAvailability {
    Available(ActionProgram),
    Unavailable { reason: String },
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionLoadFailure {
    /// Directory name, which may not be a valid action id.
    pub directory: String,
    pub error: ActionStoreError,
}

#[derive(Clone, Debug)]
pub struct MatchedAction {
    pub action: Rc<ActionHandle>,
    pub placement: MenuPlacement,
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionScript {
    pub file_name: String,
    pub contents: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionWriteRequest {
    pub definition: ActionDefinition,
    /// `None` preserves the stored script during metadata-only edits.
    pub script: Option<ActionScript>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionStoreError {
    Invalid(ActionError),
    Io(String),
    AlreadyExists(String),
    NotFound(String),
    NotARegularFile(String),
    IdMismatch { declared: String, directory: String },
    MissingEntrypoint(String),
    MissingProgram(String),
    MissingInterpreter(String),
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

pub trait ActionStore {
    /// A broken definition must not hide unrelated actions.
    fn load(&self) -> ActionCatalog;

    fn create(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError>;

    fn write(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError>;

    fn delete(&self, id: &str) -> Result<(), ActionStoreError>;

    fn read_script(&self, id: &str) -> Result<Option<ActionScript>, ActionStoreError>;

    fn import(&self, source: &Path) -> Result<String, ActionStoreError>;

    fn export(&self, id: &str, destination: &Path) -> Result<PathBuf, ActionStoreError>;
}

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

    pub fn reload(&self) -> Rc<ActionCatalog> {
        let reloaded = Rc::new(self.store.load());
        if catalog_equivalent(&self.catalog.borrow(), &reloaded) {
            return self.catalog.borrow().clone();
        }
        self.catalog.replace(reloaded.clone());
        self.notify();
        reloaded
    }

    pub fn create(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError> {
        self.store.create(request)?;
        self.reload();
        Ok(())
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

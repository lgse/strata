// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    fmt,
    future::Future,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures_channel::oneshot;

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

    fn stored(stored: StoredActions) -> Self {
        Self::new(
            stored.actions.into_iter().map(Rc::new).collect(),
            stored.failures,
        )
    }

    pub fn actions(&self) -> &[Rc<ActionHandle>] {
        &self.actions
    }

    pub fn failures(&self) -> &[ActionLoadFailure] {
        &self.failures
    }

    #[cfg(test)]
    pub fn get(&self, id: &str) -> Option<&Rc<ActionHandle>> {
        self.actions.iter().find(|action| action.id() == id)
    }

    pub fn matches(&self, inputs: &[ActionInput]) -> Vec<MatchedAction> {
        let mut matched: Vec<_> = self
            .actions
            .iter()
            .filter(|action| applies(action, inputs))
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

    pub fn numbered(&self, inputs: &[ActionInput]) -> Vec<Rc<ActionHandle>> {
        let (top, submenu): (Vec<_>, Vec<_>) = self
            .matches(inputs)
            .into_iter()
            .filter(|matched| matched.action.is_available())
            .partition(|matched| matched.placement == MenuPlacement::Top);
        top.into_iter()
            .chain(submenu)
            .map(|matched| matched.action)
            .take(NUMBERED_ACTIONS)
            .collect()
    }

    /// The action at `index` in the order Settings lists them, disabled ones
    /// included, when it is enabled, runnable and accepts every input.
    pub fn listed(&self, index: usize, inputs: &[ActionInput]) -> Option<Rc<ActionHandle>> {
        self.actions
            .get(index)
            .filter(|action| applies(action, inputs) && action.is_available())
            .cloned()
    }
}

const NUMBERED_ACTIONS: usize = 10;

fn applies(action: &ActionHandle, inputs: &[ActionInput]) -> bool {
    action.definition.enabled && action.definition.when.matches(inputs)
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

impl ActionStoreError {
    /// Localized counterpart of the English `Display`, which stays for logs.
    pub fn user_message(&self) -> String {
        use rust_i18n::t;
        match self {
            Self::Invalid(error) => return error.user_message(),
            Self::Io(message) => return message.clone(),
            Self::AlreadyExists(name) => {
                t!("An action named “%{name}” already exists", name = name)
            }
            Self::NotFound(id) => t!("The action “%{id}” no longer exists", id = id),
            Self::IdMismatch {
                declared,
                directory,
            } => t!(
                "The manifest declares id “%{declared}”, but its folder is named “%{directory}”; they must match",
                declared = declared,
                directory = directory
            ),
            Self::NotARegularFile(name) => t!(
                "“%{name}” must be a regular file, not a link or directory",
                name = name
            ),
            Self::MissingEntrypoint(name) => t!(
                "The script “%{name}” is missing beside action.toml",
                name = name
            ),
            Self::MissingProgram(program) => {
                t!("“%{program}” was not found on your PATH", program = program)
            }
            Self::MissingInterpreter(program) => t!(
                "The interpreter “%{program}” was not found. Install it, or point the action at an interpreter that exists",
                program = program
            ),
            Self::NotAnActionDirectory(path) => t!(
                "“%{path}” does not contain an action.toml file",
                path = path
            ),
        }
        .into_owned()
    }
}

impl std::error::Error for ActionStoreError {}

impl From<ActionError> for ActionStoreError {
    fn from(error: ActionError) -> Self {
        Self::Invalid(error)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoredActions {
    pub actions: Vec<ActionHandle>,
    pub failures: Vec<ActionLoadFailure>,
}

impl StoredActions {
    #[cfg(test)]
    pub fn get(&self, id: &str) -> Option<&ActionHandle> {
        self.actions.iter().find(|action| action.id() == id)
    }
}

pub trait ActionStore: Send + Sync {
    /// A broken definition must not hide unrelated actions.
    fn load(&self) -> StoredActions;

    fn locate(&self, stored: StoredActions) -> StoredActions;

    fn definition(&self, id: &str) -> Result<ActionDefinition, ActionStoreError>;

    fn read(&self, id: &str) -> Result<ActionWriteRequest, ActionStoreError>;

    fn create(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError>;

    fn write(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError>;

    fn delete(&self, id: &str) -> Result<(), ActionStoreError>;

    fn read_import(&self, source: &Path) -> Result<ActionWriteRequest, ActionStoreError>;

    fn import(&self, request: ActionWriteRequest) -> Result<String, ActionStoreError>;

    fn export(
        &self,
        request: &ActionWriteRequest,
        destination: &Path,
    ) -> Result<PathBuf, ActionStoreError>;
}

pub struct ActionRegistry {
    store: Arc<dyn ActionStore>,
    catalog: RefCell<Rc<ActionCatalog>>,
    queued: Cell<u64>,
    adopted: Cell<u64>,
    unread: Arc<AtomicUsize>,
    listeners: Rc<Listeners<Rc<dyn Fn()>>>,
}

impl ActionRegistry {
    pub fn new(store: Arc<dyn ActionStore>) -> Rc<Self> {
        Rc::new(Self {
            store,
            catalog: RefCell::default(),
            queued: Cell::new(0),
            adopted: Cell::new(0),
            unread: Arc::default(),
            listeners: Rc::new(Listeners::new()),
        })
    }

    pub fn catalog(&self) -> Rc<ActionCatalog> {
        self.catalog.borrow().clone()
    }

    pub fn reload(self: &Rc<Self>) -> impl Future<Output = Rc<ActionCatalog>> + use<> {
        let unread = self.unread.clone();
        unread.fetch_add(1, Ordering::SeqCst);
        let (order, loaded) = self.queue(move |store| {
            unread.fetch_sub(1, Ordering::SeqCst);
            store.load()
        });
        let this = self.clone();
        async move {
            if let Ok(stored) = loaded.await {
                this.adopt(order, stored).await;
            }
            this.catalog()
        }
    }

    pub fn reload_waiting(&self) -> bool {
        self.unread.load(Ordering::SeqCst) > 0
    }

    pub fn definition(
        &self,
        id: String,
    ) -> impl Future<Output = Result<ActionDefinition, ActionStoreError>> + use<> {
        let (_, read) = self.queue(move |store| store.definition(&id));
        async move { answered(read.await.ok()) }
    }

    pub fn read(
        &self,
        id: String,
    ) -> impl Future<Output = Result<ActionWriteRequest, ActionStoreError>> + use<> {
        let (_, read) = self.queue(move |store| store.read(&id));
        async move { answered(read.await.ok()) }
    }

    pub fn create(
        self: &Rc<Self>,
        request: ActionWriteRequest,
    ) -> impl Future<Output = Result<(), ActionStoreError>> + use<> {
        self.change(move |store| store.create(&request))
    }

    pub fn write(
        self: &Rc<Self>,
        request: ActionWriteRequest,
    ) -> impl Future<Output = Result<(), ActionStoreError>> + use<> {
        self.change(move |store| store.write(&request))
    }

    pub fn set_enabled(
        self: &Rc<Self>,
        id: String,
        enabled: bool,
    ) -> impl Future<Output = Result<(), ActionStoreError>> + use<> {
        self.change(move |store| {
            let mut definition = store.definition(&id)?;
            definition.enabled = enabled;
            store.write(&ActionWriteRequest {
                definition,
                script: None,
            })
        })
    }

    pub fn delete(
        self: &Rc<Self>,
        id: String,
    ) -> impl Future<Output = Result<(), ActionStoreError>> + use<> {
        self.change(move |store| store.delete(&id))
    }

    pub fn duplicate(
        self: &Rc<Self>,
        id: String,
        name: String,
    ) -> impl Future<Output = Result<String, ActionStoreError>> + use<> {
        self.change(move |store| {
            let mut copy = store.read(&id)?;
            copy.definition.name = name;
            for candidate in copy_ids(&id) {
                copy.definition.id = candidate;
                match store.create(&copy) {
                    Err(ActionStoreError::AlreadyExists(_)) => {}
                    created => return created.map(|()| copy.definition.id),
                }
            }
            Err(ActionStoreError::Io(
                rust_i18n::t!("No free action name is available for “%{id}”", id = id).into_owned(),
            ))
        })
    }

    pub fn import(
        self: &Rc<Self>,
        source: PathBuf,
    ) -> impl Future<Output = Result<String, ActionStoreError>> + use<> {
        let store = self.store.clone();
        let read = outside(move || store.read_import(&source));
        let this = self.clone();
        async move {
            let request = answered(read.await)?;
            this.change(move |store| store.import(request)).await
        }
    }

    pub fn export(
        &self,
        id: String,
        destination: PathBuf,
    ) -> impl Future<Output = Result<PathBuf, ActionStoreError>> + use<> {
        let read = self.read(id);
        let store = self.store.clone();
        async move {
            let request = read.await?;
            answered(outside(move || store.export(&request, &destination)).await)
        }
    }

    pub(crate) fn observe(&self, callback: Rc<dyn Fn()>) -> ListenerGuard<Rc<dyn Fn()>> {
        self.listeners.add(callback)
    }

    fn change<T, C>(
        self: &Rc<Self>,
        change: C,
    ) -> impl Future<Output = Result<T, ActionStoreError>> + use<T, C>
    where
        T: Send + 'static,
        C: FnOnce(&dyn ActionStore) -> Result<T, ActionStoreError> + Send + 'static,
    {
        let (order, changed) = self.queue(move |store| {
            let changed = change(store)?;
            Ok((changed, store.load()))
        });
        let this = self.clone();
        async move {
            let (changed, stored) = answered(changed.await.ok())?;
            this.adopt(order, stored).await;
            Ok(changed)
        }
    }

    fn queue<T: Send + 'static>(
        &self,
        job: impl FnOnce(&dyn ActionStore) -> T + Send + 'static,
    ) -> (u64, oneshot::Receiver<T>) {
        let order = self.queued.get() + 1;
        self.queued.set(order);
        let store = self.store.clone();
        (order, crate::storage::writer::run(move || job(&*store)))
    }

    async fn adopt(&self, order: u64, stored: StoredActions) {
        let store = self.store.clone();
        let Some(located) = outside(move || store.locate(stored)).await else {
            return;
        };
        if self.adopted.get() > order {
            return;
        }
        self.adopted.set(order);
        let catalog = ActionCatalog::stored(located);
        if catalog_equivalent(&self.catalog.borrow(), &catalog) {
            return;
        }
        self.catalog.replace(Rc::new(catalog));
        self.notify();
    }

    fn notify(&self) {
        self.listeners.notify(|callback| callback());
    }
}

fn outside<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Option<T>> {
    let (answer, answered) = oneshot::channel();
    let started = std::thread::Builder::new()
        .name("strata-actions".to_owned())
        .spawn(move || {
            let _ = answer.send(job());
        })
        .is_ok();
    async move {
        if !started {
            return None;
        }
        answered.await.ok()
    }
}

fn copy_ids(id: &str) -> impl Iterator<Item = String> + use<'_> {
    (0..1000)
        .map(move |suffix| {
            let suffix = if suffix == 0 {
                "-copy".to_owned()
            } else {
                format!("-copy-{suffix}")
            };
            let stem: String = id
                .chars()
                .take(crate::model::MAX_ACTION_ID_CHARS - suffix.len())
                .collect();
            format!("{stem}{suffix}")
        })
        .filter(move |candidate| candidate != id)
}

fn answered<T>(answer: Option<Result<T, ActionStoreError>>) -> Result<T, ActionStoreError> {
    answer.unwrap_or_else(|| {
        Err(ActionStoreError::Io(crate::i18n::tr(
            super::UNEXPECTED_SYSTEM_ERROR,
        )))
    })
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

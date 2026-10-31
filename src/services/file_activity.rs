// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt,
    fs::{self, File},
    future::Future,
    io::{self, Read},
    marker::PhantomData,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use futures_channel::oneshot;

use crate::model::Location;

pub(crate) const HELD_FOR_OPERATION: &str = "This item is being changed by a file operation.";

#[derive(Debug)]
pub(crate) struct Refused;

impl fmt::Display for Refused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(HELD_FOR_OPERATION)
    }
}

impl std::error::Error for Refused {}

impl From<Refused> for io::Error {
    fn from(refused: Refused) -> Self {
        Self::other(refused)
    }
}

impl From<Refused> for String {
    fn from(_: Refused) -> Self {
        HELD_FOR_OPERATION.to_owned()
    }
}

pub(crate) fn is_refusal(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<Refused>())
}

type Stop = Arc<dyn Fn() + Send + Sync>;

struct Reader {
    paths: Vec<PathBuf>,
    stop: Stop,
}

struct Barrier {
    paths: BTreeSet<PathBuf>,
    drained: Vec<oneshot::Sender<()>>,
}

impl Barrier {
    fn covers(&self, reader: &Reader) -> bool {
        reader.paths.iter().any(|path| {
            path.ancestors()
                .any(|ancestor| self.paths.contains(ancestor))
        })
    }
}

struct Counted(BTreeMap<PathBuf, usize>);

impl Counted {
    const fn new() -> Self {
        Self(BTreeMap::new())
    }

    fn add(&mut self, path: &Path) {
        *self.0.entry(path.to_owned()).or_default() += 1;
    }

    fn remove(&mut self, path: &Path) {
        if let Some(count) = self.0.get_mut(path) {
            *count -= 1;
            if *count == 0 {
                self.0.remove(path);
            }
        }
    }

    fn contains(&self, path: &Path) -> bool {
        self.0.contains_key(path)
    }

    fn covers(&self, path: &Path) -> bool {
        path.ancestors().any(|ancestor| self.contains(ancestor))
    }
}

struct Registry {
    next: u64,
    readers: BTreeMap<u64, Reader>,
    operations: BTreeMap<u64, Registered>,
    barriers: BTreeMap<u64, Barrier>,
    roots: Counted,
    folders: Counted,
    trees: Counted,
    fenced: Counted,
}

impl Registry {
    const fn new() -> Self {
        Self {
            next: 0,
            readers: BTreeMap::new(),
            operations: BTreeMap::new(),
            barriers: BTreeMap::new(),
            roots: Counted::new(),
            folders: Counted::new(),
            trees: Counted::new(),
            fenced: Counted::new(),
        }
    }

    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn reads_under(&self, barrier: u64) -> bool {
        self.barriers
            .get(&barrier)
            .is_some_and(|barrier| self.readers.values().any(|reader| barrier.covers(reader)))
    }

    fn take_drained(&mut self) -> Vec<oneshot::Sender<()>> {
        let drained: Vec<u64> = self
            .barriers
            .iter()
            .filter(|(_, barrier)| !barrier.drained.is_empty())
            .map(|(id, _)| *id)
            .filter(|id| !self.reads_under(*id))
            .collect();
        let mut senders = Vec::new();
        for id in drained {
            if let Some(barrier) = self.barriers.get_mut(&id) {
                senders.append(&mut barrier.drained);
            }
        }
        senders
    }

    fn register(&mut self, scope: &OperationScope) {
        for root in &scope.roots {
            self.roots.add(root);
        }
        for folder in &scope.folders {
            self.folders.add(folder);
        }
    }
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry::new());

fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct Hold {
    id: u64,
    stopped: Arc<AtomicBool>,
}

pub(crate) fn hold<P: AsRef<Path>>(
    paths: impl IntoIterator<Item = P>,
    stop: impl Fn() + Send + Sync + 'static,
) -> Result<Hold, Refused> {
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .map(|path| path.as_ref().to_owned())
        .collect();
    let mut registry = registry();
    if paths.iter().any(|path| registry.fenced.covers(path)) {
        return Err(Refused);
    }
    let stopped = Arc::new(AtomicBool::new(false));
    let flag = stopped.clone();
    let id = registry.id();
    registry.readers.insert(
        id,
        Reader {
            paths,
            stop: Arc::new(move || {
                flag.store(true, Ordering::Release);
                stop();
            }),
        },
    );
    Ok(Hold { id, stopped })
}

impl Hold {
    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    pub(crate) fn settle<T, E: From<Refused>>(&self, result: Result<T, E>) -> Result<T, E> {
        match result {
            Err(_) if self.stopped() => Err(Refused.into()),
            result => result,
        }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let drained = {
            let mut registry = registry();
            registry.readers.remove(&self.id);
            registry.take_drained()
        };
        for sender in drained {
            let _ = sender.send(());
        }
    }
}

fn hold_real(path: &Path) -> io::Result<Hold> {
    let real = path.canonicalize()?;
    Ok(hold([path, &real], || {})?)
}

pub(crate) struct HeldFile {
    file: File,
    hold: Hold,
}

pub(crate) fn open(path: &Path) -> io::Result<HeldFile> {
    crate::util::ui_thread::assert_off();
    let hold = hold_real(path)?;
    Ok(HeldFile {
        file: File::open(path)?,
        hold,
    })
}

impl Read for HeldFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.hold.stopped() {
            return Err(Refused.into());
        }
        self.file.read(buffer)
    }
}

pub(crate) struct Listing {
    entries: Option<fs::ReadDir>,
    hold: Hold,
}

pub(crate) fn list(folder: &Path) -> io::Result<Listing> {
    let hold = hold_real(folder)?;
    Ok(Listing {
        entries: Some(fs::read_dir(folder)?),
        hold,
    })
}

impl Listing {
    pub(crate) fn stopped(&self) -> bool {
        self.hold.stopped()
    }
}

impl Iterator for Listing {
    type Item = io::Result<fs::DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        let entries = self.entries.as_mut()?;
        if self.hold.stopped() {
            self.entries = None;
            return Some(Err(Refused.into()));
        }
        entries.next()
    }
}

#[derive(Default)]
pub(crate) struct OperationScope {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) folders: Vec<PathBuf>,
}

struct Registered {
    scope: OperationScope,
    trees: Vec<PathBuf>,
}

fn folder_key(folder: &Location) -> Option<PathBuf> {
    folder
        .native_path()
        .map(Path::to_path_buf)
        .or_else(|| folder.uri_value().map(PathBuf::from))
}

pub(crate) struct Activity(u64);

pub(crate) fn begin(scope: OperationScope) -> Activity {
    let mut registry = registry();
    let id = registry.id();
    registry.register(&scope);
    registry.operations.insert(
        id,
        Registered {
            scope,
            trees: Vec::new(),
        },
    );
    Activity(id)
}

impl Activity {
    pub(crate) fn include(&self, scope: OperationScope) {
        let mut registry = registry();
        registry.register(&scope);
        if let Some(operation) = registry.operations.get_mut(&self.0) {
            operation.scope.roots.extend(scope.roots);
            operation.scope.folders.extend(scope.folders);
        }
    }

    pub(crate) fn include_folder(&self, folder: &Location) {
        if let Some(folder) = folder_key(folder) {
            self.include(OperationScope {
                roots: Vec::new(),
                folders: vec![folder],
            });
        }
    }

    pub(crate) fn include_trees(&self, trees: impl IntoIterator<Item = Location>) {
        let trees: Vec<PathBuf> = trees
            .into_iter()
            .filter_map(|tree| folder_key(&tree))
            .collect();
        let mut registry = registry();
        for tree in &trees {
            registry.trees.add(tree);
        }
        if let Some(operation) = registry.operations.get_mut(&self.0) {
            operation.trees.extend(trees);
        }
    }

    pub(crate) fn fence<P: Into<PathBuf>>(&self, paths: impl IntoIterator<Item = P>) -> Fence {
        let paths: BTreeSet<PathBuf> = paths.into_iter().map(Into::into).collect();
        let mut registry = registry();
        for path in &paths {
            registry.fenced.add(path);
        }
        let id = registry.id();
        registry.barriers.insert(
            id,
            Barrier {
                paths,
                drained: Vec::new(),
            },
        );
        Fence(id)
    }
}

impl Activity {
    pub(crate) fn end(&self) {
        let mut registry = registry();
        let Some(Registered { scope, trees }) = registry.operations.remove(&self.0) else {
            return;
        };
        for root in &scope.roots {
            registry.roots.remove(root);
        }
        for folder in &scope.folders {
            registry.folders.remove(folder);
        }
        for tree in &trees {
            registry.trees.remove(tree);
        }
        drop(registry);
        let ended = EndedOperation {
            roots: scope.roots.into_iter().collect(),
            folders: scope.folders.into_iter().collect(),
            trees: trees.into_iter().collect(),
        };
        notify(&Notice::Ended(&ended));
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        self.end();
    }
}

pub(crate) struct Fence(u64);

impl Fence {
    pub(crate) fn stop_reads(&self) {
        let stops: Vec<Stop> = {
            let registry = registry();
            let Some(barrier) = registry.barriers.get(&self.0) else {
                return;
            };
            registry
                .readers
                .values()
                .filter(|reader| barrier.covers(reader))
                .map(|reader| reader.stop.clone())
                .collect()
        };
        for stop in stops {
            stop();
        }
    }

    pub(crate) fn drained(&self) -> impl Future<Output = ()> + Send + 'static {
        let (sender, receiver) = oneshot::channel();
        let settled = {
            let mut registry = registry();
            let open = registry.reads_under(self.0);
            match registry.barriers.get_mut(&self.0) {
                Some(barrier) if open => {
                    barrier.drained.push(sender);
                    None
                }
                _ => Some(sender),
            }
        };
        if let Some(sender) = settled {
            let _ = sender.send(());
        }
        async move {
            let _ = receiver.await;
        }
    }
}

impl Drop for Fence {
    fn drop(&mut self) {
        let mut registry = registry();
        if let Some(barrier) = registry.barriers.remove(&self.0) {
            for path in &barrier.paths {
                registry.fenced.remove(path);
            }
        }
    }
}

pub(crate) fn is_changing(path: &Path) -> bool {
    let registry = registry();
    registry.roots.covers(path) || registry.fenced.covers(path)
}

pub(crate) fn is_releasing() -> bool {
    !registry().barriers.is_empty()
}

/// Direct writes and recursive merges stay distinct so consumers can leave unrelated
/// descendants alone when invalidating metadata.
pub(crate) struct EndedOperation {
    pub(crate) roots: HashSet<PathBuf>,
    pub(crate) folders: HashSet<PathBuf>,
    pub(crate) trees: HashSet<PathBuf>,
}

pub(crate) enum Notice<'a> {
    Releasing(&'a [PathBuf]),
    Lifted(&'a [PathBuf]),
    Ended(&'a EndedOperation),
}

impl Notice<'_> {
    pub(crate) fn covers(&self, path: &Path) -> bool {
        match self {
            Self::Releasing(roots) | Self::Lifted(roots) => {
                roots.iter().any(|root| path.starts_with(root))
            }
            Self::Ended(ended) => path.ancestors().any(|parent| {
                ended.roots.contains(parent)
                    || ended.folders.contains(parent)
                    || ended.trees.contains(parent)
            }),
        }
    }
}

type Observer = Rc<dyn Fn(&Notice<'_>)>;

thread_local! {
    static OBSERVERS: RefCell<BTreeMap<u64, Observer>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_OBSERVER: Cell<u64> = const { Cell::new(0) };
}

pub(crate) struct Subscription(u64, PhantomData<Rc<()>>);

pub(crate) fn observe(observer: impl Fn(&Notice<'_>) + 'static) -> Subscription {
    let id = NEXT_OBSERVER.with(|next| {
        next.set(next.get() + 1);
        next.get()
    });
    OBSERVERS.with_borrow_mut(|observers| observers.insert(id, Rc::new(observer)));
    Subscription(id, PhantomData)
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = OBSERVERS.try_with(|observers| observers.borrow_mut().remove(&self.0));
    }
}

pub(crate) fn notify(notice: &Notice<'_>) {
    let observers: Vec<Observer> = OBSERVERS
        .try_with(|observers| observers.borrow().values().cloned().collect())
        .unwrap_or_default();
    for observer in observers {
        observer(notice);
    }
}

pub(crate) fn is_active_in(folder: &Path) -> bool {
    let registry = registry();
    registry.folders.contains(folder) || registry.trees.covers(folder)
}

pub(crate) fn is_active_at(folder: &Location) -> bool {
    folder_key(folder).is_some_and(|folder| is_active_in(&folder))
}

pub(crate) fn in_use_within(folder: &Path) -> bool {
    let below = |path: &Path| path != folder && path.starts_with(folder);
    let registry = registry();
    registry
        .readers
        .values()
        .any(|reader| reader.paths.iter().any(|path| below(path)))
        || registry.operations.values().any(|operation| {
            let roots = &operation.scope.roots;
            roots.iter().any(|root| below(root))
                && !roots.iter().any(|root| folder.starts_with(root))
        })
}

#[cfg(test)]
mod tests;

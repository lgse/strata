// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

pub(crate) const MAX_SCANNED_ENTRIES: usize = 16_384;
pub(crate) const MAX_MATCHING_FOLDERS: usize = 1_024;
const BATCH_SIZE: i32 = 256;

pub(crate) type LocalFuture<T> = Pin<Box<dyn Future<Output = T>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Unreadable;

#[derive(Clone, Debug)]
pub(crate) struct Child {
    pub(crate) name: OsString,
    pub(crate) folder: bool,
}

pub(crate) trait FolderSource {
    fn open(&self, directory: &Path) -> LocalFuture<Result<Box<dyn Enumeration>, Unreadable>>;
}

pub(crate) trait Enumeration {
    fn next_batch(&self) -> LocalFuture<Result<Vec<Child>, Unreadable>>;
}

/// Asynchronous GIO enumeration. Dropping a pending future cancels its I/O.
pub(crate) struct GioFolders;

impl FolderSource for GioFolders {
    fn open(&self, directory: &Path) -> LocalFuture<Result<Box<dyn Enumeration>, Unreadable>> {
        let file = gio::File::for_path(directory);
        Box::pin(async move {
            let enumerator = file
                .enumerate_children_future(
                    "standard::name,standard::type",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
                .map_err(|_| Unreadable)?;
            Ok(Box::new(GioEnumeration(enumerator)) as Box<dyn Enumeration>)
        })
    }
}

struct GioEnumeration(gio::FileEnumerator);

impl Enumeration for GioEnumeration {
    fn next_batch(&self) -> LocalFuture<Result<Vec<Child>, Unreadable>> {
        let enumerator = self.0.clone();
        Box::pin(async move {
            let infos = enumerator
                .next_files_future(BATCH_SIZE, glib::Priority::DEFAULT)
                .await
                .map_err(|_| Unreadable)?;
            Ok(infos
                .into_iter()
                .map(|info| Child {
                    name: info.name().into_os_string(),
                    folder: info.file_type() == gio::FileType::Directory,
                })
                .collect())
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Listing {
        prefix: String,
    },
    /// Folders in `directory` whose names start with `prefix`. A completion
    /// is `stem` + name + `/`, so the typed form (relative, `~`) is kept.
    Folder {
        directory: PathBuf,
        stem: String,
        prefix: String,
    },
}

pub(crate) fn scope(text: &str, current: Option<&Path>, home: &Path) -> Option<Scope> {
    if looks_like_uri(text) {
        return None;
    }
    if text == "~" {
        return Some(Scope::Folder {
            directory: home.to_path_buf(),
            stem: "~/".to_owned(),
            prefix: String::new(),
        });
    }
    let Some(slash) = text.rfind('/') else {
        // `~name` is another user's home, which navigation rejects.
        return (!text.starts_with('~')).then(|| Scope::Listing {
            prefix: text.to_owned(),
        });
    };
    let (stem, prefix) = (&text[..=slash], &text[slash + 1..]);
    let directory = if let Some(relative) = stem.strip_prefix("~/") {
        home.join(relative)
    } else if stem.starts_with('~') {
        return None;
    } else if stem.starts_with('/') {
        PathBuf::from(stem)
    } else {
        current?.join(stem)
    };
    Some(Scope::Folder {
        directory,
        stem: stem.to_owned(),
        prefix: prefix.to_owned(),
    })
}

pub(crate) fn looks_like_uri(text: &str) -> bool {
    text.starts_with("//")
        || text.starts_with('\\')
        || text
            .split('/')
            .next()
            .is_some_and(|first| first.contains(':'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hint {
    NoMatch,
    Unreadable,
    TooMany,
    Uri,
    OtherHome,
}

impl Hint {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::NoMatch => "No matching folders",
            Self::Unreadable => "Can\u{2019}t read that folder \u{2014} check the path",
            Self::TooMany => "Too many entries \u{2014} refine the path",
            Self::Uri => "URIs are not completed",
            Self::OtherHome => "Only ~ and ~/ are supported",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Complete {
        text: String,
        index: usize,
        count: usize,
    },
    Pending,
    Hint(Hint),
}

struct Matches {
    prefix: String,
    include_hidden: bool,
    scan_limit: Option<usize>,
    scanned: usize,
    names: Vec<String>,
}

impl Matches {
    fn new(prefix: &str, include_hidden: bool, scan_limit: Option<usize>) -> Self {
        Self {
            prefix: prefix.to_lowercase(),
            include_hidden: include_hidden || prefix.starts_with('.'),
            scan_limit,
            scanned: 0,
            names: Vec::new(),
        }
    }

    fn offer(&mut self, name: &OsString, folder: bool) -> Result<(), Hint> {
        self.scanned += 1;
        if self.scan_limit.is_some_and(|limit| self.scanned > limit) {
            return Err(Hint::TooMany);
        }
        // A name that is not UTF-8 could not be typed back exactly.
        let Some(name) = name.to_str().filter(|_| folder) else {
            return Ok(());
        };
        if (!self.include_hidden && name.starts_with('.'))
            || !name.to_lowercase().starts_with(&self.prefix)
        {
            return Ok(());
        }
        if self.names.len() == MAX_MATCHING_FOLDERS {
            return Err(Hint::TooMany);
        }
        self.names.push(name.to_owned());
        Ok(())
    }

    fn finish(mut self) -> Vec<String> {
        self.names.sort_by(|left, right| {
            left.to_lowercase()
                .cmp(&right.to_lowercase())
                .then_with(|| left.cmp(right))
        });
        self.names
    }
}

async fn enumerate(
    source: Rc<dyn FolderSource>,
    directory: PathBuf,
    mut matches: Matches,
) -> Result<Vec<String>, Hint> {
    let enumeration = source
        .open(&directory)
        .await
        .map_err(|_| Hint::Unreadable)?;
    loop {
        let batch = enumeration
            .next_batch()
            .await
            .map_err(|_| Hint::Unreadable)?;
        if batch.is_empty() {
            return Ok(matches.finish());
        }
        for child in &batch {
            matches.offer(&child.name, child.folder)?;
        }
    }
}

pub(crate) struct Context<'a> {
    pub(crate) current: Option<&'a Path>,
    pub(crate) home: &'a Path,
    pub(crate) show_hidden: bool,
    /// The folder names the open listing shows, plus its hidden folders when
    /// asked.
    pub(crate) listing: &'a dyn Fn(bool) -> Vec<OsString>,
}

struct Cycle {
    shown: String,
    stem: String,
    names: Vec<String>,
    index: usize,
}

impl Cycle {
    fn step(&self) -> Step {
        Step::Complete {
            text: self.shown.clone(),
            index: self.index,
            count: self.names.len(),
        }
    }

    fn select(&mut self, index: usize) -> Step {
        self.index = index;
        self.shown = format!("{}{}/", self.stem, self.names[index]);
        self.step()
    }
}

#[derive(Default)]
struct State {
    generation: Cell<u64>,
    pending: RefCell<Option<glib::JoinHandle<()>>>,
    cycle: RefCell<Option<Cycle>>,
}

impl State {
    fn start_cycle(&self, stem: String, names: Vec<String>, backward: bool) -> Step {
        if names.is_empty() {
            return Step::Hint(Hint::NoMatch);
        }
        let index = if backward { names.len() - 1 } else { 0 };
        let mut cycle = Cycle {
            shown: String::new(),
            stem,
            names,
            index,
        };
        let step = cycle.select(index);
        self.cycle.replace(Some(cycle));
        step
    }
}

#[derive(Clone)]
pub(crate) struct GoCompletion {
    source: Rc<dyn FolderSource>,
    state: Rc<State>,
}

impl GoCompletion {
    pub(crate) fn new(source: Rc<dyn FolderSource>) -> Self {
        Self {
            source,
            state: Rc::default(),
        }
    }

    /// **Tab**, or **Shift+Tab** when `backward`. When enumeration is needed
    /// this returns [`Step::Pending`] and later calls `deliver` once, unless
    /// [`Self::invalidate`] runs first. `deliver` should hold only weak
    /// references so pending work never keeps a closed window alive.
    pub(crate) fn step(
        &self,
        text: &str,
        backward: bool,
        context: &Context<'_>,
        deliver: impl FnOnce(Step) + 'static,
    ) -> Step {
        if let Some(cycle) = self.state.cycle.borrow_mut().as_mut()
            && cycle.shown == text
        {
            let count = cycle.names.len();
            let index = if backward {
                (cycle.index + count - 1) % count
            } else {
                (cycle.index + 1) % count
            };
            return cycle.select(index);
        }
        if self.is_pending() {
            return Step::Pending;
        }
        self.invalidate();
        let Some(scope) = scope(text, context.current, context.home) else {
            return Step::Hint(if looks_like_uri(text) {
                Hint::Uri
            } else if text.starts_with('~') {
                Hint::OtherHome
            } else {
                Hint::NoMatch
            });
        };
        match scope {
            Scope::Listing { prefix } => {
                let mut matches = Matches::new(&prefix, true, None);
                for name in (context.listing)(prefix.starts_with('.')) {
                    if let Err(hint) = matches.offer(&name, true) {
                        return Step::Hint(hint);
                    }
                }
                self.state
                    .start_cycle(String::new(), matches.finish(), backward)
            }
            Scope::Folder {
                directory,
                stem,
                prefix,
            } => {
                let matches = Matches::new(&prefix, context.show_hidden, Some(MAX_SCANNED_ENTRIES));
                let generation = self.state.generation.get();
                let state = Rc::downgrade(&self.state);
                let work = enumerate(self.source.clone(), directory, matches);
                let task = glib::MainContext::default().spawn_local(async move {
                    let result = work.await;
                    let Some(state) = state.upgrade() else {
                        return;
                    };
                    if state.generation.get() != generation {
                        return;
                    }
                    state.pending.take();
                    deliver(match result {
                        Ok(names) => state.start_cycle(stem, names, backward),
                        Err(hint) => Step::Hint(hint),
                    });
                });
                self.state.pending.replace(Some(task));
                Step::Pending
            }
        }
    }

    pub(crate) fn invalidate(&self) {
        self.state
            .generation
            .set(self.state.generation.get().wrapping_add(1));
        self.state.cycle.take();
        if let Some(task) = self.state.pending.take() {
            task.abort();
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.state.pending.borrow().is_some()
    }
}

#[cfg(test)]
pub(crate) mod tests;

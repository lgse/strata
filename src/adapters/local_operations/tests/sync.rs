// SPDX-License-Identifier: MIT

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use rustix::io::Errno;

use super::super::{
    MountFlushHint, SyncDestination, owed::Limits, removable_sync_roots, sync_filesystem,
};
use super::fs_quirks::{Call, Op, Racer, Support, Trash};
use super::*;
use crate::services::{KeptCause, KeptItem};

mod barriers;
mod batches;
mod flush;
mod roots;

type Before = (Op, &'static str, usize, Racer);

/// A `sources` folder and a `destination` folder beneath one root, and the
/// names moved between them.
struct Layout {
    sources: PathBuf,
    destination: PathBuf,
    names: Vec<String>,
}

impl Layout {
    fn create(root: &Path, names: impl IntoIterator<Item = impl AsRef<str>>) -> io::Result<Self> {
        let layout = Self {
            sources: root.join("sources"),
            destination: root.join("destination"),
            names: names
                .into_iter()
                .map(|name| name.as_ref().to_owned())
                .collect(),
        };
        fs::create_dir(&layout.sources)?;
        fs::create_dir(&layout.destination)?;
        Ok(layout)
    }

    /// Writes each named source with the contents `of` its name.
    fn write_sources(&self, of: impl Fn(&str) -> Vec<u8>) -> io::Result<()> {
        for name in &self.names {
            fs::write(self.sources.join(name), of(name))?;
        }
        Ok(())
    }

    fn names(&self) -> Vec<&str> {
        self.names.iter().map(String::as_str).collect()
    }

    fn paths(&self) -> Vec<PathBuf> {
        self.names
            .iter()
            .map(|name| self.sources.join(name))
            .collect()
    }

    fn move_all(&self, id: u64, barriers: &Barriers) -> Moved {
        let sources = self.paths();
        paste_moves(
            Moving {
                id,
                destination: &self.destination,
                sources: &sources,
                cancel_after: None,
            },
            barriers,
        )
    }
}

#[derive(Debug, PartialEq)]
struct Seen {
    deleted: Vec<String>,
    copied: Vec<String>,
    flushing: bool,
    counted: usize,
}

fn seen(deleted: &[&str], copied: &[&str], flushing: bool, counted: usize) -> Seen {
    Seen {
        deleted: strings(deleted),
        copied: strings(copied),
        flushing,
        counted,
    }
}

struct Barriers {
    seen: Arc<std::sync::Mutex<Vec<Seen>>>,
    flushing: Arc<AtomicBool>,
    counted: Arc<AtomicUsize>,
}

impl Barriers {
    fn new() -> Self {
        Self {
            seen: Arc::default(),
            flushing: Arc::default(),
            counted: Arc::default(),
        }
    }

    /// A racer before each barrier of the layout's names, the first of which
    /// runs `also` first.
    fn racers(&self, layout: &Layout, mut also: Option<Racer>) -> Vec<Before> {
        (0..layout.names.len())
            .map(|_| {
                let watch = self.watcher(layout);
                let mut also = also.take();
                let racer: Racer = Box::new(move || {
                    if let Some(also) = &mut also {
                        also();
                    }
                    watch();
                });
                (Op::SyncFs, "*", 0, racer)
            })
            .collect()
    }

    /// Records which names are gone from the sources and which stand at the
    /// destination, with the flush and count shown so far.
    fn watcher(&self, layout: &Layout) -> impl Fn() + Send + 'static {
        let (seen, flushing, counted) = (
            self.seen.clone(),
            self.flushing.clone(),
            self.counted.clone(),
        );
        let (sources, destination) = (layout.sources.clone(), layout.destination.clone());
        let names = layout.names.clone();
        move || {
            let deleted = names
                .iter()
                .filter(|name| !sources.join(name).exists())
                .cloned()
                .collect();
            let copied = names
                .iter()
                .filter(|name| destination.join(name).exists())
                .cloned()
                .collect();
            seen.lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(Seen {
                    deleted,
                    copied,
                    flushing: flushing.load(Ordering::SeqCst),
                    counted: counted.load(Ordering::SeqCst),
                });
        }
    }

    fn seen(&self) -> Vec<Seen> {
        std::mem::take(&mut *self.seen.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

struct Moving<'a> {
    id: u64,
    destination: &'a Path,
    sources: &'a [PathBuf],
    /// Drops the operation once this many items are reported completed.
    cancel_after: Option<usize>,
}

/// A finished move: its handle, every event, and the sources reported moved
/// while they still existed.
struct Moved {
    _operation: Rc<RefCell<Option<LoadHandle>>>,
    events: Rc<RefCell<Vec<OperationEvent>>>,
    early: Rc<RefCell<Vec<Location>>>,
}

impl Moved {
    fn events(&self) -> std::cell::Ref<'_, Vec<OperationEvent>> {
        self.events.borrow()
    }

    fn moved_count(&self) -> usize {
        moved_count(&self.events())
    }

    /// `count` items were reported moved, none before its source was gone.
    fn assert_moved_after_deletion(&self, count: usize, row: &str) {
        assert_eq!(self.moved_count(), count, "{row}");
        assert!(
            self.early.borrow().is_empty(),
            "{row}: {:?}",
            self.early.borrow()
        );
    }
}

fn paste_moves(moving: Moving<'_>, barriers: &Barriers) -> Moved {
    let events = Rc::new(RefCell::new(Vec::new()));
    let early = Rc::new(RefCell::new(Vec::new()));
    let handle = Rc::new(RefCell::new(None));
    let (emitted, reported_early, slot) = (events.clone(), early.clone(), handle.clone());
    let (flushing, counted) = (barriers.flushing.clone(), barriers.counted.clone());
    let cancel_after = moving.cancel_after;
    let items = moving
        .sources
        .iter()
        .map(|source| item(source, TransferConflict::FailIfExists))
        .collect();
    let operation = LocalOperationProvider.paste(
        request(moving.id, moving.destination, items, true),
        Rc::new(move |event| {
            match &event {
                OperationEvent::FlushingToDevice { .. } => {
                    flushing.store(true, Ordering::SeqCst);
                }
                OperationEvent::ItemMoved { from, .. }
                    if from.native_path().is_some_and(Path::exists) =>
                {
                    reported_early.borrow_mut().push(from.clone());
                }
                OperationEvent::TransferProgress {
                    completed_items, ..
                } => {
                    counted.store(*completed_items, Ordering::SeqCst);
                    if Some(*completed_items) == cancel_after {
                        slot.borrow_mut().take();
                    }
                }
                _ => {}
            }
            emitted.borrow_mut().push(event);
        }),
    );
    *handle.borrow_mut() = Some(operation);
    pump_until_transfer(&events);
    Moved {
        _operation: handle,
        events,
        early,
    }
}

fn item(source: &Path, conflict: TransferConflict) -> PasteItem {
    PasteItem {
        source: Location::local(source),
        conflict,
        target_name: None,
    }
}

fn request(id: u64, destination: &Path, items: Vec<PasteItem>, move_sources: bool) -> PasteRequest {
    PasteRequest {
        id: OperationRequestId(id),
        destination: Location::local(destination),
        items,
        move_sources,
    }
}

/// Pastes and pumps until the transfer settles, keeping its handle alive.
fn paste_pumped(request: PasteRequest) -> (LoadHandle, Vec<OperationEvent>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.paste(
        request,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    pump_until_transfer(&events);
    (operation, events.take())
}

fn assert_pasted(events: &[OperationEvent], row: &str) {
    assert!(
        matches!(
            terminal_transfer(events),
            Some(OperationEvent::Pasted { .. })
        ),
        "{row}: {events:?}"
    );
}

fn moved_count(events: &[OperationEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, OperationEvent::ItemMoved { .. }))
        .count()
}

fn kept_items(events: &[OperationEvent]) -> Vec<KeptItem> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemsReported { report, .. } => Some(report.kept.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The calls another scope had made when a source was first deleted.
struct CallsAtFirstUnlink(Arc<std::sync::Mutex<Option<Vec<Call>>>>);

impl CallsAtFirstUnlink {
    /// Installs quirks on `sources` that record `read()` before its first unlink.
    fn install(
        sources: &Path,
        read: impl Fn() -> Vec<Call> + Send + Sync + 'static,
    ) -> (FsQuirks, Self) {
        let recorded = Arc::new(std::sync::Mutex::new(None));
        let record = recorded.clone();
        let quirks = FsQuirks::install(
            sources,
            Quirks {
                before: vec![(
                    Op::Unlink,
                    "*",
                    0,
                    Box::new(move || {
                        *record.lock().unwrap_or_else(|error| error.into_inner()) = Some(read());
                    }),
                )],
                ..Quirks::default()
            },
        );
        (quirks, Self(recorded))
    }

    fn take(&self) -> Result<Vec<Call>, Box<dyn Error>> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .ok_or("a source was deleted")?)
    }
}

struct LimitsGuard;

impl LimitsGuard {
    fn set(limits: Limits) -> Self {
        super::super::owed::set_batch_limits_for_test(Some(limits));
        Self
    }
}

impl Drop for LimitsGuard {
    fn drop(&mut self) {
        super::super::owed::set_batch_limits_for_test(None);
    }
}

const NO_LIMITS: Limits = Limits {
    items: usize::MAX,
    bytes: u64::MAX,
    age: Duration::MAX,
};

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

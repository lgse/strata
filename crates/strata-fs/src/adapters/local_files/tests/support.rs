// SPDX-License-Identifier: MIT

use std::task::{Poll, Waker};

use super::*;
use crate::test_support::MainContextGuard;

pub(super) type Events = Rc<RefCell<Vec<DirectoryEvent>>>;
pub(super) type Changes = Rc<RefCell<Vec<DirectoryChange>>>;
pub(super) type FinishedFlags = (bool, Option<bool>, Option<bool>);

pub(super) fn serial() -> MainContextGuard<'static> {
    ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned")
}

pub(super) fn drain_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() && Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        done(),
        "the awaited event did not arrive within the deadline"
    );
}

pub(super) fn unique_fixture_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("strata-local-files-{label}-{unique}"))
}

/// A uniquely named directory that each test removes once it has what it needs.
pub(super) struct FixtureDir(PathBuf);

impl FixtureDir {
    pub(super) fn new(label: &str) -> Self {
        let root = unique_fixture_root(label);
        fs::create_dir_all(&root).expect("the fixture directory should be created");
        Self(root)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }

    pub(super) fn location(&self) -> Location {
        Location::local(&self.0)
    }

    pub(super) fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, contents).expect("the fixture file should be written");
        path
    }

    pub(super) fn write_numbered(&self, count: usize) {
        for index in 0..count {
            self.write(&format!("file-{index}.txt"), b"content");
        }
    }

    pub(super) fn remove(self) {
        fs::remove_dir_all(&self.0).expect("the fixture directory should be removed");
    }
}

pub(super) fn directory_request(location: Location) -> DirectoryRequest {
    DirectoryRequest {
        id: RequestId(1),
        location,
        batch_size: 64,
        include_metadata: false,
        max_entries: 100,
        time_budget: Duration::from_secs(10),
        background: false,
    }
}

pub(super) fn listing_request(path: &Path, max_entries: usize) -> DirectoryRequest {
    DirectoryRequest {
        max_entries,
        ..directory_request(Location::local(path))
    }
}

pub(super) fn background_request(folder: &Path) -> DirectoryRequest {
    DirectoryRequest {
        background: true,
        ..listing_request(folder, 100)
    }
}

pub(super) fn metadata_request(entries: Vec<Location>) -> MetadataRequest {
    MetadataRequest {
        id: RequestId(1),
        entries,
        full: false,
        include_icon_details: false,
        time_budget: Duration::from_secs(10),
    }
}

pub(super) fn icon_details_request(id: u64, path: &Path) -> MetadataRequest {
    MetadataRequest {
        id: RequestId(id),
        include_icon_details: true,
        ..metadata_request(vec![Location::local(path)])
    }
}

fn ends_load(event: &DirectoryEvent) -> bool {
    matches!(
        event,
        DirectoryEvent::Finished { .. } | DirectoryEvent::Failed { .. }
    )
}

fn ends_fill(event: &DirectoryEvent) -> bool {
    matches!(event, DirectoryEvent::MetadataFinished { .. })
}

/// `enumerate()` spawns its work on `glib::MainContext::default()` internally (not whatever
/// context happens to be thread-default), so it can only be driven via that same shared context
/// -- a private context pushed as thread-default would never see the spawned task at all. Bridge
/// the callback-based API into a future with `poll_fn` and drive it with `block_on`. The shared
/// lock is still required: concurrent `block_on(default())` calls from different test-harness
/// threads panic with a GLib thread-affinity error, same as concurrent `spawn_local`/`iteration()`
/// would.
fn run_until(
    ends: fn(&DirectoryEvent) -> bool,
    start: impl FnOnce(Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle,
) -> Vec<DirectoryEvent> {
    let _serial = serial();
    glib::MainContext::default().block_on(async move {
        let events: Events = Rc::default();
        let waker: Rc<RefCell<Option<Waker>>> = Rc::default();
        let collected = events.clone();
        let collected_waker = waker.clone();
        let emit: Rc<dyn Fn(DirectoryEvent)> = Rc::new(move |event| {
            let is_terminal = ends(&event);
            collected.borrow_mut().push(event);
            if is_terminal && let Some(waker) = collected_waker.borrow_mut().take() {
                waker.wake();
            }
        });
        let handle = start(emit);
        std::future::poll_fn(|cx| {
            if events.borrow().iter().any(ends) {
                Poll::Ready(())
            } else {
                *waker.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await;
        drop(handle);
        events.borrow().clone()
    })
}

pub(super) fn run_events(
    start: impl FnOnce(Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle,
) -> Vec<DirectoryEvent> {
    run_until(ends_load, start)
}

pub(super) fn run_enumerate(request: DirectoryRequest) -> Vec<DirectoryEvent> {
    run_events(|emit| LocalFileSource.enumerate(request, emit))
}

pub(super) fn run_fill(request: MetadataRequest) -> Vec<DirectoryEvent> {
    run_until(ends_fill, |emit| {
        LocalFileSource.fill_metadata(request, emit)
    })
}

pub(super) fn batch_sizes(events: &[DirectoryEvent]) -> Vec<usize> {
    events
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::Batch { entries, .. } => Some(entries.len()),
            _ => None,
        })
        .collect()
}

pub(super) fn batched_entry_count(events: &[DirectoryEvent]) -> usize {
    batch_sizes(events).into_iter().sum()
}

pub(super) fn batched_entries(events: &[DirectoryEvent]) -> Vec<FileEntry> {
    events
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::Batch { entries, .. } => Some(entries.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// `truncated`, `can_trash` and `can_delete` of the load's `Finished` event.
pub(super) fn finished_flags(events: &[DirectoryEvent]) -> Option<FinishedFlags> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::Finished {
            truncated,
            can_trash,
            can_delete,
            ..
        } => Some((*truncated, *can_trash, *can_delete)),
        _ => None,
    })
}

pub(super) fn finished_truncated(events: &[DirectoryEvent]) -> Option<bool> {
    finished_flags(events).map(|(truncated, ..)| truncated)
}

pub(super) fn failure_message(events: &[DirectoryEvent]) -> Option<&str> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::Failed { message, .. } => Some(message.as_str()),
        _ => None,
    })
}

pub(super) fn fill_outcome(events: &[DirectoryEvent]) -> Option<MetadataOutcome> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::MetadataFinished { outcome, .. } => Some(*outcome),
        _ => None,
    })
}

pub(super) fn fill_chunk_count(events: &[DirectoryEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, DirectoryEvent::MetadataFilled { .. }))
        .count()
}

pub(super) fn fill_summary(events: &[DirectoryEvent]) -> (Option<MetadataOutcome>, usize) {
    (fill_outcome(events), fill_chunk_count(events))
}

pub(super) fn filled_updates(events: &[DirectoryEvent]) -> impl Iterator<Item = &MetadataUpdate> {
    events
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::MetadataFilled { updates, .. } => Some(updates.iter()),
            _ => None,
        })
        .flatten()
}

fn first_update_field<'a, T>(
    mut events: impl Iterator<Item = &'a DirectoryEvent>,
    field: impl Fn(&MetadataUpdate) -> T,
) -> Option<T> {
    events.find_map(|event| match event {
        DirectoryEvent::MetadataFilled { updates, .. } => updates.first().map(&field),
        _ => None,
    })
}

/// `field` of the first update in the earliest fill chunk.
pub(super) fn first_filled<T>(
    events: &[DirectoryEvent],
    field: impl Fn(&MetadataUpdate) -> T,
) -> Option<T> {
    first_update_field(events.iter(), field)
}

/// `field` of the first update in the latest fill chunk.
pub(super) fn last_filled<T>(
    events: &[DirectoryEvent],
    field: impl Fn(&MetadataUpdate) -> T,
) -> Option<T> {
    first_update_field(events.iter().rev(), field)
}

pub(super) fn recorder(events: &Events) -> Rc<dyn Fn(DirectoryEvent)> {
    let events = events.clone();
    Rc::new(move |event| events.borrow_mut().push(event))
}

pub(super) fn change_recorder() -> (Changes, Rc<dyn Fn(DirectoryChange)>) {
    let changes: Changes = Rc::default();
    let collected = changes.clone();
    (
        changes,
        Rc::new(move |change| collected.borrow_mut().push(change)),
    )
}

pub(super) fn enumerate_into(events: &Events, request: DirectoryRequest) -> LoadHandle {
    LocalFileSource.enumerate(request, recorder(events))
}

pub(super) fn load_into(events: &Events, root: &Path, id: u64) -> LoadHandle {
    enumerate_into(
        events,
        DirectoryRequest {
            id: RequestId(id),
            time_budget: Duration::from_secs(30),
            ..directory_request(Location::local(root))
        },
    )
}

/// How load `id` ended: the entries it listed, or its failure message.
pub(super) fn outcome_of(events: &Events, id: u64) -> Option<Result<usize, String>> {
    let events = events.borrow();
    let listed = events
        .iter()
        .filter_map(|event| match event {
            DirectoryEvent::Batch {
                request_id,
                entries,
            } if request_id.0 == id => Some(entries.len()),
            _ => None,
        })
        .sum();
    events.iter().find_map(|event| match event {
        DirectoryEvent::Finished { request_id, .. } if request_id.0 == id => Some(Ok(listed)),
        DirectoryEvent::Failed {
            request_id,
            message,
            ..
        } if request_id.0 == id => Some(Err(message.clone())),
        _ => None,
    })
}

pub(super) fn folder_identity(folder: &Path) -> Result<ItemIdentity, String> {
    let stamp = crate::adapters::local_operations::folder_stamp(folder, false)
        .map_err(|error| error.to_string())?;
    directory_stamp(folder, &stamp, SystemTime::now())
        .identity
        .ok_or_else(|| "no identity".to_owned())
}

pub(super) fn assert_contains_all(text: &str, needles: &[&str]) {
    for needle in needles {
        assert!(text.contains(needle), "{needle:?} is missing:\n{text}");
    }
}

pub(super) fn assert_contains_none(text: &str, needles: &[&str]) {
    for needle in needles {
        assert!(!text.contains(needle), "{needle:?} leaked:\n{text}");
    }
}

// SPDX-License-Identifier: MIT

use super::*;

const UNTRUNCATED_RECENT: FinishedFlags = (false, None, None);

struct RecentFixtureSource {
    entries: RefCell<VecDeque<RecentEntryResolution>>,
    requests: Rc<RefCell<Vec<usize>>>,
    pending_after_first_batch: bool,
}

impl RecentFixtureSource {
    fn new(
        entries: Vec<RecentEntryResolution>,
        pending_after_first_batch: bool,
    ) -> (Self, Rc<RefCell<Vec<usize>>>) {
        let requests = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                entries: RefCell::new(entries.into_iter().collect()),
                requests: requests.clone(),
                pending_after_first_batch,
            },
            requests,
        )
    }

    fn numbered(count: usize) -> Self {
        let entries = (0..count)
            .map(|index| recent_resolution(&format!("entry-{index}")))
            .collect();
        Self::new(entries, false).0
    }
}

impl RecentEnumerationSource for RecentFixtureSource {
    fn open(&self, _deadline: Instant) -> RecentEnumerationFuture<Result<(), RecentSourceError>> {
        Box::pin(async { Ok(()) })
    }

    fn next_batch(
        &self,
        batch_size: usize,
        _include_metadata: bool,
        deadline: Instant,
    ) -> RecentEnumerationFuture<Result<Option<Vec<RecentEntryResolution>>, RecentSourceError>>
    {
        let request_number = {
            let mut requests = self.requests.borrow_mut();
            requests.push(batch_size);
            requests.len()
        };
        if Instant::now() >= deadline {
            return Box::pin(async { Err(RecentSourceError::TimedOut) });
        }
        if self.pending_after_first_batch && request_number > 1 {
            return Box::pin(std::future::pending());
        }
        let entries = {
            let mut remaining = self.entries.borrow_mut();
            (0..batch_size)
                .filter_map(|_| remaining.pop_front())
                .collect::<Vec<_>>()
        };
        let batch = (!entries.is_empty()).then_some(entries);
        Box::pin(async move { Ok(batch) })
    }
}

struct FailingRecentSource {
    fail_on_open: bool,
}

impl RecentEnumerationSource for FailingRecentSource {
    fn open(&self, _deadline: Instant) -> RecentEnumerationFuture<Result<(), RecentSourceError>> {
        let fail = self.fail_on_open;
        Box::pin(async move {
            if fail {
                Err(RecentSourceError::Failed(
                    "recent backend is gone".to_owned(),
                ))
            } else {
                Ok(())
            }
        })
    }

    fn next_batch(
        &self,
        _batch_size: usize,
        _include_metadata: bool,
        _deadline: Instant,
    ) -> RecentEnumerationFuture<Result<Option<Vec<RecentEntryResolution>>, RecentSourceError>>
    {
        Box::pin(async { Err(RecentSourceError::Failed("recent read failed".to_owned())) })
    }
}

fn recent_fixture_entry(name: &str) -> FileEntry {
    let info = gio::FileInfo::new();
    info.set_name(name);
    info.set_display_name(name);
    info.set_file_type(gio::FileType::Regular);
    let mut entry = entry_from_info(Location::local(format!("/fixture/{name}")), info);
    entry.recent_unix_seconds = MetadataValue::Known(42);
    entry
}

fn recent_resolution(name: &str) -> RecentEntryResolution {
    RecentEntryResolution::Entry(Box::new(recent_fixture_entry(name)))
}

fn recent_listing(batch_size: usize, max_entries: usize) -> DirectoryRequest {
    DirectoryRequest {
        batch_size,
        max_entries,
        ..directory_request(Location::uri("recent:///"))
    }
}

fn recent_request() -> DirectoryRequest {
    DirectoryRequest {
        time_budget: Duration::from_secs(5),
        ..recent_listing(2, 10)
    }
}

fn run_recent_enumerate(
    request: DirectoryRequest,
    source: impl RecentEnumerationSource + 'static,
) -> Vec<DirectoryEvent> {
    run_events(|emit| enumerate_recent_with_source(request, emit, Instant::now(), Box::new(source)))
}

fn listed_names(events: &[DirectoryEvent]) -> Vec<String> {
    batched_entries(events)
        .iter()
        .map(|entry| entry.native_name.to_string_lossy().into_owned())
        .collect()
}

fn has_event(events: &Events, wanted: fn(&DirectoryEvent) -> bool) -> bool {
    events.borrow().iter().any(wanted)
}

#[test]
fn recent_enumeration_publishes_valid_targets_and_skips_stale_entries() {
    let (source, requests) = RecentFixtureSource::new(
        vec![
            recent_resolution("valid-one"),
            RecentEntryResolution::Stale,
            recent_resolution("valid-two"),
        ],
        false,
    );
    let events = run_recent_enumerate(
        DirectoryRequest {
            include_metadata: true,
            ..recent_listing(2, 10)
        },
        source,
    );

    assert_eq!(listed_names(&events), ["valid-one", "valid-two"]);
    assert_eq!(finished_flags(&events), Some(UNTRUNCATED_RECENT));
    assert_eq!(&*requests.borrow(), &[2, 2, 2]);
}

#[test]
fn recent_enumeration_finishes_normally_when_empty() {
    let (source, _) = RecentFixtureSource::new(Vec::new(), false);
    let events = run_recent_enumerate(recent_listing(2, 10), source);

    assert!(batched_entries(&events).is_empty());
    assert_eq!(finished_flags(&events), Some(UNTRUNCATED_RECENT));
}

#[test]
fn recent_enumeration_respects_the_request_batch_size() {
    let source = RecentFixtureSource::numbered(5);
    let requests = source.requests.clone();
    let events = run_recent_enumerate(recent_listing(2, 10), source);

    assert_eq!(batch_sizes(&events), [2, 2, 1]);
    assert!(requests.borrow().iter().all(|size| *size == 2));
}

#[test]
fn recent_enumeration_respects_max_entries_and_reports_truncation() {
    let events = run_recent_enumerate(recent_listing(2, 3), RecentFixtureSource::numbered(5));

    assert_eq!(batched_entry_count(&events), 3);
    assert_eq!(finished_truncated(&events), Some(true));
}

#[test]
fn a_failing_recent_open_reports_the_backend_error_instead_of_an_empty_collection() {
    let events = run_recent_enumerate(recent_request(), FailingRecentSource { fail_on_open: true });

    assert_eq!(failure_message(&events), Some("recent backend is gone"));
    assert_eq!(finished_truncated(&events), None);
    assert!(batched_entries(&events).is_empty());
}

#[test]
fn a_failing_recent_read_reports_the_backend_error_instead_of_finishing_cleanly() {
    let events = run_recent_enumerate(
        recent_request(),
        FailingRecentSource {
            fail_on_open: false,
        },
    );

    assert_eq!(failure_message(&events), Some("recent read failed"));
    assert_eq!(finished_truncated(&events), None);
}

#[test]
fn an_unresolvable_target_does_not_discard_the_entries_beside_it() {
    let (source, _) = RecentFixtureSource::new(
        vec![
            recent_resolution("before"),
            RecentEntryResolution::TimedOut,
            recent_resolution("after"),
        ],
        false,
    );

    let events = run_recent_enumerate(
        DirectoryRequest {
            batch_size: 3,
            ..recent_request()
        },
        source,
    );

    assert_eq!(
        batched_entries(&events)
            .iter()
            .map(|entry| entry.display_name.clone())
            .collect::<Vec<_>>(),
        ["before", "after"]
    );
    assert_eq!(finished_truncated(&events), Some(true));
}

#[test]
fn recent_enumeration_honors_a_zero_time_budget() {
    let events = run_enumerate(DirectoryRequest {
        time_budget: Duration::ZERO,
        ..recent_listing(2, 10)
    });

    assert!(batched_entries(&events).is_empty());
    assert_eq!(finished_flags(&events), Some((true, None, None)));
}

#[test]
fn cancelling_recent_enumeration_stops_further_publication() {
    let _serial = serial();
    let (source, _) = RecentFixtureSource::new(vec![recent_resolution("first")], true);
    let events = Events::default();
    let handle = enumerate_recent_with_source(
        recent_listing(1, 10),
        recorder(&events),
        Instant::now(),
        Box::new(source),
    );
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !has_event(&events, |event| {
        matches!(event, DirectoryEvent::Batch { .. })
    }) && Instant::now() < deadline
    {
        context.iteration(true);
    }

    assert_eq!(batched_entry_count(&events.borrow()), 1);
    drop(handle);
    for _ in 0..3 {
        while context.iteration(false) {}
    }

    assert_eq!(batched_entry_count(&events.borrow()), 1);
    assert!(!has_event(&events, |event| {
        matches!(event, DirectoryEvent::Finished { .. })
    }));
}

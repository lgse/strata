// SPDX-License-Identifier: MIT

use super::*;
use crate::services::WatchRequest;

type LateWatch = (Location, WatchCallback, Rc<dyn Fn(Watching)>);
type Answer = Rc<dyn Fn(RequestId) -> Vec<DirectoryEvent>>;

enum Watcher {
    /// Keeps each monitor request so the test reports its state and changes later.
    Recorded,
    /// Starts the local adapter's real directory monitor.
    Local,
}

/// Answers every load with a scripted list of events and starts monitors as told.
struct LateMonitorSource {
    answer: Answer,
    watcher: Watcher,
    gap_answer: RefCell<StampAnswer>,
    watches: RefCell<Vec<LateWatch>>,
    loads: RefCell<Vec<Location>>,
    probes: Cell<usize>,
}

fn finished(request_id: RequestId, truncated: bool) -> DirectoryEvent {
    DirectoryEvent::Finished {
        request_id,
        truncated,
        can_trash: None,
        can_delete: None,
    }
}

impl LateMonitorSource {
    fn scripted(answer: Answer, watcher: Watcher, gap_answer: StampAnswer) -> Self {
        Self {
            answer,
            watcher,
            gap_answer: RefCell::new(gap_answer),
            watches: RefCell::default(),
            loads: RefCell::default(),
            probes: Cell::new(0),
        }
    }

    /// Stamps every load with `baseline`, and answers a probe with it too.
    fn new(baseline: DirectoryStamp) -> Self {
        let gap_answer = StampAnswer::Stamp(Box::new(baseline.clone()));
        let answer: Answer = Rc::new(move |request_id| {
            vec![
                DirectoryEvent::Stamp {
                    request_id,
                    stamp: Box::new(baseline.clone()),
                },
                finished(request_id, false),
            ]
        });
        Self::scripted(answer, Watcher::Recorded, gap_answer)
    }

    fn answering(answer: fn(RequestId) -> Vec<DirectoryEvent>) -> Self {
        Self::scripted(Rc::new(answer), Watcher::Recorded, StampAnswer::Unsupported)
    }

    fn watched_locally() -> Self {
        Self::scripted(
            Rc::new(|request_id| vec![finished(request_id, false)]),
            Watcher::Local,
            StampAnswer::Unsupported,
        )
    }

    fn watch_of(&self, location: &Location) -> LateWatch {
        self.watches
            .borrow()
            .iter()
            .find(|(watched, _, _)| watched == location)
            .cloned()
            .expect("a requested monitor")
    }

    fn report(&self, location: &Location, watching: Watching) {
        let (_, _, report) = self.watch_of(location);
        report(watching);
    }

    fn notify(&self, location: &Location, change: DirectoryChange) {
        let (_, notify, _) = self.watch_of(location);
        notify(change);
    }

    fn loads_of(&self, location: &Location) -> usize {
        self.loads
            .borrow()
            .iter()
            .filter(|loaded| *loaded == location)
            .count()
    }
}

impl FileSource for LateMonitorSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.loads.borrow_mut().push(request.location.clone());
        for event in (self.answer)(request.id) {
            emit(event);
        }
        LoadHandle::new(|| {})
    }

    fn start_watch(
        &self,
        request: WatchRequest,
        notify: WatchCallback,
        report: Rc<dyn Fn(Watching)>,
    ) -> LoadHandle {
        match self.watcher {
            Watcher::Local => crate::adapters::LocalFileSource.start_watch(request, notify, report),
            Watcher::Recorded => {
                self.watches
                    .borrow_mut()
                    .push((request.location, notify, report));
                LoadHandle::new(|| {})
            }
        }
    }

    fn probe_stamp(&self, _location: Location, emit: Rc<dyn Fn(StampAnswer)>) -> LoadHandle {
        self.probes.set(self.probes.get() + 1);
        emit(self.gap_answer.borrow().clone());
        LoadHandle::new(|| {})
    }
}

fn stamp_at(changed: i64, read_at: Duration) -> DirectoryStamp {
    DirectoryStamp {
        modified: (changed, 0),
        changed: Some((changed, 0)),
        read_at: std::time::UNIX_EPOCH + read_at,
        ..identified_stamp(folder_identity(1, 1))
    }
}

#[test]
fn recent_navigation_keeps_the_normal_monitor_lifecycle() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let browser = Browser::new(Rc::new(LateMonitorSource::watched_locally()));

    browser.navigate(Location::uri("recent:///"));

    assert_eq!(browser.monitors.borrow().len(), 1);
    assert_eq!(browser.location_at(0), Some(Location::uri("recent:///")));
    assert_eq!(browser.column_snapshot(0).expect("Recent column").count, 0);
}

fn counts_as_monitored(watching: Option<Watching>) -> Option<String> {
    let (parent, child) = (
        Location::local("/fixture"),
        Location::local("/fixture/child"),
    );
    let source = Rc::new(LateMonitorSource::new(stamp_at(
        100,
        Duration::from_secs(1_000),
    )));
    let browser = Browser::new(source.clone());
    browser.navigate(parent.clone());
    browser.show_child(0, child.clone());
    source.report(&child, Watching::Live);
    if let Some(watching) = watching {
        source.report(&parent, watching);
    }
    let monitored = watching == Some(Watching::Live);
    let events = recorded_events(&browser);

    browser.refresh_after_cancellation(&HashSet::from([parent.clone()]));
    let (refreshed, requested) = (source.loads_of(&parent) - 1, source.watches.borrow().len());
    source.notify(&child, DirectoryChange::Remove(child.clone()));

    let got = (
        browser.column_has_monitor(0),
        refreshed,
        requested,
        count_events(&events, |event| matches!(event, BrowserEvent::Reset)),
    );
    let expected = (
        monitored,
        usize::from(!monitored),
        2,
        usize::from(!monitored),
    );
    (got != expected).then(|| format!("{watching:?}: got {got:?}, expected {expected:?}"))
}

#[test]
fn a_column_counts_as_monitored_only_once_its_monitor_is_live() {
    assert_rows(
        [None, Some(Watching::Unavailable), Some(Watching::Live)],
        counts_as_monitored,
    );
}

fn relists_the_gap(
    row: &str,
    baseline: DirectoryStamp,
    answer: StampAnswer,
    relisted: bool,
) -> Option<String> {
    let folder = Location::local("/fixture");
    let source = Rc::new(LateMonitorSource::new(baseline));
    source.gap_answer.replace(answer);
    let browser = Browser::new(source.clone());
    browser.navigate(folder.clone());
    let events = recorded_events(&browser);
    let probes_before_live = source.probes.get();

    source.report(&folder, Watching::Live);

    let got = (
        probes_before_live,
        source.probes.get(),
        source.loads_of(&folder) - 1,
        count_events(&events, |event| {
            matches!(event, BrowserEvent::ColumnRefreshing { depth: 0 })
        }),
    );
    let expected = (0, 1, usize::from(relisted), usize::from(relisted));
    (got != expected).then(|| format!("{row}: got {got:?}, expected {expected:?}"))
}

#[test]
fn a_change_between_the_stamp_and_the_live_monitor_is_listed_once_more() {
    let settled = stamp_at(100, Duration::from_secs(1_000));
    let changed_at_the_read = DirectoryStamp {
        coarse_window: Some(Duration::from_secs(2)),
        ..stamp_at(100, Duration::from_millis(101_500))
    };
    let fine_grained_at_the_read = stamp_at(100, Duration::from_millis(100_050));
    let rows = [
        (
            "unchanged",
            settled.clone(),
            StampAnswer::Stamp(Box::new(settled.clone())),
            false,
        ),
        (
            "moved",
            settled.clone(),
            StampAnswer::Stamp(Box::new(stamp_at(200, Duration::from_secs(1_000)))),
            true,
        ),
        (
            "coarse times changed just before the load",
            changed_at_the_read.clone(),
            StampAnswer::Stamp(Box::new(changed_at_the_read)),
            true,
        ),
        (
            "fine-grained times changed just before the load",
            fine_grained_at_the_read.clone(),
            StampAnswer::Stamp(Box::new(fine_grained_at_the_read)),
            false,
        ),
        ("gone", settled.clone(), StampAnswer::Missing, true),
        (
            "unreachable",
            settled.clone(),
            StampAnswer::Unreachable,
            false,
        ),
    ];
    assert_rows(rows, |(row, baseline, answer, relisted)| {
        relists_the_gap(row, baseline, answer, relisted)
    });
}

type Script = fn(RequestId) -> Vec<DirectoryEvent>;

const ANSWERS: [(&str, Script, usize); 4] = [
    (
        "stamped, then out of time",
        |request_id| {
            vec![
                DirectoryEvent::Stamp {
                    request_id,
                    stamp: Box::new(stamp_at(100, Duration::from_secs(1_000))),
                },
                finished(request_id, true),
            ]
        },
        1,
    ),
    (
        "listed, then out of time",
        |request_id| {
            vec![
                DirectoryEvent::Batch {
                    request_id,
                    entries: vec![fixture_entry("/fixture/a")],
                },
                finished(request_id, true),
            ]
        },
        1,
    ),
    ("empty", |request_id| vec![finished(request_id, false)], 1),
    (
        "out of time before any answer",
        |request_id| vec![finished(request_id, true)],
        0,
    ),
];

#[test]
fn a_monitor_is_requested_only_once_the_folder_answered() {
    assert_rows(ANSWERS, |(row, answer, expected)| {
        let source = Rc::new(LateMonitorSource::answering(answer));
        let browser = Browser::new(source.clone());

        browser.navigate(Location::local("/fixture"));

        let requested = source.watches.borrow().len();
        (requested != expected)
            .then(|| format!("{row}: {requested} monitors requested, expected {expected}"))
    });
}

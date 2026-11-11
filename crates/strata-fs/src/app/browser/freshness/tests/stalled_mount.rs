// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use super::*;
use crate::{
    adapters::{FsOp, FsQuirks, LocalFileSource, Quirks, mount_lanes},
    test_support::Gate,
};

#[derive(Clone, Copy)]
enum Waiting {
    CheckOfTheFolder,
    ListingInAnotherTab,
    GvfsRequest,
    RequestOutlivingTheListing,
    Nothing,
    RequestBeforeTheFailure,
}

const ROWS: [(&str, Waiting); 6] = [
    (
        "a folder opened once its check gave up, as at start-up",
        Waiting::CheckOfTheFolder,
    ),
    (
        "a child folder shown while another tab waits on it",
        Waiting::ListingInAnotherTab,
    ),
    (
        "a GVfs folder opened while its backend waits",
        Waiting::GvfsRequest,
    ),
    (
        "a refresh queued behind a listing, refused as the mount comes back",
        Waiting::RequestOutlivingTheListing,
    ),
    (
        "a folder that failed while its mount answered",
        Waiting::Nothing,
    ),
    (
        "a folder that failed while its mount answered, after a refused refresh",
        Waiting::RequestBeforeTheFailure,
    ),
];

/// Whether every stage completed, reloads and failure while stalled, the final names,
/// reloads once the mount answers, and whether the folder still shows its failure.
type Outcome = (bool, (usize, bool), Vec<String>, usize, bool);

fn pump_until(done: &dyn Fn() -> bool) -> bool {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() && Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    done()
}

fn wait_for_the_released_stamp() {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !context.pending() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(50));
}

/// A real folder whose stamp waits on `gate`, beside one whose stamp fails.
/// Fields drop in order: the browser before the quirks, the quirks before the gate.
struct StalledMount {
    events: Events,
    browser: Rc<Browser>,
    target: Location,
    depth: usize,
    _quirks: FsQuirks,
    gate: Gate,
    folder: PathBuf,
    _root: tempfile::TempDir,
}

/// What else holds the mount while the folder is opened.
#[derive(Default)]
struct Waiters {
    check: Option<LoadHandle>,
    _other_tab: Option<Rc<Browser>>,
    request: Option<mount_lanes::Admitted>,
}

impl StalledMount {
    fn new(waiting: Waiting) -> Self {
        let root = tempfile::tempdir().expect("fixture");
        let folder = root.path().join("folder");
        std::fs::create_dir_all(folder.join("stalled")).expect("stalled folder");
        std::fs::create_dir(folder.join("broken")).expect("broken folder");
        std::fs::write(folder.join("stalled/listed.txt"), "").expect("listed file");
        let gate = Gate::default();
        let quirks = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![(FsOp::FolderStamp, "stalled", 0, gate.racer())],
                fail: vec![(
                    FsOp::FolderStamp,
                    "broken",
                    waiting.stamps_before_breaking(),
                    rustix::io::Errno::IO,
                )],
                ..Quirks::default()
            },
        );
        let (target, depth) = waiting.target(&folder);
        let browser = Browser::new(Rc::new(LocalFileSource));
        let events = recorded(&browser);
        Self {
            events,
            browser,
            target,
            depth,
            _quirks: quirks,
            gate,
            folder,
            _root: root,
        }
    }

    fn reloads(&self) -> usize {
        count(&self.events, |event| match event {
            BrowserEvent::ColumnReloaded { depth: reloaded } => *reloaded == self.depth,
            _ => false,
        })
    }

    fn finished(&self) -> usize {
        count(&self.events, |event| {
            matches!(event, BrowserEvent::LoadFinished { .. })
        })
    }

    fn load_state_is(&self, wanted: fn(&LoadState) -> bool) -> bool {
        self.browser
            .state
            .borrow()
            .columns
            .get(self.depth)
            .is_some_and(|column| wanted(&column.load_state))
    }

    fn failed(&self) -> bool {
        self.load_state_is(|state| matches!(state, LoadState::Error(_)))
    }

    fn loading(&self) -> bool {
        self.load_state_is(|state| matches!(state, LoadState::Loading))
    }

    fn responsiveness(&self) -> Responsiveness {
        LocalFileSource.responsiveness(&self.target)
    }

    fn admit(&self, name: &str) -> Option<mount_lanes::Admitted> {
        mount_lanes::admit(&self.folder.join(name), mount_lanes::Class::Interactive).ok()
    }

    fn open_the_parent(&self) {
        self.browser.navigate(Location::local(&self.folder));
        pump_until(&|| self.browser.settled(0));
    }

    fn list_in_another_tab(&self) -> Rc<Browser> {
        self.open_the_parent();
        let tab = Browser::new(Rc::new(LocalFileSource));
        tab.navigate(self.target.clone());
        tab
    }

    fn refresh_behind_a_listing(&self) -> Option<mount_lanes::Admitted> {
        let request = self.admit("stalled");
        self.browser.navigate(self.target.clone());
        pump_until(&|| self.gate.reached());
        self.browser.retry_column(self.depth);
        for _ in 0..50 {
            glib::MainContext::default().iteration(false);
        }
        request
    }

    fn request_after_listing_the_child(&self) -> Option<mount_lanes::Admitted> {
        self.open_the_parent();
        self.browser.show_child(0, self.target.clone());
        pump_until(&|| {
            self.browser.settled(self.depth) && self.browser.column_has_monitor(self.depth)
        });
        self.admit("broken")
    }

    fn poll(&self) -> (usize, bool) {
        let reloads_before = self.reloads();
        self.browser.follow_responsiveness();
        (self.reloads() - reloads_before, self.failed())
    }

    fn reloads_once_followed(&self) -> usize {
        let reloads_before = self.reloads();
        self.browser.follow_responsiveness();
        pump_until(&|| !self.loading());
        self.browser.follow_responsiveness();
        self.reloads() - reloads_before
    }
}

impl Waiting {
    fn stamps_before_breaking(self) -> usize {
        match self {
            Self::RequestBeforeTheFailure => 2,
            _ => 0,
        }
    }

    fn target(self, folder: &Path) -> (Location, usize) {
        match self {
            Self::CheckOfTheFolder | Self::RequestOutlivingTheListing => {
                (Location::local(folder.join("stalled")), 0)
            }
            Self::ListingInAnotherTab => (Location::local(folder.join("stalled")), 1),
            Self::GvfsRequest => (
                Location::uri("sftp://strata-recovering-backend.invalid/share"),
                0,
            ),
            Self::Nothing => (Location::local(folder.join("broken")), 0),
            Self::RequestBeforeTheFailure => (Location::local(folder.join("broken")), 1),
        }
    }

    fn wait(self, mount: &StalledMount) -> Waiters {
        match self {
            Self::CheckOfTheFolder => Waiters {
                check: Some(
                    LocalFileSource.validate_location_async(mount.target.clone(), Rc::new(|_| {})),
                ),
                ..Waiters::default()
            },
            Self::ListingInAnotherTab => Waiters {
                _other_tab: Some(mount.list_in_another_tab()),
                ..Waiters::default()
            },
            Self::GvfsRequest => Waiters {
                request: mount_lanes::admit_remote(
                    mount_lanes::authority(mount.target.uri_value().unwrap_or_default()),
                    mount_lanes::Class::Interactive,
                )
                .ok(),
                ..Waiters::default()
            },
            Self::RequestOutlivingTheListing => Waiters {
                request: mount.refresh_behind_a_listing(),
                ..Waiters::default()
            },
            Self::Nothing => Waiters::default(),
            Self::RequestBeforeTheFailure => Waiters {
                request: mount.request_after_listing_the_child(),
                ..Waiters::default()
            },
        }
    }

    fn open_while_stalled(self, mount: &StalledMount) -> bool {
        let finished_before = mount.finished();
        match self {
            Self::ListingInAnotherTab => mount.browser.show_child(0, mount.target.clone()),
            Self::RequestOutlivingTheListing => {}
            Self::RequestBeforeTheFailure => mount.browser.retry_column(mount.depth),
            _ => mount.browser.navigate(mount.target.clone()),
        }
        match self {
            Self::RequestOutlivingTheListing => mount.loading(),
            Self::RequestBeforeTheFailure => {
                pump_until(&|| mount.finished() > finished_before) && !mount.failed()
            }
            _ => pump_until(&|| mount.failed()),
        }
    }

    fn failed_once_answering(self, mount: &StalledMount) -> bool {
        match self {
            Self::RequestOutlivingTheListing => pump_until(&|| mount.failed()),
            Self::RequestBeforeTheFailure => {
                mount.browser.retry_column(mount.depth);
                pump_until(&|| mount.failed())
            }
            _ => true,
        }
    }

    fn expected(self) -> Outcome {
        let listed = owned(&["listed.txt"]);
        match self {
            Self::CheckOfTheFolder | Self::ListingInAnotherTab => {
                (true, (0, true), listed, 1, false)
            }
            Self::RequestOutlivingTheListing => (true, (0, false), listed, 1, false),
            Self::GvfsRequest => (true, (0, true), Vec::new(), 1, true),
            Self::Nothing => (true, (0, true), Vec::new(), 0, true),
            Self::RequestBeforeTheFailure => (true, (0, false), Vec::new(), 0, true),
        }
    }
}

fn reloads_once_the_mount_answers(row: &str, waiting: Waiting) -> Option<String> {
    let mount = StalledMount::new(waiting);
    let mut waiters = waiting.wait(&mount);
    let stalled = matches!(waiting, Waiting::Nothing)
        || pump_until(&|| mount.responsiveness() == Responsiveness::Stalled);
    drop(waiters.check.take());
    let opened_while_stalled = waiting.open_while_stalled(&mount);
    let polled_while_stalled = mount.poll();

    let quiet = pump_until(&|| !glib::MainContext::default().pending());
    mount.gate.release();
    if matches!(waiting, Waiting::RequestOutlivingTheListing) {
        wait_for_the_released_stamp();
    }
    if let Some(request) = waiters.request.take() {
        request.progress().beat();
    }
    let answering = pump_until(&|| mount.responsiveness() == Responsiveness::Answering);
    let failed_once_answering = waiting.failed_once_answering(&mount);
    let reloads = mount.reloads_once_followed();

    let stages = [
        stalled,
        opened_while_stalled,
        quiet,
        answering,
        failed_once_answering,
    ];
    let got = (
        stages.iter().all(|completed| *completed),
        polled_while_stalled,
        names_at(&mount.browser, mount.depth),
        reloads,
        mount.failed(),
    );
    mismatch(row, got, waiting.expected())
}

#[test]
fn a_folder_that_failed_while_its_mount_was_stalled_reloads_once_the_mount_answers() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    assert_rows(ROWS, |(row, waiting)| {
        reloads_once_the_mount_answers(row, waiting)
    });
}

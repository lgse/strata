// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy)]
enum KeptRefresh {
    MonitorRescan,
    ReloadKey,
    CutShort,
    RefusedAsItAnswers,
    ReloadedOnceAnswering,
    BackgroundRelist,
    HeldByAnInlineEdit,
    HeldByAnOperation,
    OwedAndRefusedAgainAsItAnswers,
    OwedAndBackgroundRelistKept,
}

const ROWS: [(&str, KeptRefresh); 10] = [
    (
        "a monitor's rescan of a folder on a USB stick",
        KeptRefresh::MonitorRescan,
    ),
    ("F5", KeptRefresh::ReloadKey),
    ("F5 cut short by its time budget", KeptRefresh::CutShort),
    (
        "F5 queued behind a listing and refused as the mount answers again",
        KeptRefresh::RefusedAsItAnswers,
    ),
    (
        "F5, and F5 again once the mount answers",
        KeptRefresh::ReloadedOnceAnswering,
    ),
    (
        "a background re-list, left to the next check",
        KeptRefresh::BackgroundRelist,
    ),
    (
        "F5, with an inline rename open when the mount answers",
        KeptRefresh::HeldByAnInlineEdit,
    ),
    (
        "F5, with an operation writing into the folder when the mount answers",
        KeptRefresh::HeldByAnOperation,
    ),
    (
        "F5 owed, then another F5 refused as the mount answers again",
        KeptRefresh::OwedAndRefusedAgainAsItAnswers,
    ),
    (
        "F5 owed, then a background re-list kept once the mount answers",
        KeptRefresh::OwedAndBackgroundRelistKept,
    ),
];

/// A folder whose listing changed elsewhere while its mount stopped answering.
struct Fixture {
    source: Rc<Folders>,
    folder: Location,
    browser: Rc<Browser>,
    clock: Clock,
    editing: Rc<Cell<bool>>,
    events: Events,
}

impl Fixture {
    fn new(refresh: KeptRefresh) -> Self {
        let source = Rc::new(Folders::default());
        let folder = match refresh.on_a_network_share() {
            true => source.add("/net/share", network(1), Some(Watching::Live)),
            false => source.add("/run/media/me/stick", local(1), Some(Watching::Live)),
        };
        let (browser, clock) = browser_on(&source);
        let editing = hold_freshness(&browser, false);
        browser.navigate(folder.clone());
        let events = recorded(&browser);
        source.change(&folder, &["listed", "added elsewhere"]);
        Self {
            source,
            folder,
            browser,
            clock,
            editing,
            events,
        }
    }

    fn stall(&self) {
        self.source.responsiveness.set(Responsiveness::Stalled);
    }

    fn reload_queued(&self) {
        self.source.queue_loads.set(true);
        self.browser.reload_active();
        self.source.queue_loads.set(false);
    }

    fn fail_a_held_background_relist(&self) {
        self.source.hold_loads.set(true);
        self.clock.tick_at(15);
        self.source.fail_loads.set(true);
        self.source.finish_held_loads();
        self.source.fail_loads.set(false);
    }

    fn answered(&self) -> (usize, bool, bool) {
        (
            self.source.loads_of(&self.folder),
            self.browser.freshness.following.get(),
            spinner_follows_the_load(&self.browser, &self.events),
        )
    }

    fn settle(&self) {
        self.source.hold_loads.set(false);
        self.source.finish_held_loads();
        self.browser.follow_responsiveness();
        self.clock.tick_at(30);
    }

    fn reloads_or_failures(&self) -> usize {
        count(&self.events, |event| {
            matches!(
                event,
                BrowserEvent::LoadFailed { .. } | BrowserEvent::ColumnReloaded { .. }
            )
        })
    }
}

impl KeptRefresh {
    fn on_a_network_share(self) -> bool {
        matches!(
            self,
            Self::BackgroundRelist | Self::OwedAndBackgroundRelistKept
        )
    }

    fn owed_twice(self) -> bool {
        matches!(
            self,
            Self::OwedAndRefusedAgainAsItAnswers | Self::OwedAndBackgroundRelistKept
        )
    }

    fn held(self) -> bool {
        matches!(self, Self::HeldByAnInlineEdit | Self::HeldByAnOperation)
    }

    fn keep_while_stalled(self, fixture: &Fixture) {
        match self {
            Self::MonitorRescan => {
                fixture.stall();
                fixture
                    .source
                    .notify(&fixture.folder, DirectoryChange::Rescan);
            }
            Self::ReloadKey
            | Self::ReloadedOnceAnswering
            | Self::HeldByAnInlineEdit
            | Self::HeldByAnOperation
            | Self::OwedAndBackgroundRelistKept => {
                fixture.stall();
                fixture.browser.reload_active();
            }
            Self::CutShort => {
                fixture.source.probes_run_out_of_time.set(true);
                fixture.stall();
                fixture.browser.reload_active();
            }
            Self::RefusedAsItAnswers => {
                fixture.source.queue_loads.set(true);
                fixture.stall();
                fixture.browser.reload_active();
                fixture.source.queue_loads.set(false);
            }
            Self::OwedAndRefusedAgainAsItAnswers => {
                fixture.stall();
                fixture.browser.reload_active();
                fixture.reload_queued();
            }
            Self::BackgroundRelist => self.stall_a_held_check(fixture),
        }
    }

    fn stall_a_held_check(self, fixture: &Fixture) {
        fixture.source.hold_checks.set(true);
        fixture.clock.tick_at(15);
        fixture.stall();
        fixture.source.hold_checks.set(false);
        fixture.source.answer_held_checks();
    }

    fn once_answering(self, fixture: &Fixture) {
        match self {
            Self::RefusedAsItAnswers => fixture.source.refuse_queued_loads(),
            Self::ReloadedOnceAnswering => {
                fixture.browser.reload_active();
                fixture.browser.follow_responsiveness();
            }
            Self::OwedAndRefusedAgainAsItAnswers => {
                fixture.source.hold_loads.set(true);
                fixture.source.refuse_queued_loads();
            }
            Self::OwedAndBackgroundRelistKept => fixture.fail_a_held_background_relist(),
            _ => fixture.browser.follow_responsiveness(),
        }
    }

    fn kept_loads(self) -> usize {
        match self {
            Self::OwedAndRefusedAgainAsItAnswers => 3,
            _ => 2,
        }
    }

    fn loads_once_answering(self) -> usize {
        match self {
            _ if self.owed_twice() => 4,
            _ if self.held() => 2,
            Self::BackgroundRelist => 2,
            _ => 3,
        }
    }

    fn expected(self) -> KeptOutcome {
        (
            (owned(&["listed"]), self.kept_loads()),
            (
                self.loads_once_answering(),
                self.owed_twice() || self.held(),
                true,
            ),
            owned(&["added elsewhere", "listed"]),
            usize::from(matches!(self, Self::ReloadedOnceAnswering)),
        )
    }
}

/// Kept rows and loads, (loads, following, spinner follows the load) once answering,
/// the final names, and reloads or failures.
type KeptOutcome = (
    (Vec<String>, usize),
    (usize, bool, bool),
    Vec<String>,
    usize,
);

fn spinner_follows_the_load(browser: &Browser, events: &RefCell<Vec<BrowserEvent>>) -> bool {
    let spinning =
        events.borrow().iter().rev().find_map(|event| match event {
            BrowserEvent::ColumnRefreshing { depth: 0 }
            | BrowserEvent::ColumnReloaded { depth: 0 } => Some(true),
            BrowserEvent::LoadFinished { depth: 0, .. }
            | BrowserEvent::LoadFailed { depth: 0, .. } => Some(false),
            _ => None,
        });
    spinning.unwrap_or(false)
        == browser
            .column_snapshot(0)
            .is_some_and(|column| column.loading)
}

fn refresh_once_answering(row: &str, refresh: KeptRefresh) -> Option<String> {
    let fixture = Fixture::new(refresh);
    refresh.keep_while_stalled(&fixture);
    let kept = (
        names_at(&fixture.browser, 0),
        fixture.source.loads_of(&fixture.folder),
    );

    let activity =
        matches!(refresh, KeptRefresh::HeldByAnOperation).then(|| writing_into(&fixture.folder));
    fixture
        .editing
        .set(matches!(refresh, KeptRefresh::HeldByAnInlineEdit));
    fixture.source.responsiveness.set(Responsiveness::Answering);
    refresh.once_answering(&fixture);
    let once_answering = fixture.answered();
    fixture.editing.set(false);
    drop(activity);
    fixture.settle();

    let got = (
        kept,
        once_answering,
        names_at(&fixture.browser, 0),
        fixture.reloads_or_failures(),
    );
    let expected = refresh.expected();
    (got != expected).then(|| {
        format!(
            "{row}: got (kept, (loads, following, spinner follows the load) once answering, \
             names, reloads or failures) {got:?}, expected {expected:?}"
        )
    })
}

#[test]
fn a_refresh_kept_while_its_mount_was_not_answering_runs_again_once_it_answers() {
    assert_rows(ROWS, |(row, refresh)| refresh_once_answering(row, refresh));
}

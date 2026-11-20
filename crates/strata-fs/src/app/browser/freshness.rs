// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
    time::{Duration, Instant},
};

use crate::{
    app::navigation::LoadState,
    model::Location,
    services::{DirectoryStamp, LoadHandle, MountToken, RequestId, Responsiveness, file_activity},
};

#[cfg(test)]
use crate::services::StampAnswer;

use super::Browser;

const FOCUSED_INTERVAL: Duration = Duration::from_secs(3);
const VISIBLE_INTERVAL: Duration = Duration::from_secs(15);
const RELIST_DUTY: u32 = 10;
const UNCHANGED_BEFORE_SLOWING: u32 = 20;
const MOST_SLOWING: u32 = 4;
const POWER_SAVER_SLOWING: u32 = 4;
const UNREACHABLE_RETRY: Duration = Duration::from_secs(30);
const RELIST_AFTER_AWAY: Duration = Duration::from_secs(30);

thread_local! {
    static BLIND_MOUNTS: RefCell<HashSet<MountToken>> = RefCell::default();
}

fn blind(mount: &MountToken) -> bool {
    BLIND_MOUNTS.with(|mounts| mounts.borrow().contains(mount))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Presence {
    Focused,
    #[default]
    Visible,
    Hidden,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VisibleDepths {
    #[default]
    All,
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FreshnessInput {
    pub presence: Presence,
    pub scope: VisibleDepths,
    pub power_saver: bool,
    pub watch_network_folders: bool,
}

impl Default for FreshnessInput {
    fn default() -> Self {
        Self {
            presence: Presence::default(),
            scope: VisibleDepths::default(),
            power_saver: false,
            watch_network_folders: true,
        }
    }
}

struct Listing {
    request: RequestId,
    started: Instant,
    stamp: Option<(DirectoryStamp, Instant)>,
    digest: Option<u64>,
    background: bool,
    confirming: bool,
}

#[derive(Default)]
pub(super) struct ColumnFreshness {
    baseline: Option<DirectoryStamp>,
    digest: Option<u64>,
    listing: Option<Listing>,
    next_due: Option<Instant>,
    quiet_until: Option<Instant>,
    confirm_at: Option<Instant>,
    confirming: bool,
    relist: bool,
    unchanged: u32,
    checking: bool,
    not_responding: bool,
    names_may_lag: bool,
    retry_when_answering: bool,
}

impl ColumnFreshness {
    fn mount(&self) -> Option<MountToken> {
        self.baseline
            .as_ref()
            .and_then(|baseline| baseline.mount.clone())
    }

    fn on_blind_mount(&self) -> bool {
        self.mount().is_some_and(|mount| blind(&mount))
    }
}

#[derive(Default)]
pub(super) struct Freshness {
    input: Cell<FreshnessInput>,
    hold: RefCell<Option<Rc<dyn Fn() -> bool>>>,
    armed_for: Cell<Option<Instant>>,
    left_focus: Cell<Option<Duration>>,
    timer: RefCell<Option<glib::SourceId>>,
    jobs: RefCell<Vec<(Rc<Cell<usize>>, LoadHandle)>>,
    following: Cell<bool>,
    follow_timer: RefCell<Option<glib::SourceId>>,
    lanes: RefCell<Option<LoadHandle>>,
    #[cfg(any(test, feature = "test-support"))]
    clock: Cell<Option<(Instant, Duration)>>,
}

mod checks;
mod responsiveness;

/// What one finished listing says about its mount, compared with the listing before it.
struct BlindEvidence {
    moved: bool,
    same_names: bool,
    lagged: bool,
    unchanged: bool,
    cached: bool,
}

impl BlindEvidence {
    fn names_may_lag(&self) -> bool {
        self.same_names && (self.moved || self.lagged)
    }

    /// Names changed under a stamp that did not move, with no daemon cache to
    /// blame: this mount hides changes from stamps.
    fn proves_blind(&self) -> bool {
        self.unchanged && !self.same_names && !self.lagged && !self.cached
    }

    fn count_unchanged(&self, unchanged: u32) -> u32 {
        match self.unchanged && self.same_names {
            true => unchanged.saturating_add(1),
            false => 0,
        }
    }
}

fn listing_evidence(
    listing: Option<&Listing>,
    current: Option<RequestId>,
) -> Option<(&DirectoryStamp, u64)> {
    let listing = listing.filter(|listing| Some(listing.request) == current)?;
    let (stamp, _) = listing.stamp.as_ref()?;
    Some((stamp, listing.digest?))
}

impl Browser {
    pub fn set_freshness(self: &Rc<Self>, input: FreshnessInput) {
        let previous = self.freshness.input.replace(input);
        let focused = |input: FreshnessInput| input.presence == Presence::Focused;
        if focused(previous) && !focused(input) {
            self.freshness.left_focus.set(Some(self.boot_clock()));
        }
        if !focused(previous) && focused(input) {
            self.regain_focus();
        } else if previous != input {
            self.reschedule_freshness();
        }
        if previous.presence != input.presence {
            self.follow_responsiveness();
        }
    }

    fn regain_focus(self: &Rc<Self>) {
        let now = self.freshness_now();
        let reread = self
            .freshness
            .left_focus
            .take()
            .is_some_and(|left| self.boot_clock().saturating_sub(left) >= RELIST_AFTER_AWAY);
        for (depth, column) in self.freshness_columns().into_iter().enumerate() {
            let mut column = column.borrow_mut();
            if !self.qualifies(depth, &column) {
                continue;
            }
            column.unchanged = 0;
            column.relist |= reread;
            column.next_due = Some(now);
        }
        self.freshness_tick();
    }

    pub fn freshness_input(&self) -> FreshnessInput {
        self.freshness.input.get()
    }

    pub fn set_freshness_hold(&self, hold: impl Fn() -> bool + 'static) {
        self.freshness.hold.replace(Some(Rc::new(hold)));
    }

    fn freshness_now(&self) -> Instant {
        #[cfg(any(test, feature = "test-support"))]
        if let Some((now, _)) = self.freshness.clock.get() {
            return now;
        }
        Instant::now()
    }

    fn boot_clock(&self) -> Duration {
        #[cfg(any(test, feature = "test-support"))]
        if let Some((_, booted)) = self.freshness.clock.get() {
            return booted;
        }
        Duration::try_from(rustix::time::clock_gettime(rustix::time::ClockId::Boottime))
            .unwrap_or_default()
    }

    fn check_interval(&self, unchanged: u32) -> Duration {
        let input = self.freshness.input.get();
        let interval = match input.presence {
            Presence::Focused => FOCUSED_INTERVAL,
            Presence::Visible | Presence::Hidden => VISIBLE_INTERVAL,
        };
        let saving = if input.power_saver {
            POWER_SAVER_SLOWING
        } else {
            1
        };
        interval
            * saving
            * 2u32
                .saturating_pow(unchanged / UNCHANGED_BEFORE_SLOWING)
                .min(MOST_SLOWING)
    }

    fn column_interval(&self, column: &ColumnFreshness) -> Duration {
        let interval = self.check_interval(column.unchanged);
        match column.on_blind_mount() {
            true => interval.max(VISIBLE_INTERVAL),
            false => interval,
        }
    }

    fn checking(&self) -> bool {
        let input = self.freshness.input.get();
        match input.presence {
            Presence::Focused => true,
            Presence::Visible => !input.power_saver,
            Presence::Hidden => false,
        }
    }

    fn column_freshness(&self, depth: usize) -> Option<Rc<RefCell<ColumnFreshness>>> {
        self.monitors
            .borrow()
            .get(depth)
            .map(|monitor| monitor.freshness.clone())
    }

    pub(super) fn record_baseline(
        &self,
        depth: usize,
        request_id: RequestId,
        stamp: &DirectoryStamp,
    ) {
        if let Some(column) = self.column_freshness(depth)
            && let Some(listing) = column
                .borrow_mut()
                .listing
                .as_mut()
                .filter(|listing| listing.request == request_id)
        {
            listing.stamp = Some((stamp.clone(), self.freshness_now()));
        }
    }

    pub(super) fn record_digest(&self, request_id: RequestId, digest: u64) {
        let depth = self.state.borrow().depth_for_request(request_id);
        if let Some(column) = depth.and_then(|depth| self.column_freshness(depth))
            && let Some(listing) = column
                .borrow_mut()
                .listing
                .as_mut()
                .filter(|listing| listing.request == request_id)
        {
            listing.digest = Some(digest);
        }
    }

    pub(super) fn note_listing_started(
        self: &Rc<Self>,
        depth: usize,
        request_id: RequestId,
        background: bool,
    ) {
        if let Some(column) = self.column_freshness(depth) {
            let mut column = column.borrow_mut();
            column.relist = false;
            let confirming = std::mem::take(&mut column.confirming) && background;
            column.listing = Some(Listing {
                request: request_id,
                started: self.freshness_now(),
                stamp: None,
                digest: None,
                background,
                confirming,
            });
        }
        self.keep_following_responsiveness(true);
    }

    pub(super) fn note_listing_unfinished(&self, depth: usize, refused: bool) {
        let unanswered = refused
            || self.location_at(depth).is_some_and(|location| {
                self.source.responsiveness(&location) != Responsiveness::Answering
            });
        self.owe_refresh(depth, unanswered);
    }

    pub(super) fn owe_refresh(&self, depth: usize, owed: bool) {
        if let Some(column) = self.column_freshness(depth) {
            column.borrow_mut().retry_when_answering = owed;
        }
    }

    pub(super) fn take_owed_refresh(&self, depth: usize) -> bool {
        self.column_freshness(depth)
            .is_some_and(|column| std::mem::take(&mut column.borrow_mut().retry_when_answering))
    }

    fn end_listing(&self, depth: usize, column: &mut ColumnFreshness) -> Option<Listing> {
        let now = self.freshness_now();
        let current = self.state.borrow().request_id_for_depth(depth);
        let listing = column
            .listing
            .take()
            .filter(|listing| Some(listing.request) == current);
        let took = listing.as_ref().map_or(Duration::ZERO, |listing| {
            now.saturating_duration_since(listing.started)
        });
        let quiet_until = now + took * RELIST_DUTY;
        column.quiet_until = Some(quiet_until);
        column.next_due = Some((now + self.column_interval(column)).max(quiet_until));
        listing
    }

    fn learn_blind_mounts(&self, depth: usize, column: &mut ColumnFreshness) {
        let current = self.state.borrow().request_id_for_depth(depth);
        let Some((stamp, digest)) = listing_evidence(column.listing.as_ref(), current) else {
            return;
        };
        let (Some(baseline), Some(before), Some(mount)) =
            (column.baseline.as_ref(), column.digest, stamp.mount.clone())
        else {
            return;
        };
        let moved = stamp.moved_since(baseline);
        let evidence = BlindEvidence {
            same_names: digest == before,
            moved,
            lagged: column.names_may_lag,
            unchanged: !moved && !baseline.may_hide_changes(),
            cached: stamp.daemon_cache.is_some() || baseline.daemon_cache.is_some(),
        };
        column.names_may_lag = evidence.names_may_lag();
        if evidence.proves_blind() {
            BLIND_MOUNTS.with(|mounts| mounts.borrow_mut().insert(mount.clone()));
        }
        if blind(&mount) {
            column.unchanged = evidence.count_unchanged(column.unchanged);
        }
    }

    pub(super) fn note_listing_finished(self: &Rc<Self>, depth: usize) {
        let Some(column) = self.column_freshness(depth) else {
            return;
        };
        {
            let mut column = column.borrow_mut();
            column.retry_when_answering = false;
            self.learn_blind_mounts(depth, &mut column);
            if let Some(Listing {
                stamp: Some((stamp, stamped)),
                digest,
                confirming,
                ..
            }) = self.end_listing(depth, &mut column)
            {
                column.confirm_at = stamp
                    .confirm_after()
                    .filter(|_| !confirming)
                    .map(|after| stamped + after);
                column.baseline = Some(stamp);
                column.digest = digest;
            }
        }
        self.reschedule_freshness();
        self.start_relists();
        self.follow_responsiveness();
    }

    pub(super) fn note_listing_kept(self: &Rc<Self>, depth: usize) {
        if let Some(column) = self.column_freshness(depth) {
            let mut column = column.borrow_mut();
            if self
                .end_listing(depth, &mut column)
                .is_some_and(|listing| listing.confirming)
            {
                column.confirm_at = column.next_due;
            }
        }
        self.reschedule_freshness();
        self.start_relists();
        self.follow_responsiveness();
    }

    fn watched(&self, depth: usize, column: &ColumnFreshness) -> bool {
        !column.checking && self.qualifies(depth, column)
    }

    fn qualifies(&self, depth: usize, column: &ColumnFreshness) -> bool {
        !matches!(
            self.state
                .borrow()
                .columns
                .get(depth)
                .map(|column| &column.load_state),
            Some(LoadState::Error(_) | LoadState::Denied { .. })
        ) && column.baseline.as_ref().is_some_and(|baseline| {
            if baseline.outside_writers {
                self.freshness.input.get().watch_network_folders
            } else {
                !self.column_has_monitor(depth)
            }
        }) && match self.freshness.input.get().scope {
            VisibleDepths::All => true,
            VisibleDepths::Active => self.active_depth() == Some(depth),
        }
    }

    pub(super) fn settled(&self, depth: usize) -> bool {
        matches!(
            self.state
                .borrow()
                .columns
                .get(depth)
                .map(|column| &column.load_state),
            Some(LoadState::Ready | LoadState::Empty)
        ) && !self.staging.borrow().contains_key(&depth)
            && !self.sorting.borrow().contains_key(&depth)
    }

    pub fn operation_touches(location: &Location) -> bool {
        location
            .native_path()
            .is_some_and(file_activity::is_changing)
            || file_activity::is_active_at(location)
    }

    fn freshness_held(&self) -> bool {
        let hold = self.freshness.hold.borrow().clone();
        hold.is_some_and(|hold| hold())
    }

    fn freshness_columns(&self) -> Vec<Rc<RefCell<ColumnFreshness>>> {
        self.monitors
            .borrow()
            .iter()
            .map(|monitor| monitor.freshness.clone())
            .collect()
    }

    pub(super) fn reschedule_freshness(self: &Rc<Self>) {
        let now = self.freshness_now();
        let due = self
            .checking()
            .then(|| {
                self.freshness_columns()
                    .iter()
                    .enumerate()
                    .filter_map(|(depth, column)| {
                        let column = column.borrow();
                        let owed = column
                            .quiet_until
                            .filter(|quiet| column.relist && *quiet > now);
                        self.watched(depth, &column)
                            .then(|| {
                                column
                                    .next_due
                                    .into_iter()
                                    .chain(column.confirm_at)
                                    .chain(owed)
                                    .min()
                            })
                            .flatten()
                    })
                    .min()
            })
            .flatten();
        if due == self.freshness.armed_for.get() {
            return;
        }
        self.freshness.armed_for.set(due);
        if let Some(timer) = self.freshness.timer.take() {
            timer.remove();
        }
        let Some(due) = due.filter(|_| !cfg!(any(test, feature = "test-support"))) else {
            return;
        };
        let wait = due.saturating_duration_since(now);
        let seconds = u32::try_from(wait.as_secs() + u64::from(wait.subsec_nanos() > 0))
            .unwrap_or(u32::MAX)
            .max(1);
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_seconds_local_once(seconds, move || {
            if let Some(browser) = weak.upgrade() {
                browser.freshness.timer.take();
                browser.freshness.armed_for.set(None);
                browser.freshness_tick();
            }
        });
        self.freshness.timer.replace(Some(timer));
    }
}

#[cfg(test)]
mod tests;

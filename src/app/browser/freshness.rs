// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use gtk::glib;

use crate::{
    app::{BrowserEvent, navigation::LoadState},
    model::Location,
    services::{
        DirectoryChange, DirectoryStamp, LoadHandle, MountToken, RequestId, Responsiveness,
        StampAnswer, file_activity,
    },
};

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

type Checked = (usize, Location, Weak<RefCell<ColumnFreshness>>);

#[derive(Clone, Copy)]
enum Finding {
    Unchanged,
    Stale,
    Departed,
    Inaccessible,
    Unreachable,
    Unanswered,
    Unsupported,
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
    #[cfg(test)]
    clock: Cell<Option<(Instant, Duration)>>,
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
        #[cfg(test)]
        if let Some((now, _)) = self.freshness.clock.get() {
            return now;
        }
        Instant::now()
    }

    fn boot_clock(&self) -> Duration {
        #[cfg(test)]
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
        let Some(Listing {
            stamp: Some((stamp, _)),
            digest: Some(digest),
            ..
        }) = column
            .listing
            .as_ref()
            .filter(|listing| Some(listing.request) == current)
        else {
            return;
        };
        let (Some(baseline), Some(before), Some(mount)) =
            (column.baseline.as_ref(), column.digest, stamp.mount.clone())
        else {
            return;
        };
        let moved = stamp.moved_since(baseline);
        let same_names = *digest == before;
        let lagged = column.names_may_lag;
        column.names_may_lag = same_names && (moved || lagged);
        let unchanged = !moved && !baseline.may_hide_changes();
        let cached = stamp.daemon_cache.is_some() || baseline.daemon_cache.is_some();
        if unchanged && !same_names && !lagged && !cached {
            BLIND_MOUNTS.with(|mounts| mounts.borrow_mut().insert(mount.clone()));
        }
        if blind(&mount) {
            column.unchanged = match unchanged && same_names {
                true => column.unchanged.saturating_add(1),
                false => 0,
            };
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

    pub(crate) fn operation_touches(location: &Location) -> bool {
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

    pub(crate) fn freshness_tick(self: &Rc<Self>) {
        let now = self.freshness_now();
        self.freshness
            .jobs
            .borrow_mut()
            .retain(|(unanswered, _)| unanswered.get() > 0);
        let mut groups: Vec<(Option<MountToken>, Vec<Checked>)> = Vec::new();
        if self.checking() {
            self.start_relists();
            for (depth, column) in self.freshness_columns().into_iter().enumerate() {
                let Some(location) = self.location_at(depth) else {
                    continue;
                };
                let due = |at: Option<Instant>| at.is_some_and(|at| at <= now);
                let (check, confirm, mount) = {
                    let column = column.borrow();
                    if !self.watched(depth, &column) {
                        continue;
                    }
                    (due(column.next_due), due(column.confirm_at), column.mount())
                };
                if !check && !confirm {
                    continue;
                }
                let later = |column: &ColumnFreshness| Some(now + self.column_interval(column));
                if mount.as_ref().is_some_and(blind) {
                    let mut column = column.borrow_mut();
                    column.relist = true;
                    column.confirm_at = None;
                    column.next_due = later(&column);
                    continue;
                }
                if !self.settled(depth) || Self::operation_touches(&location) {
                    let mut column = column.borrow_mut();
                    if check {
                        column.next_due = later(&column);
                    }
                    if confirm {
                        column.confirm_at = later(&column);
                    }
                    continue;
                }
                {
                    let mut column = column.borrow_mut();
                    if confirm {
                        column.confirm_at = None;
                        column.confirming = true;
                    }
                    column.checking = true;
                }
                let checked = (depth, location, Rc::downgrade(&column));
                match groups
                    .iter_mut()
                    .find(|(token, _)| mount.is_some() && *token == mount)
                {
                    Some((_, group)) => group.push(checked),
                    None => groups.push((mount, vec![checked])),
                }
            }
        }
        for (_, group) in groups {
            self.check_columns(group);
        }
        if self.checking() {
            self.start_relists();
        }
        self.reschedule_freshness();
        self.follow_responsiveness();
    }

    fn check_columns(self: &Rc<Self>, group: Vec<Checked>) {
        let mut locations: Vec<Location> = Vec::new();
        for (_, location, _) in &group {
            if !locations.contains(location) {
                locations.push(location.clone());
            }
        }
        let unanswered = Rc::new(Cell::new(locations.len()));
        let left = unanswered.clone();
        let weak = Rc::downgrade(self);
        let departed = Cell::new(None::<usize>);
        let emit = Rc::new(move |location: Location, answer: StampAnswer| {
            left.set(left.get().saturating_sub(1));
            let Some(browser) = weak.upgrade() else {
                return;
            };
            for (depth, _, column) in group.iter().filter(|(_, checked, _)| *checked == location) {
                let below_departed = departed.get().is_some_and(|above| above < *depth);
                if browser.apply_check(*depth, column, &location, answer.clone(), below_departed) {
                    departed.set(Some(*depth));
                }
            }
            browser.start_relists();
            browser.follow_responsiveness();
        });
        let job = self.source.check_stamps(locations, emit);
        if unanswered.get() > 0 {
            self.freshness.jobs.borrow_mut().push((unanswered, job));
        }
    }

    fn apply_check(
        self: &Rc<Self>,
        depth: usize,
        column: &Weak<RefCell<ColumnFreshness>>,
        location: &Location,
        answer: StampAnswer,
        below_departed: bool,
    ) -> bool {
        let Some(column) = column.upgrade() else {
            return false;
        };
        column.borrow_mut().checking = false;
        let current = self
            .column_freshness(depth)
            .is_some_and(|known| Rc::ptr_eq(&known, &column))
            && self.location_at(depth).as_ref() == Some(location);
        if !current {
            return false;
        }
        let now = self.freshness_now();
        let (finding, confirming) = {
            let mut column = column.borrow_mut();
            let finding = match (&answer, column.baseline.as_ref()) {
                (StampAnswer::Stamp(stamp), Some(baseline)) if stamp.replaced(baseline) => {
                    Finding::Departed
                }
                (StampAnswer::Stamp(stamp), Some(baseline)) if !stamp.moved_since(baseline) => {
                    Finding::Unchanged
                }
                (StampAnswer::Stamp(_), _) => Finding::Stale,
                (StampAnswer::Missing, _) => Finding::Departed,
                (StampAnswer::Inaccessible, _) => Finding::Inaccessible,
                (StampAnswer::Unreachable, _) => Finding::Unreachable,
                (StampAnswer::Unresponsive, _) => Finding::Unanswered,
                (StampAnswer::Unsupported, _) => Finding::Unsupported,
            };
            column.unchanged = match finding {
                Finding::Unchanged => column.unchanged.saturating_add(1),
                _ => 0,
            };
            let at_interval = now + self.check_interval(column.unchanged);
            column.next_due = match finding {
                Finding::Unsupported => None,
                Finding::Unreachable => Some(now + UNREACHABLE_RETRY),
                _ => Some(
                    column
                        .quiet_until
                        .map_or(at_interval, |quiet| at_interval.max(quiet)),
                ),
            };
            (finding, std::mem::take(&mut column.confirming))
        };
        let free = self.settled(depth) && !Self::operation_touches(location);
        let relist = match finding {
            Finding::Stale => true,
            Finding::Unchanged => confirming,
            _ => false,
        };
        {
            let mut column = column.borrow_mut();
            if relist && free && !self.freshness_held() {
                column.relist = true;
                column.confirming = confirming && matches!(finding, Finding::Unchanged);
            } else if confirming {
                column.confirm_at = column.next_due;
            }
        }
        let departed = match finding {
            Finding::Inaccessible if free && !self.freshness_held() => {
                self.refresh_column_preserving_entries(depth);
                false
            }
            Finding::Departed if free && !below_departed => {
                self.handle_root_departure(
                    depth,
                    location,
                    DirectoryChange::Remove(location.clone()),
                );
                true
            }
            _ => false,
        };
        self.reschedule_freshness();
        departed
    }

    fn start_relists(self: &Rc<Self>) {
        while let Some(depth) = self.next_relist() {
            self.refresh_keeping_listing(depth, true);
        }
    }

    fn next_relist(&self) -> Option<usize> {
        let now = self.freshness_now();
        let columns = self.freshness_columns();
        let busy: Vec<MountToken> = columns
            .iter()
            .filter_map(|column| {
                let column = column.borrow();
                let working = column.checking
                    || column
                        .listing
                        .as_ref()
                        .is_some_and(|listing| listing.background);
                working.then(|| column.mount()).flatten()
            })
            .collect();
        for (depth, column) in columns.iter().enumerate() {
            let mut column = column.borrow_mut();
            if !column.relist
                || column.mount().is_some_and(|mount| busy.contains(&mount))
                || column.quiet_until.is_some_and(|quiet| quiet > now)
            {
                continue;
            }
            if self.settled(depth)
                && self
                    .location_at(depth)
                    .is_some_and(|location| !Self::operation_touches(&location))
                && !self.freshness_held()
            {
                column.relist = false;
                return Some(depth);
            }
        }
        None
    }

    pub(super) fn column_not_responding(&self, depth: usize) -> bool {
        self.column_freshness(depth)
            .is_some_and(|column| column.borrow().not_responding)
    }

    pub(super) fn follow_lanes(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let lanes = self.source.follow_responsiveness(Rc::new(move || {
            if let Some(browser) = weak.upgrade() {
                browser.follow_responsiveness();
            }
        }));
        self.freshness.lanes.replace(Some(lanes));
    }

    pub(crate) fn follow_responsiveness(self: &Rc<Self>) {
        let mut follow = false;
        let mut changed = Vec::new();
        let mut answered = Vec::new();
        for (depth, column) in self.freshness_columns().into_iter().enumerate() {
            let Some(location) = self.location_at(depth) else {
                continue;
            };
            let responsiveness = self.source.responsiveness(&location);
            let (loading, failed) = match self
                .state
                .borrow()
                .columns
                .get(depth)
                .map(|column| &column.load_state)
            {
                Some(LoadState::Loading) => (true, false),
                Some(LoadState::Error(_) | LoadState::Denied { .. }) => (false, true),
                _ => (false, false),
            };
            let answering = responsiveness == Responsiveness::Answering;
            let not_responding = responsiveness == Responsiveness::NotResponding && !failed;
            let mut column = column.borrow_mut();
            if !loading
                && answering
                && column.retry_when_answering
                && (failed || (!self.freshness_held() && !Self::operation_touches(&location)))
            {
                column.retry_when_answering = false;
                answered.push((depth, failed));
            }
            follow |= loading || column.checking || !answering || column.retry_when_answering;
            if column.not_responding != not_responding {
                column.not_responding = not_responding;
                changed.push((depth, not_responding));
            }
        }
        for (depth, not_responding) in changed {
            self.emit(BrowserEvent::ColumnNotResponding {
                depth,
                not_responding,
            });
        }
        self.keep_following_responsiveness(follow);
        for (depth, failed) in answered {
            match failed {
                true => self.retry_column(depth),
                false => self.refresh_keeping_listing(depth, false),
            }
        }
    }

    fn keep_following_responsiveness(self: &Rc<Self>, follow: bool) {
        let follow = follow && self.freshness.input.get().presence != Presence::Hidden;
        if follow == self.freshness.following.replace(follow) {
            return;
        }
        if let Some(timer) = self.freshness.follow_timer.take() {
            timer.remove();
        }
        if !follow || cfg!(test) {
            return;
        }
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_seconds_local_once(1, move || {
            if let Some(browser) = weak.upgrade() {
                browser.freshness.follow_timer.take();
                browser.freshness.following.set(false);
                browser.follow_responsiveness();
            }
        });
        self.freshness.follow_timer.replace(Some(timer));
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
        let Some(due) = due.filter(|_| !cfg!(test)) else {
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

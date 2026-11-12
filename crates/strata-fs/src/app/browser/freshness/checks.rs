// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Instant,
};

use crate::{
    model::Location,
    services::{DirectoryChange, DirectoryStamp, MountToken, StampAnswer},
};

use super::{Browser, ColumnFreshness, UNREACHABLE_RETRY, blind};

/// A column whose stamp one check asks for, held weakly so a closed column
/// ignores its late answer.
struct Checked {
    depth: usize,
    location: Location,
    column: Weak<RefCell<ColumnFreshness>>,
}

/// One check in flight for a group of columns on the same mount; `departed`
/// remembers the shallowest folder found gone so deeper ones do not repeat it.
struct StampCheck {
    group: Vec<Checked>,
    departed: Cell<Option<usize>>,
}

/// Which of a column's deadlines a tick found due.
#[derive(Clone, Copy)]
struct Due {
    check: bool,
    confirm: bool,
}

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

impl Finding {
    fn of(answer: &StampAnswer, baseline: Option<&DirectoryStamp>) -> Self {
        match (answer, baseline) {
            (StampAnswer::Stamp(stamp), Some(baseline)) if stamp.replaced(baseline) => {
                Self::Departed
            }
            (StampAnswer::Stamp(stamp), Some(baseline)) if !stamp.moved_since(baseline) => {
                Self::Unchanged
            }
            (StampAnswer::Stamp(_), _) => Self::Stale,
            (StampAnswer::Missing, _) => Self::Departed,
            (StampAnswer::Inaccessible, _) => Self::Inaccessible,
            (StampAnswer::Unreachable, _) => Self::Unreachable,
            (StampAnswer::Unresponsive, _) => Self::Unanswered,
            (StampAnswer::Unsupported, _) => Self::Unsupported,
        }
    }

    fn relists(self, confirming: bool) -> bool {
        match self {
            Self::Stale => true,
            Self::Unchanged => confirming,
            _ => false,
        }
    }
}

/// What one answer means for its column: the finding, whether the column was
/// confirming a quiet stamp, and whether it is free to be relisted right now.
#[derive(Clone, Copy)]
struct Verdict {
    finding: Finding,
    confirming: bool,
    free: bool,
}

/// Columns grouped by mount, so one check answers every column of a share.
#[derive(Default)]
struct CheckGroups(Vec<(Option<MountToken>, Vec<Checked>)>);

impl CheckGroups {
    fn add(&mut self, mount: Option<MountToken>, checked: Checked) {
        match self
            .0
            .iter_mut()
            .find(|(token, _)| mount.is_some() && *token == mount)
        {
            Some((_, group)) => group.push(checked),
            None => self.0.push((mount, vec![checked])),
        }
    }
}

fn checked_locations(group: &[Checked]) -> Vec<Location> {
    let mut locations: Vec<Location> = Vec::new();
    for checked in group {
        if !locations.contains(&checked.location) {
            locations.push(checked.location.clone());
        }
    }
    locations
}

fn busy_mounts(columns: &[Rc<RefCell<ColumnFreshness>>]) -> Vec<MountToken> {
    columns
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
        .collect()
}

fn begin_check(column: &mut ColumnFreshness, confirm: bool) {
    if confirm {
        column.confirm_at = None;
        column.confirming = true;
    }
    column.checking = true;
}

impl Browser {
    pub(crate) fn freshness_tick(self: &Rc<Self>) {
        let now = self.freshness_now();
        self.freshness
            .jobs
            .borrow_mut()
            .retain(|(unanswered, _)| unanswered.get() > 0);
        let mut groups = CheckGroups::default();
        if self.checking() {
            self.start_relists();
            for (depth, column) in self.freshness_columns().into_iter().enumerate() {
                if let Some((mount, checked)) = self.due_check(now, depth, &column) {
                    groups.add(mount, checked);
                }
            }
        }
        for (_, group) in groups.0 {
            self.check_columns(group);
        }
        if self.checking() {
            self.start_relists();
        }
        self.reschedule_freshness();
        self.follow_responsiveness();
    }

    /// A blind mount relists instead of checking, a busy column waits for its
    /// next interval, and the rest join a stamp check.
    fn due_check(
        &self,
        now: Instant,
        depth: usize,
        column: &Rc<RefCell<ColumnFreshness>>,
    ) -> Option<(Option<MountToken>, Checked)> {
        let location = self.location_at(depth)?;
        let (due, mount) = self.column_due(now, depth, &column.borrow())?;
        if mount.as_ref().is_some_and(blind) {
            self.relist_blind_column(now, &mut column.borrow_mut());
            return None;
        }
        if !self.settled(depth) || Self::operation_touches(&location) {
            self.postpone_check(now, &mut column.borrow_mut(), due);
            return None;
        }
        begin_check(&mut column.borrow_mut(), due.confirm);
        let checked = Checked {
            depth,
            location,
            column: Rc::downgrade(column),
        };
        Some((mount, checked))
    }

    fn column_due(
        &self,
        now: Instant,
        depth: usize,
        column: &ColumnFreshness,
    ) -> Option<(Due, Option<MountToken>)> {
        if !self.watched(depth, column) {
            return None;
        }
        let due = |at: Option<Instant>| at.is_some_and(|at| at <= now);
        let due = Due {
            check: due(column.next_due),
            confirm: due(column.confirm_at),
        };
        (due.check || due.confirm).then(|| (due, column.mount()))
    }

    fn relist_blind_column(&self, now: Instant, column: &mut ColumnFreshness) {
        column.relist = true;
        column.confirm_at = None;
        column.next_due = Some(now + self.column_interval(column));
    }

    fn postpone_check(&self, now: Instant, column: &mut ColumnFreshness, due: Due) {
        if due.check {
            column.next_due = Some(now + self.column_interval(column));
        }
        if due.confirm {
            column.confirm_at = Some(now + self.column_interval(column));
        }
    }

    fn check_columns(self: &Rc<Self>, group: Vec<Checked>) {
        let locations = checked_locations(&group);
        let unanswered = Rc::new(Cell::new(locations.len()));
        let left = unanswered.clone();
        let weak = Rc::downgrade(self);
        let check = StampCheck {
            group,
            departed: Cell::new(None),
        };
        let emit = Rc::new(move |location: Location, answer: StampAnswer| {
            left.set(left.get().saturating_sub(1));
            let Some(browser) = weak.upgrade() else {
                return;
            };
            browser.apply_answer(&check, &location, answer);
        });
        let job = self.source.check_stamps(locations, emit);
        if unanswered.get() > 0 {
            self.freshness.jobs.borrow_mut().push((unanswered, job));
        }
    }

    fn apply_answer(self: &Rc<Self>, check: &StampCheck, location: &Location, answer: StampAnswer) {
        for checked in check
            .group
            .iter()
            .filter(|checked| checked.location == *location)
        {
            let below_departed = check
                .departed
                .get()
                .is_some_and(|above| above < checked.depth);
            if self.apply_check(checked, answer.clone(), below_departed) {
                check.departed.set(Some(checked.depth));
            }
        }
        self.start_relists();
        self.follow_responsiveness();
    }

    fn apply_check(
        self: &Rc<Self>,
        checked: &Checked,
        answer: StampAnswer,
        below_departed: bool,
    ) -> bool {
        let Some(column) = checked.column.upgrade() else {
            return false;
        };
        column.borrow_mut().checking = false;
        if !self.still_checks(checked, &column) {
            return false;
        }
        let (finding, confirming) = self.note_finding(&mut column.borrow_mut(), &answer);
        let verdict = Verdict {
            finding,
            confirming,
            free: self.settled(checked.depth) && !Self::operation_touches(&checked.location),
        };
        self.plan_relist(&mut column.borrow_mut(), verdict);
        let departed = self.follow_finding(checked, verdict, below_departed);
        self.reschedule_freshness();
        departed
    }

    /// The column may have been closed or reused for another folder meanwhile.
    fn still_checks(&self, checked: &Checked, column: &Rc<RefCell<ColumnFreshness>>) -> bool {
        self.column_freshness(checked.depth)
            .is_some_and(|known| Rc::ptr_eq(&known, column))
            && self.location_at(checked.depth).as_ref() == Some(&checked.location)
    }

    fn note_finding(&self, column: &mut ColumnFreshness, answer: &StampAnswer) -> (Finding, bool) {
        let now = self.freshness_now();
        let finding = Finding::of(answer, column.baseline.as_ref());
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
    }

    fn plan_relist(&self, column: &mut ColumnFreshness, verdict: Verdict) {
        let relist =
            verdict.finding.relists(verdict.confirming) && verdict.free && !self.freshness_held();
        if relist {
            column.relist = true;
            column.confirming = verdict.confirming && matches!(verdict.finding, Finding::Unchanged);
        } else if verdict.confirming {
            column.confirm_at = column.next_due;
        }
    }

    /// Returns whether the folder itself is gone.
    fn follow_finding(
        self: &Rc<Self>,
        checked: &Checked,
        verdict: Verdict,
        below_departed: bool,
    ) -> bool {
        match verdict.finding {
            Finding::Inaccessible if verdict.free && !self.freshness_held() => {
                self.refresh_column_preserving_entries(checked.depth);
                false
            }
            Finding::Departed if verdict.free && !below_departed => {
                self.handle_root_departure(
                    checked.depth,
                    &checked.location,
                    DirectoryChange::Remove(checked.location.clone()),
                );
                true
            }
            _ => false,
        }
    }

    pub(super) fn start_relists(self: &Rc<Self>) {
        while let Some(depth) = self.next_relist() {
            self.refresh_keeping_listing(depth, true);
        }
    }

    fn next_relist(&self) -> Option<usize> {
        let now = self.freshness_now();
        let columns = self.freshness_columns();
        let busy = busy_mounts(&columns);
        for (depth, column) in columns.iter().enumerate() {
            let mut column = column.borrow_mut();
            let blocked = !column.relist
                || column.mount().is_some_and(|mount| busy.contains(&mount))
                || column.quiet_until.is_some_and(|quiet| quiet > now);
            if blocked {
                continue;
            }
            if self.may_relist(depth) {
                column.relist = false;
                return Some(depth);
            }
        }
        None
    }

    fn may_relist(&self, depth: usize) -> bool {
        self.settled(depth)
            && self
                .location_at(depth)
                .is_some_and(|location| !Self::operation_touches(&location))
            && !self.freshness_held()
    }
}

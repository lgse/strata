// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use crate::{
    app::{BrowserEvent, navigation::LoadState},
    model::Location,
    services::Responsiveness,
};

use super::{Browser, ColumnFreshness, Presence};

/// What one pass over the columns found: whether any still needs following,
/// which changed their not-responding state, and which may retry a refresh
/// now that their source answers (with whether their load had failed).
#[derive(Default)]
struct Sweep {
    follow: bool,
    changed: Vec<(usize, bool)>,
    answered: Vec<(usize, bool)>,
}

/// A column's load state beside its source's responsiveness.
#[derive(Clone, Copy)]
struct Lane {
    loading: bool,
    answering: bool,
}

impl Lane {
    fn retries(&self, column: &ColumnFreshness) -> bool {
        !self.loading && self.answering && column.retry_when_answering
    }

    fn needs_following(&self, column: &ColumnFreshness) -> bool {
        self.loading || column.checking || !self.answering || column.retry_when_answering
    }
}

impl Browser {
    pub(in crate::app::browser) fn column_not_responding(&self, depth: usize) -> bool {
        self.column_freshness(depth)
            .is_some_and(|column| column.borrow().not_responding)
    }

    pub(in crate::app::browser) fn follow_lanes(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let lanes = self.source.follow_responsiveness(Rc::new(move || {
            if let Some(browser) = weak.upgrade() {
                browser.follow_responsiveness();
            }
        }));
        self.freshness.lanes.replace(Some(lanes));
    }

    pub fn follow_responsiveness(self: &Rc<Self>) {
        let mut sweep = Sweep::default();
        for (depth, column) in self.freshness_columns().into_iter().enumerate() {
            self.sweep_column(depth, &column, &mut sweep);
        }
        for (depth, not_responding) in sweep.changed {
            self.emit(BrowserEvent::ColumnNotResponding {
                depth,
                not_responding,
            });
        }
        self.keep_following_responsiveness(sweep.follow);
        for (depth, failed) in sweep.answered {
            match failed {
                true => self.retry_column(depth),
                false => self.refresh_keeping_listing(depth, false),
            }
        }
    }

    fn sweep_column(&self, depth: usize, column: &Rc<RefCell<ColumnFreshness>>, sweep: &mut Sweep) {
        let Some(location) = self.location_at(depth) else {
            return;
        };
        let responsiveness = self.source.responsiveness(&location);
        let (loading, failed) = self.load_progress(depth);
        let lane = Lane {
            loading,
            answering: responsiveness == Responsiveness::Answering,
        };
        let not_responding = responsiveness == Responsiveness::NotResponding && !failed;
        let mut column = column.borrow_mut();
        if lane.retries(&column) && self.may_retry(&location, failed) {
            column.retry_when_answering = false;
            sweep.answered.push((depth, failed));
        }
        sweep.follow |= lane.needs_following(&column);
        if column.not_responding != not_responding {
            column.not_responding = not_responding;
            sweep.changed.push((depth, not_responding));
        }
    }

    /// Whether the column is loading, and whether its load failed.
    fn load_progress(&self, depth: usize) -> (bool, bool) {
        match self
            .state
            .borrow()
            .columns
            .get(depth)
            .map(|column| &column.load_state)
        {
            Some(LoadState::Loading) => (true, false),
            Some(LoadState::Error(_) | LoadState::Denied { .. }) => (false, true),
            _ => (false, false),
        }
    }

    /// A failed load always retries; a kept listing waits for a quiet folder.
    fn may_retry(&self, location: &Location, failed: bool) -> bool {
        failed || (!self.freshness_held() && !Self::operation_touches(location))
    }

    pub(super) fn keep_following_responsiveness(self: &Rc<Self>, follow: bool) {
        let follow = follow && self.freshness.input.get().presence != Presence::Hidden;
        if follow == self.freshness.following.replace(follow) {
            return;
        }
        if let Some(timer) = self.freshness.follow_timer.take() {
            timer.remove();
        }
        if !follow || cfg!(any(test, feature = "test-support")) {
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
}

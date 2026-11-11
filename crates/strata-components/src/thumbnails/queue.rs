//! Priority: what each viewer shows and the margin around it, ranked into requests.

use std::collections::HashSet;
use std::ops::Range;

use super::step::Step;
use super::store::{Inner, Job, ThumbnailStore, Ticket};
use crate::contract::{EntryId, EntrySource, ThumbKey};

/// How far around a viewer's visible rows, in viewports per side, pruning leaves handles
/// alone. Rune keeps an index-built list's rows mounted up to two viewports past the viewport
/// (`OVERSCAN` in `el/virtual_list.rs` at the pinned rev), and rows drawn during motion up to
/// two more; a row mounted outside this guard still builds again on the commit `sync` asks
/// for after a prune (see [`ThumbnailStore::sync`]).
const GUARD: usize = 5;

/// One view showing entries from the store: a list, a grid, a Columns column, a peek.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewerId(u32);

/// What a viewer shows, as [`ThumbnailStore::sync`] takes it.
///
/// Fields may be added: build one with [`Viewport::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Viewport {
    /// The entries in the viewport, by index in the source: C1e's `on_visible` range, or
    /// `CollectionState::visible_range`.
    pub visible: Range<usize>,
    /// Entries a line: 1 for a list, the tiles a row for a grid. The margin is whole lines.
    pub per_line: usize,
    /// The step the viewer draws at.
    pub step: Step,
    /// The host's listing version (C1e's): ranges of two versions aren't compared for the
    /// scroll direction.
    pub listing: u64,
}

impl Viewport {
    /// `visible` in a list drawn at `step`, listing version 0.
    pub fn new(visible: Range<usize>, step: Step) -> Self {
        Self {
            visible,
            per_line: 1,
            step,
            listing: 0,
        }
    }

    /// With `n` entries a line.
    pub fn per_line(mut self, n: usize) -> Self {
        self.per_line = n.max(1);
        self
    }

    /// With listing version `v`.
    pub fn listing(mut self, v: u64) -> Self {
        self.listing = v;
        self
    }
}

/// Which way a viewer last scrolled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// Toward the end of the listing (and the start, before any scroll).
    #[default]
    Down,
    /// Toward its start.
    Up,
}

/// How near a request's row is to what its viewer shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// In the viewport.
    Visible,
    /// In the margin the viewer is scrolling toward.
    Ahead,
    /// In the margin behind it.
    Behind,
}

/// A thumbnail to load: render `key` (the entry `entry`) with its long edge at most `step`,
/// then hand the result to [`ThumbnailStore::complete`] with `ticket`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Request {
    /// What to hand back.
    pub ticket: Ticket,
    /// The thumbnail key.
    pub key: ThumbKey,
    /// The entry it was asked for (the first viewer's, when two show it).
    pub entry: EntryId,
    /// The step to render at.
    pub step: Step,
    /// How near its row is.
    pub tier: Tier,
}

pub(crate) type Keyed = Vec<(ThumbKey, EntryId)>;

/// A key in `pop`'s ranked list: its tier, its entry and the step its viewer draws at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Queued {
    tier: Tier,
    key: ThumbKey,
    entry: EntryId,
}

#[derive(Default)]
pub(crate) struct Viewer {
    /// The viewport last synced, for the early-out (and the step its keys are wanted at).
    view: Option<Viewport>,
    /// The next sync recomputes the lists even for an unchanged viewport (new settings).
    stale: bool,
    /// The viewport before that one's direction was judged against.
    last: Option<Viewport>,
    direction: Direction,
    visible: Keyed,
    prev_visible: Vec<ThumbKey>,
    ahead: Keyed,
    behind: Keyed,
}

impl Viewer {
    /// Makes the next sync recompute the lists even for an unchanged viewport. The viewport
    /// itself stays: its keys are still wanted at its step, and a resync still has it.
    pub(crate) fn stale(&mut self) {
        self.stale = true;
    }

    /// Shows `to` wherever this viewer showed `from` (a relocated key), until its next sync.
    pub(crate) fn rename(&mut self, from: ThumbKey, to: ThumbKey) {
        for list in [&mut self.visible, &mut self.ahead, &mut self.behind] {
            for (k, _) in list.iter_mut() {
                if *k == from {
                    *k = to;
                }
            }
        }
        for k in &mut self.prev_visible {
            if *k == from {
                *k = to;
            }
        }
    }

    fn tier(&self, tier: Tier) -> &Keyed {
        match tier {
            Tier::Visible => &self.visible,
            Tier::Ahead => &self.ahead,
            Tier::Behind => &self.behind,
        }
    }
}

impl ThumbnailStore {
    /// A new viewer. Drop it with [`ThumbnailStore::drop_viewer`] when its view goes.
    pub fn viewer(&mut self) -> ViewerId {
        let viewers = &mut self.inner.get_mut().viewers;
        viewers.push(Some(Viewer::default()));
        ViewerId((viewers.len() - 1) as u32)
    }

    /// Forgets a viewer: what only it showed is no longer protected or asked for.
    pub fn drop_viewer(&mut self, v: ViewerId) {
        let inner = self.inner.get_mut();
        if let Some(slot) = inner.viewers.get_mut(v.0 as usize) {
            *slot = None;
        }
        inner.rewant();
        inner.enforce();
        // What it wanted no longer holds room: margins the budget dropped may fit now.
        inner.deferred.clear();
    }

    /// The direction viewer `v` last scrolled.
    pub fn direction(&self, v: ViewerId) -> Direction {
        let inner = self.inner.borrow();
        inner
            .viewers
            .get(v.0 as usize)
            .and_then(Option::as_ref)
            .map_or(Direction::Down, |w| w.direction)
    }

    /// Records what viewer `v` shows from `src`. Call it from C1e's `on_visible` (and from
    /// `poll`, cheaply: an unchanged viewport returns at once).
    ///
    /// The viewport's keys are touched (drawn, least-recently-used order) and protected from
    /// eviction, with the previous sync's: Rune reports a reflowed grid's rows a frame late
    /// (C1e's draft U-C1f-5). Requests out for keys no viewer shows or has in its margin any
    /// more are cancelled ([`ThumbnailStore::take_cancelled`]).
    ///
    /// True only when the store pruned handles it no longer needed: then the host's handler
    /// returns true, so the view runs once and any row still drawing a pruned key builds on
    /// its new handles. That happens once per few thousand keys scrolled past.
    pub fn sync(&mut self, v: ViewerId, src: &dyn EntrySource, view: Viewport) -> bool {
        self.sync_with(v, src, view, false)
    }

    /// Syncs viewer `v` again after its source changed (a refresh, a filter, a re-sort),
    /// with its last viewport and listing version `listing`.
    pub fn resync(&mut self, v: ViewerId, src: &dyn EntrySource, listing: u64) -> bool {
        let last = {
            let inner = self.inner.borrow();
            inner
                .viewers
                .get(v.0 as usize)
                .and_then(Option::as_ref)
                .and_then(|w| w.view.clone())
        };
        match last {
            Some(view) => self.sync_with(v, src, view.listing(listing), true),
            None => false,
        }
    }

    /// Syncs viewer `v` again at a new step with its last viewport: a zoom or a change of
    /// display scale that moved no row.
    pub fn set_step(&mut self, v: ViewerId, src: &dyn EntrySource, step: Step) -> bool {
        let last = {
            let inner = self.inner.borrow();
            inner
                .viewers
                .get(v.0 as usize)
                .and_then(Option::as_ref)
                .and_then(|w| w.view.clone().or_else(|| w.last.clone()))
        };
        match last {
            Some(mut view) => {
                view.step = step;
                self.sync_with(v, src, view, false)
            }
            None => false,
        }
    }

    fn sync_with(
        &mut self,
        v: ViewerId,
        src: &dyn EntrySource,
        view: Viewport,
        force: bool,
    ) -> bool {
        let inner = self.inner.get_mut();
        let margin = inner.settings.margin;
        let Some(Some(w)) = inner.viewers.get_mut(v.0 as usize) else {
            return false;
        };
        if !force && !w.stale && w.view.as_ref() == Some(&view) {
            return false;
        }
        w.stale = false;
        if let Some(last) = &w.last
            && last.listing == view.listing
            && last.per_line == view.per_line
        {
            if view.visible.start > last.visible.start {
                w.direction = Direction::Down;
            } else if view.visible.start < last.visible.start {
                w.direction = Direction::Up;
            }
        } else if w.last.as_ref().is_none_or(|l| l.listing != view.listing) {
            // A new listing starts afresh; a reflow (another count a line) moves `start`
            // without a scroll, so it keeps the direction.
            w.direction = Direction::Down;
        }
        let (visible, above, below) = spans(&view, src.len(), margin);
        let ((ahead, ahead_above), (behind, behind_above)) = match w.direction {
            Direction::Down => ((below, false), (above, true)),
            Direction::Up => ((above, true), (below, false)),
        };
        let prev = std::mem::take(&mut w.visible);
        w.prev_visible = prev.into_iter().map(|(k, _)| k).collect();
        w.visible = keyed(src, visible.clone());
        w.ahead = keyed_lines(src, &ahead, view.per_line, ahead_above);
        w.behind = keyed_lines(src, &behind, view.per_line, behind_above);
        // A resync's range is the old listing's: the next sync starts the direction afresh.
        w.last = (!force).then(|| view.clone());
        w.view = Some(view);
        let shown: Vec<ThumbKey> = w.visible.iter().map(|(k, _)| *k).collect();
        let margins: Vec<ThumbKey> = w.ahead.iter().chain(&w.behind).map(|(k, _)| *k).collect();
        for k in margins {
            inner.ensure(k);
        }
        for k in shown {
            inner.ensure(k);
            inner.touch(k);
        }
        inner.deferred.clear();
        inner.rewant();
        inner.enforce();
        inner.prune(src, visible)
    }

    /// The best request to work on next, or `None` when nothing is wanted (or thumbnails are
    /// off). The host pulls as many as it has workers for, from `poll`.
    ///
    /// Order: every viewer's visible keys in reading order, then the margin each viewer is
    /// scrolling toward, nearest line first, then the margin behind, nearest first. Within a
    /// tier, keys never loaded go before keys held at another step. Keys in a request, failed
    /// less than [`Settings`](super::Settings)' `retry_after` ago or stopped are skipped. Margin
    /// keys wait while the rows on screen fill the budget.
    pub fn pop(&mut self, now: f64) -> Option<Request> {
        let inner = self.inner.get_mut();
        if !inner.enabled {
            return None;
        }
        if now >= inner.next_expiry {
            inner.dirty = true;
        }
        if inner.dirty {
            inner.rank(now);
        }
        let budget = inner.settings.budget_bytes;
        while inner.cursor < inner.queue.len() {
            let Queued { tier, key, entry } = inner.queue[inner.cursor];
            inner.cursor += 1;
            #[cfg(test)]
            {
                inner.scans += 1;
            }
            if inner.in_flight.contains(&key) || inner.deferred.contains(&key) {
                continue;
            }
            let Some(&step) = inner.wanted.get(&key) else {
                continue;
            };
            let Some(e) = inner.entries.get_mut(&key) else {
                continue;
            };
            match e.job {
                Job::Requested { .. } | Job::Stopped => continue,
                // Checked again here (the ranking checked it too): the wait may have begun since.
                Job::Failed { until } if now < until => {
                    inner.next_expiry = inner.next_expiry.min(until);
                    continue;
                }
                Job::Failed { .. } | Job::Idle => {}
            }
            if e.held.is_some_and(|h| h.step == step) {
                continue;
            }
            let room = step.max_bytes();
            let needs = inner
                .pinned
                .saturating_add(inner.margin_out_bytes)
                .saturating_add(room);
            if tier != Tier::Visible && needs > budget {
                // A margin image could only land by evicting a key some viewer wants: wait
                // until a request ends, a wanted image goes, or the viewport moves.
                inner.gated = true;
                continue;
            }
            let n = inner.next_ticket;
            inner.next_ticket += 1;
            let e = inner.entries.get_mut(&key).expect("checked above");
            e.job = Job::Requested { n, step };
            inner.in_flight.insert(key);
            if tier != Tier::Visible {
                inner.margin_out.insert(key, room);
                inner.margin_out_bytes += room;
            }
            return Some(Request {
                ticket: Ticket { key, step, n },
                key,
                entry,
                step,
                tier,
            });
        }
        None
    }
}

impl Viewer {
    fn view_step(&self) -> Step {
        self.view.as_ref().map_or(Step::new(1), |v| v.step)
    }
}

impl Inner {
    /// Ranks every wanted key that may need a request: by tier, every viewer's viewport
    /// before any margin, keys never loaded before keys held at another step. `pop` then
    /// walks it once; it is ranked again only when something it skipped could have changed.
    fn rank(&mut self, now: f64) {
        self.queue.clear();
        self.cursor = 0;
        self.dirty = false;
        self.gated = false;
        self.next_expiry = f64::INFINITY;
        for tier in [Tier::Visible, Tier::Ahead, Tier::Behind] {
            for restep in [false, true] {
                for w in self.viewers.iter().flatten() {
                    for &(key, entry) in w.tier(tier) {
                        let Some(e) = self.entries.get(&key) else {
                            continue;
                        };
                        let step = self.wanted.get(&key).copied().unwrap_or(w.view_step());
                        match e.job {
                            Job::Requested { .. } | Job::Stopped => continue,
                            Job::Failed { until } if now < until => {
                                self.next_expiry = self.next_expiry.min(until);
                                continue;
                            }
                            Job::Failed { .. } | Job::Idle => {}
                        }
                        let ranked = match e.held {
                            None => !restep,
                            Some(h) => restep && h.step != step,
                        };
                        if ranked {
                            self.queue.push(Queued { tier, key, entry });
                        }
                    }
                }
            }
        }
    }

    /// Recomputes what every viewer wants and protects, and cancels requests nobody wants.
    pub fn rewant(&mut self) {
        self.wanted.clear();
        self.protected.clear();
        for w in self.viewers.iter().flatten() {
            let step = w.view_step();
            for (k, _) in w.visible.iter().chain(&w.ahead).chain(&w.behind) {
                let s = self.wanted.entry(*k).or_insert(step);
                *s = (*s).max(step);
            }
            self.protected.extend(w.visible.iter().map(|(k, _)| *k));
            self.protected.extend(w.prev_visible.iter().copied());
        }
        let stale: Vec<ThumbKey> = self
            .in_flight
            .iter()
            .copied()
            .filter(|k| !self.wanted.contains_key(k))
            .collect();
        for k in stale {
            self.cancel(k);
        }
        // A margin key now on screen needs no margin room.
        let shown: Vec<ThumbKey> = self
            .margin_out
            .keys()
            .copied()
            .filter(|k| self.protected.contains(k))
            .collect();
        for k in shown {
            if let Some(b) = self.margin_out.remove(&k) {
                self.margin_out_bytes -= b;
            }
        }
        self.repin();
        self.dirty = true;
    }

    /// Drops handles that hold nothing and wait for nothing, once there are many, except
    /// those any viewer wants or protects and those near `visible` in `src`. True if any
    /// went.
    fn prune(&mut self, src: &dyn EntrySource, visible: Range<usize>) -> bool {
        let busy = self.lru.len() + self.in_flight.len();
        if self.entries.len().saturating_sub(busy) <= self.prune_slack {
            return false;
        }
        let span = visible.len().max(1) * GUARD;
        let near = visible.start.saturating_sub(span)..(visible.end + span).min(src.len());
        let mut keep: HashSet<ThumbKey> = near.filter_map(|i| src.entry(i).thumbnail).collect();
        keep.extend(self.wanted.keys().copied());
        keep.extend(self.protected.iter().copied());
        let before = self.entries.len();
        // Failed and stopped keys go too: near a viewer they keep their wait, and one seen
        // again after a prune is just asked for again.
        self.entries.retain(|k, e| {
            e.held.is_some() || matches!(e.job, Job::Requested { .. }) || keep.contains(k)
        });
        self.entries.len() != before
    }
}

/// The visible range clamped to `len`, and the margins above and below it, in whole lines.
fn spans(view: &Viewport, len: usize, margin: f32) -> (Range<usize>, Range<usize>, Range<usize>) {
    let start = view.visible.start.min(len);
    let end = view.visible.end.min(len).max(start);
    let per = view.per_line.max(1);
    let lines = (end - start).div_ceil(per);
    // `as` saturates (and takes NaN to 0), so an absurd margin is the whole listing.
    let m = ((lines as f32 * margin.max(0.0)).ceil() as usize)
        .saturating_mul(per)
        .min(len);
    let above = start.saturating_sub(m)..start;
    let below = end..(end + m).min(len);
    (start..end, above, below)
}

fn keyed(src: &dyn EntrySource, range: Range<usize>) -> Keyed {
    range
        .filter_map(|i| {
            let e = src.entry(i);
            Some((e.thumbnail?, e.id))
        })
        .collect()
}

/// The margin `range`, nearest line first: below the viewport that is reading order; above
/// it, the lines from the last one up, each line still in reading order.
fn keyed_lines(src: &dyn EntrySource, range: &Range<usize>, per_line: usize, above: bool) -> Keyed {
    if !above {
        return keyed(src, range.clone());
    }
    let per = per_line.max(1);
    let mut out = Keyed::new();
    let mut end = range.end;
    while end > range.start {
        let start = end.saturating_sub(per).max(range.start);
        out.extend(keyed(src, start..end));
        end = start;
    }
    out
}

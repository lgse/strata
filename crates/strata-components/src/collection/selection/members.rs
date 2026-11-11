//! Which ids are selected: an id set plus an optional overlay of index ranges.
//!
//! A range operation over 100,000 entries can't afford to hash every id it covers (that alone
//! takes over a millisecond). So a range lands as an overlay: the index ranges it covers, with a
//! plain copy of their ids. While the source keeps its order, membership is a range check on
//! the index. [`Members::freeze`] (on every sync) hashes the copied ids, because after a re-sort
//! or a live update the indices no longer name the same entries, and the copy is all that
//! still says which entries the range held.
//!
//! On top of both sits a small set of flipped ids: Space inside a 10xer `v`/`V` range flips an
//! entry over whatever the range makes of it, and the flip holds while later steps replace the
//! range ([`Members::flip_over`]). An entry is selected if `op(base, overlay)` says so, unless
//! it is flipped.

use std::collections::HashSet;
use std::hash::{BuildHasherDefault, Hasher};

use crate::contract::{EntryId, EntrySource, IndexRanges};

/// A set of entry ids, hashed with [`IdHasher`].
pub(super) type IdSet = HashSet<EntryId, BuildHasherDefault<IdHasher>>;

/// A hasher for [`EntryId`]s: splitmix64's finaliser over the raw value. Hosts often number
/// ids in sequence, which the standard SipHash handles well but slowly; this mixes as well for
/// a single `u64` at a fraction of the cost.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // EntryId hashes through `write_u64`; this only runs for other keys, byte by byte.
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }

    fn write_u64(&mut self, x: u64) {
        let mut z = (self.0 ^ x).wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        self.0 = z ^ (z >> 31);
    }
}

/// How an overlay combines with the base set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Op {
    /// Selected if in the base or the overlay.
    Add,
    /// Selected if in the base and not the overlay.
    Remove,
    /// Selected if in exactly one of them.
    Toggle,
}

impl Op {
    fn apply(self, in_base: bool, in_overlay: bool) -> bool {
        match self {
            Self::Add => in_base || in_overlay,
            Self::Remove => in_base && !in_overlay,
            Self::Toggle => in_base != in_overlay,
        }
    }
}

#[derive(Clone, Debug)]
struct Overlay {
    op: Op,
    ids: OverlayIds,
}

#[derive(Clone, Debug)]
enum OverlayIds {
    /// Index ranges into the source as it was when they were captured, with their ids in
    /// index order. Valid until the source changes, which a sync marks.
    Live {
        ranges: IndexRanges,
        ids: Vec<EntryId>,
        source_len: usize,
    },
    /// The ids alone, after the source changed.
    Frozen(IdSet),
}

impl Overlay {
    fn contains_at<S: EntrySource + ?Sized>(&self, source: &S, index: usize, id: EntryId) -> bool {
        match &self.ids {
            OverlayIds::Live {
                ranges, source_len, ..
            } => {
                debug_assert_eq!(
                    source.len(),
                    *source_len,
                    "the source changed without a Selection::sync"
                );
                let inside = ranges.contains(index);
                debug_assert!(
                    !inside || self.copied_at(index) == Some(id),
                    "the source changed without a Selection::sync"
                );
                inside
            }
            OverlayIds::Frozen(set) => set.contains(&id),
        }
    }

    /// The id a live overlay copied for `index`, if its ranges cover it.
    fn copied_at(&self, index: usize) -> Option<EntryId> {
        let OverlayIds::Live { ranges, ids, .. } = &self.ids else {
            return None;
        };
        let mut offset = 0;
        for r in ranges.ranges() {
            if r.contains(&index) {
                return ids.get(offset + index - r.start).copied();
            }
            offset += r.len();
        }
        None
    }

    /// Whether every one of a live overlay's copied ids is still at its index in `source`:
    /// O(the range), for the paths verbs act on. A frozen overlay doesn't depend on the source.
    fn matches_exactly<S: EntrySource + ?Sized>(&self, source: &S) -> bool {
        let OverlayIds::Live {
            ranges,
            ids,
            source_len,
        } = &self.ids
        else {
            return true;
        };
        source.len() == *source_len
            && ranges
                .indices()
                .zip(ids)
                .all(|(i, &id)| source.id_at(i) == id)
    }

    fn contains<S: EntrySource + ?Sized>(&self, source: &S, id: EntryId) -> bool {
        match &self.ids {
            OverlayIds::Live {
                ranges, source_len, ..
            } => {
                debug_assert_eq!(
                    source.len(),
                    *source_len,
                    "the source changed without a Selection::sync"
                );
                source.index_of(id).is_some_and(|i| ranges.contains(i))
            }
            OverlayIds::Frozen(set) => set.contains(&id),
        }
    }

    /// Whether `id` is among the overlay's ids, read from its copy rather than the source.
    fn holds(&self, id: EntryId) -> bool {
        match &self.ids {
            OverlayIds::Live { ids, .. } => ids.contains(&id),
            OverlayIds::Frozen(set) => set.contains(&id),
        }
    }

    /// Whether a live overlay's indices still name its copied ids in `source`, as far as a
    /// cheap check can tell: the same length, and the same first and last ids. A missed
    /// [`Selection::sync`](super::Selection::sync) that re-sorted the source usually fails
    /// it; [`Overlay::matches_exactly`] catches the rest. A frozen overlay doesn't depend on
    /// the source.
    fn matches<S: EntrySource + ?Sized>(&self, source: &S) -> bool {
        let OverlayIds::Live {
            ranges,
            ids,
            source_len,
        } = &self.ids
        else {
            return true;
        };
        debug_assert_eq!(
            source.len(),
            *source_len,
            "the source changed without a Selection::sync"
        );
        let (Some(first), Some(last)) = (ranges.ranges().first(), ranges.ranges().last()) else {
            return true;
        };
        source.len() == *source_len
            && ids.first() == Some(&source.id_at(first.start))
            && ids.last() == Some(&source.id_at(last.end - 1))
    }

    fn for_each_id(&self, mut f: impl FnMut(EntryId)) {
        match &self.ids {
            OverlayIds::Live { ids, .. } => ids.iter().copied().for_each(&mut f),
            OverlayIds::Frozen(set) => set.iter().copied().for_each(&mut f),
        }
    }
}

/// The selected ids: `base`, combined with `overlay` by its [`Op`], with `flips` flipped.
#[derive(Clone, Debug, Default)]
pub(super) struct Members {
    base: IdSet,
    overlay: Option<Overlay>,
    /// Ids flipped over what the base and the overlay make: Space inside an extend run.
    /// Usually empty; [`Members::commit`] folds them into the base with the overlay.
    flips: IdSet,
    /// How many ids are selected, kept exact by every change.
    len: usize,
}

impl Members {
    pub(super) fn len(&self) -> usize {
        self.len
    }

    fn flipped(&self, id: &EntryId) -> bool {
        !self.flips.is_empty() && self.flips.contains(id)
    }

    /// Whether the base and the overlay select `id`, before the flips, reading the overlay's
    /// copied ids: O(the overlay) for a live one, so callers freeze it first.
    fn held_before_flips(&self, id: EntryId) -> bool {
        let in_base = self.base.contains(&id);
        match &self.overlay {
            None => in_base,
            Some(ov) => ov.op.apply(in_base, ov.holds(id)),
        }
    }

    /// Whether the entry at `index` is selected.
    pub(super) fn contains_at<S: EntrySource + ?Sized>(&self, source: &S, index: usize) -> bool {
        let id = source.id_at(index);
        let in_base = !self.base.is_empty() && self.base.contains(&id);
        let ranged = match &self.overlay {
            None => in_base,
            Some(ov) => ov.op.apply(in_base, ov.contains_at(source, index, id)),
        };
        ranged != self.flipped(&id)
    }

    /// Whether `id` is selected. An id the source doesn't list can still be selected while a
    /// listing loads.
    pub(super) fn contains<S: EntrySource + ?Sized>(&self, source: &S, id: EntryId) -> bool {
        let in_base = self.base.contains(&id);
        let ranged = match &self.overlay {
            None => in_base,
            Some(ov) => ov.op.apply(in_base, ov.contains(source, id)),
        };
        ranged != self.flipped(&id)
    }

    /// Selects nothing. Returns whether anything was selected.
    pub(super) fn clear(&mut self) -> bool {
        let changed = self.len > 0;
        self.base.clear();
        self.overlay = None;
        self.flips.clear();
        self.len = 0;
        changed
    }

    /// Selects only `id`. Returns whether that changed anything.
    pub(super) fn set_single<S: EntrySource + ?Sized>(&mut self, source: &S, id: EntryId) -> bool {
        let changed = !(self.len == 1 && self.contains(source, id));
        self.base.clear();
        self.base.insert(id);
        self.overlay = None;
        self.flips.clear();
        self.len = 1;
        changed
    }

    /// Folds the overlay and the flips into the base, so the base alone says what is selected.
    pub(super) fn commit(&mut self) {
        let base = &mut self.base;
        if let Some(ov) = self.overlay.take() {
            ov.for_each_id(|id| match ov.op {
                Op::Add => {
                    base.insert(id);
                }
                Op::Remove => {
                    base.remove(&id);
                }
                Op::Toggle => {
                    if !base.remove(&id) {
                        base.insert(id);
                    }
                }
            });
        }
        for id in self.flips.drain() {
            if !base.remove(&id) {
                base.insert(id);
            }
        }
        debug_assert_eq!(self.base.len(), self.len, "selection count drifted");
        self.len = self.base.len();
    }

    /// Flips `id` over what the base and the overlay select, keeping both: an extend run
    /// replaces its overlay on every step, and the flip holds through those steps.
    pub(super) fn flip_over<S: EntrySource + ?Sized>(&mut self, source: &S, id: EntryId) {
        let was_selected = self.contains(source, id);
        if !self.flips.remove(&id) {
            self.flips.insert(id);
        }
        if was_selected {
            self.len -= 1;
        } else {
            self.len += 1;
        }
    }

    /// Flips `id` in or out.
    pub(super) fn flip(&mut self, id: EntryId) {
        self.commit();
        if !self.base.remove(&id) {
            self.base.insert(id);
        }
        self.len = self.base.len();
    }

    /// Adds `id`. Returns whether it wasn't selected before.
    pub(super) fn insert(&mut self, id: EntryId) -> bool {
        self.commit();
        let added = self.base.insert(id);
        self.len = self.base.len();
        added
    }

    /// Renames `from` to `to` wherever the selection holds it: in the base, in the overlay's
    /// copied ids and in the flips. An open gesture goes on over the renamed id. Returns
    /// whether `from` was selected.
    ///
    /// `to` is a new id, so the selection doesn't hold it yet (debug builds check). If it
    /// does, the two merge and the count follows.
    pub(super) fn relocate(&mut self, from: EntryId, to: EntryId) -> bool {
        let holds = |m: &Self, id: EntryId| {
            m.base.contains(&id)
                || m.overlay.as_ref().is_some_and(|ov| ov.holds(id))
                || m.flips.contains(&id)
        };
        let already = holds(self, to);
        debug_assert!(
            !already,
            "relocate onto {to:?}, which the selection already holds"
        );
        if already {
            // Copied ids must stay one per index, so merge in a set.
            self.freeze();
        }
        let in_base = self.base.remove(&from);
        if in_base {
            self.base.insert(to);
        }
        let mut was_selected = in_base;
        if let Some(ov) = &mut self.overlay {
            let in_overlay = match &mut ov.ids {
                OverlayIds::Live { ids, .. } => match ids.iter_mut().find(|id| **id == from) {
                    Some(id) => {
                        *id = to;
                        true
                    }
                    None => false,
                },
                OverlayIds::Frozen(set) => {
                    let held = set.remove(&from);
                    if held {
                        set.insert(to);
                    }
                    held
                }
            };
            was_selected = ov.op.apply(in_base, in_overlay);
        }
        if self.flips.remove(&from) {
            self.flips.insert(to);
            was_selected = !was_selected;
        }
        if already {
            // `to` was held already, so it and `from` may have merged into one member.
            self.recount();
        }
        was_selected
    }

    /// Adds `id` to the base alone, leaving an open gesture's overlay and flips to apply over
    /// it, as they do over the rest of the base. Returns whether that selected it.
    pub(super) fn insert_base(&mut self, id: EntryId) -> bool {
        self.freeze();
        let selected = |m: &Self| m.held_before_flips(id) != m.flipped(&id);
        let before = selected(self);
        self.base.insert(id);
        self.recount();
        selected(self) != before
    }

    /// Replaces the overlay with `ranges`, combined by `op` with the base and under the flips,
    /// or with nothing if `clear_base`, which drops the flips too. Returns whether the
    /// selected set changed.
    ///
    /// Callers fold an overlay from another gesture into the base first ([`Members::commit`]):
    /// the overlay this replaces belongs to the same gesture, so the base is that gesture's
    /// starting point.
    pub(super) fn set_ranges<S: EntrySource + ?Sized>(
        &mut self,
        source: &S,
        clear_base: bool,
        op: Op,
        ranges: IndexRanges,
    ) -> bool {
        let mut ids = Vec::with_capacity(ranges.count());
        ids.extend(ranges.indices().map(|i| source.id_at(i)));

        let base_after = !clear_base && !self.base.is_empty();
        let flips_after = !clear_base && !self.flips.is_empty();
        let ranged_after = |id: &EntryId| {
            let in_base = base_after && self.base.contains(id);
            let in_ranges = source.index_of(*id).is_some_and(|i| ranges.contains(i));
            op.apply(in_base, in_ranges)
        };
        let ranged_len = if base_after {
            let inside = ids.iter().filter(|id| self.base.contains(id)).count();
            match op {
                Op::Add => self.base.len() + (ids.len() - inside),
                Op::Remove => self.base.len() - inside,
                Op::Toggle => self.base.len() + (ids.len() - inside) - inside,
            }
        } else {
            match op {
                Op::Add | Op::Toggle => ids.len(),
                Op::Remove => 0,
            }
        };
        // Each flip takes one member out of the ranged set or adds one to it.
        let flips = self.flips.iter().filter(|_| flips_after);
        let new_len = flips.fold(
            ranged_len,
            |len, id| {
                if ranged_after(id) { len - 1 } else { len + 1 }
            },
        );

        let changed = match self.delta_since_live(source, op, clear_base, &ranges) {
            Some(changed) => changed,
            None if new_len != self.len => true,
            None => {
                // Same count: the sets differ only if some new member wasn't selected before.
                let new_contains =
                    |id: &EntryId| ranged_after(id) != (flips_after && self.flips.contains(id));
                let base_ids = self.base.iter().filter(|_| base_after);
                let flipped_ids = self.flips.iter().filter(|_| flips_after);
                base_ids
                    .chain(ids.iter())
                    .chain(flipped_ids)
                    .any(|id| new_contains(id) && !self.contains(source, *id))
            }
        };

        if clear_base {
            self.base.clear();
            self.flips.clear();
        }
        self.overlay = (!ids.is_empty()).then(|| Overlay {
            op,
            ids: OverlayIds::Live {
                ranges,
                ids,
                source_len: source.len(),
            },
        });
        self.len = new_len;
        changed
    }

    /// Whether replacing the live overlay with `ranges` changes the set, worked out from the
    /// indices the two cover differently: `None` when there's no live overlay of the same `op`
    /// over the same base and flips to compare with.
    fn delta_since_live<S: EntrySource + ?Sized>(
        &self,
        source: &S,
        op: Op,
        clear_base: bool,
        ranges: &IndexRanges,
    ) -> Option<bool> {
        let Some(
            old @ Overlay {
                op: old_op,
                ids: OverlayIds::Live {
                    ranges: old_ranges, ..
                },
            },
        ) = &self.overlay
        else {
            return None;
        };
        let drops = clear_base && !(self.base.is_empty() && self.flips.is_empty());
        if *old_op != op || drops || !old.matches(source) {
            // A missed sync leaves the old indices meaningless, possibly past the end.
            return None;
        }
        let delta = symmetric_difference(old_ranges, ranges);
        let in_base = |i: usize| self.base.contains(&source.id_at(i));
        Some(match op {
            Op::Toggle => !delta.is_empty(),
            Op::Add => delta.indices().any(|i| !in_base(i)),
            Op::Remove => !self.base.is_empty() && delta.indices().any(in_base),
        })
    }

    /// Drops the overlay, leaving the base and the flips. Returns whether the selected set
    /// changed.
    pub(super) fn discard_overlay(&mut self) -> bool {
        let Some(ov) = self.overlay.take() else {
            return false;
        };
        let mut changed = false;
        ov.for_each_id(|id| {
            // A flip turns both sides over, so it can't make them differ.
            let in_base = self.base.contains(&id);
            changed |= ov.op.apply(in_base, true) != in_base;
        });
        self.recount();
        changed
    }

    /// Selects exactly the listed entries that aren't selected now. Returns whether that
    /// changed anything.
    pub(super) fn invert<S: EntrySource + ?Sized>(&mut self, source: &S) -> bool {
        let mut ranges = IndexRanges::new();
        let mut ids = Vec::new();
        for i in 0..source.len() {
            if !self.contains_at(source, i) {
                ranges.push(i..i + 1);
                ids.push(source.id_at(i));
            }
        }
        let changed = source.len() > 0 || self.len > 0;
        self.base.clear();
        self.flips.clear();
        self.len = ids.len();
        self.overlay = (!ids.is_empty()).then(|| Overlay {
            op: Op::Add,
            ids: OverlayIds::Live {
                ranges,
                ids,
                source_len: source.len(),
            },
        });
        changed
    }

    /// Turns a live overlay's index ranges into its ids, because the source is changing.
    pub(super) fn freeze(&mut self) {
        if let Some(ov) = &mut self.overlay
            && let OverlayIds::Live { ids, .. } = &mut ov.ids
        {
            let mut set = IdSet::with_capacity_and_hasher(ids.len(), Default::default());
            set.extend(ids.iter().copied());
            ov.ids = OverlayIds::Frozen(set);
        }
    }

    /// Drops every id the source doesn't list. Returns whether any was selected.
    pub(super) fn retain_listed<S: EntrySource + ?Sized>(&mut self, source: &S) -> bool {
        self.freeze();
        let listed = |id: &EntryId| source.index_of(*id).is_some();
        self.base.retain(listed);
        if let Some(Overlay {
            ids: OverlayIds::Frozen(set),
            ..
        }) = &mut self.overlay
        {
            set.retain(listed);
        }
        self.flips.retain(listed);
        let before = self.len;
        self.recount();
        self.len != before
    }

    /// Counts the members again. Reads a live overlay's copied ids once per flip, so callers
    /// with flips freeze it first.
    fn recount(&mut self) {
        let ranged_len = match &self.overlay {
            None => self.base.len(),
            Some(ov) => {
                let (mut inside, mut outside) = (0, 0);
                ov.for_each_id(|id| {
                    if self.base.contains(&id) {
                        inside += 1;
                    } else {
                        outside += 1;
                    }
                });
                match ov.op {
                    Op::Add => self.base.len() + outside,
                    Op::Remove => self.base.len() - inside,
                    Op::Toggle => self.base.len() + outside - inside,
                }
            }
        };
        self.len = self.flips.iter().fold(ranged_len, |len, &id| {
            if self.held_before_flips(id) {
                len - 1
            } else {
                len + 1
            }
        });
    }

    /// The selected ids the source lists, in its order.
    pub(super) fn listed_in_order<S: EntrySource + ?Sized>(&self, source: &S) -> Vec<EntryId> {
        if self.len == 0 {
            return Vec::new();
        }
        if let Some(ov) = &self.overlay
            && !ov.matches_exactly(source)
        {
            // A missed sync: what verbs act on must not come from stale indices, so read the
            // copied ids instead, as a sync would have.
            let mut copied = IdSet::default();
            ov.for_each_id(|id| {
                copied.insert(id);
            });
            let selected = |id: &EntryId| {
                ov.op.apply(self.base.contains(id), copied.contains(id)) != self.flipped(id)
            };
            let flipped_only = self
                .flips
                .iter()
                .filter(|id| !self.base.contains(id) && !copied.contains(id));
            let mut found: Vec<(usize, EntryId)> = self
                .base
                .union(&copied)
                .chain(flipped_only)
                .filter(|id| selected(id))
                .filter_map(|&id| source.index_of(id).map(|i| (i, id)))
                .collect();
            found.sort_unstable_by_key(|&(i, _)| i);
            return found.into_iter().map(|(_, id)| id).collect();
        }
        let n = source.len();
        // A few ids among many entries: sort them by index rather than scan the source.
        if self.overlay.is_none() && self.flips.is_empty() && self.len.saturating_mul(16) < n {
            let mut found: Vec<(usize, EntryId)> = self
                .base
                .iter()
                .filter_map(|&id| source.index_of(id).map(|i| (i, id)))
                .collect();
            found.sort_unstable_by_key(|&(i, _)| i);
            return found.into_iter().map(|(_, id)| id).collect();
        }
        (0..n)
            .filter(|&i| self.contains_at(source, i))
            .map(|i| source.id_at(i))
            .collect()
    }

    /// Whether the selection is exactly `ids`. Freezes a live overlay first, which needs no
    /// source: its copied ids are its members.
    pub(super) fn equals(&mut self, ids: &IdSet) -> bool {
        self.freeze();
        if self.len != ids.len() {
            return false;
        }
        ids.iter()
            .all(|id| self.held_before_flips(*id) != self.flipped(id))
    }

    /// Selects exactly `ids`.
    pub(super) fn set_ids(&mut self, ids: IdSet) {
        self.len = ids.len();
        self.base = ids;
        self.overlay = None;
        self.flips.clear();
    }
}

/// The indices in exactly one of `a` and `b`.
fn symmetric_difference(a: &IndexRanges, b: &IndexRanges) -> IndexRanges {
    let mut points: Vec<usize> = a
        .ranges()
        .iter()
        .chain(b.ranges())
        .flat_map(|r| [r.start, r.end])
        .collect();
    points.sort_unstable();
    points.dedup();
    let mut out = IndexRanges::new();
    for w in points.windows(2) {
        if a.contains(w[0]) != b.contains(w[0]) {
            out.push(w[0]..w[1]);
        }
    }
    out
}

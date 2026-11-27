// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod members;
mod restore;

#[cfg(test)]
mod tests;

use std::ops::{BitOr, BitOrAssign};
use std::sync::atomic::{AtomicU64, Ordering};

pub use restore::{Position, RestoreMemory, ScrollMemo};

use self::members::{IdSet, Members, Op};
use crate::contract::{EntryId, EntrySource, IndexRanges};

/// What an operation changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Changed {
    /// Which entries are selected, or whether they count as a fill (a provisional pick becoming a real one), changed.
    pub selection: bool,
    /// The cursor moved to another entry, or to none.
    pub cursor: bool,
    /// The anchor moved to another entry, or to none.
    pub anchor: bool,
}

impl Changed {
    /// Nothing changed.
    pub const NONE: Self = Self {
        selection: false,
        cursor: false,
        anchor: false,
    };

    /// Whether anything changed.
    pub const fn any(self) -> bool {
        self.selection || self.cursor || self.anchor
    }
}

impl BitOr for Changed {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            selection: self.selection || other.selection,
            cursor: self.cursor || other.cursor,
            anchor: self.anchor || other.anchor,
        }
    }
}

impl BitOrAssign for Changed {
    fn bitor_assign(&mut self, other: Self) {
        *self = *self | other;
    }
}

/// How `Selection::extend_to` combines the range from the anchor with the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Extend {
    /// The range replaces the selection: Shift+click, Shift+arrow.
    Replace,
    /// The range adds to the selection as it was when this run of extends began: Ctrl+Shift+click, 10xer `v`.
    Add,
    /// The range removes from the selection as it was when this run of extends began: 10xer `V`.
    Remove,
}

/// Whether `Selection::move_cursor` moves the anchor with the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnchorRule {
    /// The anchor stays.
    Keep,
    /// The anchor moves to the new cursor.
    Move,
}

/// How a marquee band combines with the selection at press.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BandMode {
    /// The band replaces the selection, which [`Selection::begin_band`] clears.
    Replace,
    /// Each crossed entry flips relative to the selection at press (Ctrl).
    Toggle,
    /// The crossed entries add to the selection at press (Shift).
    Add,
}

/// Whether the source holds its whole listing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Listing {
    /// Every entry is listed: ids the source lacks are gone.
    #[default]
    Complete,
    /// A (re)load is still delivering entries: ids the source lacks may still arrive.
    Loading,
}

/// Where the cursor goes when its entry leaves a complete listing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorGone {
    /// The neighbour takes the cursor, unselected: a reload, a 10xer `d` or `M`, a plain move out of the folder.
    #[default]
    Neighbor,
    /// The neighbour takes the cursor and joins the selection: a delete in the default map.
    NeighborSelected,
    /// No entry takes the cursor: a Columns column whose child column is open.
    Clear,
}

/// How `Selection::sync` treats the source it is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SyncRule {
    /// Whether the listing is complete.
    pub listing: Listing,
    /// Where the cursor goes if its entry left.
    pub cursor_gone: CursorGone,
}

impl SyncRule {
    /// A complete listing, with `cursor_gone` for a cursor whose entry left.
    pub const fn complete(cursor_gone: CursorGone) -> Self {
        Self {
            listing: Listing::Complete,
            cursor_gone,
        }
    }

    /// A listing still loading.
    pub const fn loading() -> Self {
        Self {
            listing: Listing::Loading,
            cursor_gone: CursorGone::Neighbor,
        }
    }
}

/// What a 10xer file verb acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Targets {
    /// The filled selection, in display order.
    Filled(Vec<EntryId>),
    /// Nothing is filled: the cursor's entry.
    Cursor(EntryId),
    /// Neither: an empty folder, or no cursor.
    Nothing,
}

/// The gesture the next operation may continue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Gesture {
    #[default]
    None,
    /// A run of [`Selection::extend_to`] calls in one mode, from one anchor.
    Extend(Extend),
    /// A marquee between [`Selection::begin_band`] and its end.
    Band(BandMode),
}

/// One collection's selection, cursor and anchor, by entry id.
#[derive(Clone, Debug)]
pub struct Selection {
    members: Members,
    gesture: Gesture,
    cursor: Option<EntryId>,
    /// The cursor's index when last seen, for the reload rule once its entry is gone.
    cursor_hint: Option<usize>,
    removal_hint: Option<(EntryId, usize)>,
    anchor: Option<EntryId>,
    provisional: bool,
    revision: u64,
}

/// The process-wide source of [`Selection::revision`] values.
static REVISIONS: AtomicU64 = AtomicU64::new(0);

fn next_revision() -> u64 {
    REVISIONS.fetch_add(1, Ordering::Relaxed) + 1
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            members: Members::default(),
            gesture: Gesture::None,
            cursor: None,
            cursor_hint: None,
            removal_hint: None,
            anchor: None,
            provisional: false,
            revision: next_revision(),
        }
    }
}

impl Selection {
    /// Nothing selected, no cursor, no anchor.
    pub fn new() -> Self {
        Self::default()
    }

    // Reading.

    /// The cursor's entry.
    pub fn cursor(&self) -> Option<EntryId> {
        self.cursor
    }

    /// The anchor's entry.
    pub fn anchor(&self) -> Option<EntryId> {
        self.anchor
    }

    /// Where the cursor is in `source`.
    pub fn cursor_index<S: EntrySource + ?Sized>(&self, source: &S) -> Option<usize> {
        self.cursor.and_then(|id| source.index_of(id))
    }

    /// Where the anchor is in `source`.
    pub fn anchor_index<S: EntrySource + ?Sized>(&self, source: &S) -> Option<usize> {
        self.anchor.and_then(|id| source.index_of(id))
    }

    /// Whether the entry at `index` is selected. Cheap enough to call for every drawn row.
    pub fn is_selected_at<S: EntrySource + ?Sized>(&self, source: &S, index: usize) -> bool {
        index < source.len() && self.members.contains_at(source, index)
    }

    /// Whether `id` is selected.
    pub fn contains<S: EntrySource + ?Sized>(&self, source: &S, id: EntryId) -> bool {
        self.members.contains(source, id)
    }

    /// Whether `id` is held in the selection's base, range overlay, or run flips, without consulting an `EntrySource`.
    pub fn holds(&self, id: EntryId) -> bool {
        self.members.holds(id)
    }

    /// How many entries are selected.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the selection is a load's automatic pick (`Selection::auto_select`), which shows as selected but isn't a fill.
    pub fn is_provisional(&self) -> bool {
        self.provisional
    }

    /// Whether a marquee band gesture is open (`begin_band` before its release or cancel).
    pub fn in_band(&self) -> bool {
        matches!(self.gesture, Gesture::Band(_))
    }

    /// A value that goes up whenever an operation reports a change, unique across the process.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The selected entries, in display order: what the default map's verbs and a drag act on.
    pub fn selected_ids<S: EntrySource + ?Sized>(&self, source: &S) -> Vec<EntryId> {
        self.members.listed_in_order(source)
    }

    /// What a 10xer verb acts on: the fill, else the cursor's entry.
    pub fn targets<S: EntrySource + ?Sized>(&self, source: &S) -> Targets {
        if !self.provisional && !self.is_empty() {
            let filled = self.selected_ids(source);
            if !filled.is_empty() {
                return Targets::Filled(filled);
            }
        }
        match self.cursor {
            Some(id) if source.index_of(id).is_some() => Targets::Cursor(id),
            _ => Targets::Nothing,
        }
    }

    // Pointer and keyboard.

    /// Selects only the entry at `index`, and moves the cursor and the anchor to it.
    pub fn select_only<S: EntrySource + ?Sized>(&mut self, source: &S, index: usize) -> Changed {
        self.pick(source, index, false)
    }

    pub(crate) fn select_only_keeping_anchor<S: EntrySource + ?Sized>(
        &mut self,
        source: &S,
        index: usize,
        missing_stays: bool,
    ) -> Changed {
        let anchor = self.anchor;
        if anchor.is_none() && !missing_stays {
            return self.select_only(source, index);
        }
        let mut changed = self.select_only(source, index);
        self.anchor = anchor;
        changed.anchor = false;
        changed
    }

    pub(crate) fn focus_selected_keeping_anchor<S: EntrySource + ?Sized>(
        &mut self,
        source: &S,
        index: usize,
    ) -> Changed {
        let Some(id) = listed(source, index) else {
            return Changed::NONE;
        };
        self.gesture = Gesture::None;
        let mut changed = Changed {
            selection: self.set_provisional(false),
            ..Changed::NONE
        };
        changed |= self.place_cursor(id, index);
        self.finish(changed)
    }

    /// Selects the entry at `index` as a load's automatic pick (provisional), with the cursor and the anchor on it.
    pub fn auto_select<S: EntrySource + ?Sized>(&mut self, source: &S, index: usize) -> Changed {
        self.pick(source, index, true)
    }

    fn pick<S: EntrySource + ?Sized>(
        &mut self,
        source: &S,
        index: usize,
        provisional: bool,
    ) -> Changed {
        let Some(id) = listed(source, index) else {
            return Changed::NONE;
        };
        self.gesture = Gesture::None;
        let mut changed = Changed {
            selection: self.members.set_single(source, id) | self.set_provisional(provisional),
            ..Changed::NONE
        };
        changed |= self.place_cursor(id, index);
        changed.anchor |= self.set_anchor(Some(id));
        self.finish(changed)
    }

    /// Flips the entry at `index` in or out of the selection, and moves the cursor and the anchor to it.
    pub fn toggle<S: EntrySource + ?Sized>(&mut self, source: &S, index: usize) -> Changed {
        let Some(id) = listed(source, index) else {
            return Changed::NONE;
        };
        self.gesture = Gesture::None;
        self.provisional = false;
        self.members.flip(id);
        let mut changed = Changed {
            selection: true,
            ..Changed::NONE
        };
        changed |= self.place_cursor(id, index);
        changed.anchor |= self.set_anchor(Some(id));
        self.finish(changed)
    }

    /// Flips the entry at `index` inside an `Extend::Add` or `Extend::Remove` run: 10xer Space inside a `v` or `V` range.
    pub fn toggle_in_run<S: EntrySource + ?Sized>(&mut self, source: &S, index: usize) -> Changed {
        if !matches!(self.gesture, Gesture::Extend(Extend::Add | Extend::Remove)) {
            return self.toggle(source, index);
        }
        let Some(id) = listed(source, index) else {
            return Changed::NONE;
        };
        self.members.flip_over(source, id);
        let changed = Changed {
            selection: true,
            ..Changed::NONE
        };
        self.finish(changed)
    }

    pub(crate) fn extend_keeping_anchor<S: EntrySource + ?Sized>(
        &mut self,
        source: &S,
        index: usize,
        mode: Extend,
        missing_stays: bool,
    ) -> Changed {
        let missing = missing_stays && self.anchor.is_none();
        let mut changed = self.extend_to(source, index, mode);
        if missing {
            self.anchor = None;
            changed.anchor = false;
        }
        changed
    }

    /// Selects the range from the anchor to the entry at `index` by `mode`, and moves the cursor there.
    pub fn extend_to<S: EntrySource + ?Sized>(
        &mut self,
        source: &S,
        index: usize,
        mode: Extend,
    ) -> Changed {
        let Some(id) = listed(source, index) else {
            return Changed::NONE;
        };
        let mut changed = Changed::NONE;
        let anchor = match self.anchor_index(source) {
            Some(a) => a,
            None => {
                changed.anchor |= self.set_anchor(Some(id));
                index
            }
        };
        if self.gesture != Gesture::Extend(mode) && mode != Extend::Replace {
            self.members.commit();
        }
        self.gesture = Gesture::Extend(mode);
        let op = match mode {
            Extend::Replace | Extend::Add => Op::Add,
            Extend::Remove => Op::Remove,
        };
        let range = IndexRanges::from(anchor.min(index)..anchor.max(index) + 1);
        changed.selection = self
            .members
            .set_ranges(source, mode == Extend::Replace, op, range)
            | self.set_provisional(false);
        changed |= self.place_cursor(id, index);
        self.finish(changed)
    }

    /// Moves the cursor to the entry at `index` and keeps the selection.
    pub fn move_cursor<S: EntrySource + ?Sized>(
        &mut self,
        source: &S,
        index: usize,
        anchor: AnchorRule,
    ) -> Changed {
        let Some(id) = listed(source, index) else {
            return Changed::NONE;
        };
        self.gesture = Gesture::None;
        let mut changed = Changed::NONE;
        if self.provisional && self.cursor != Some(id) {
            self.provisional = false;
            changed.selection = self.members.clear();
        }
        changed |= self.place_cursor(id, index);
        if anchor == AnchorRule::Move {
            changed.anchor |= self.set_anchor(Some(id));
        }
        self.finish(changed)
    }

    /// Selects every entry `source` lists now; entries listed later stay unselected.
    pub fn select_all<S: EntrySource + ?Sized>(&mut self, source: &S) -> Changed {
        let n = source.len();
        if n == 0 {
            return Changed::NONE;
        }
        self.gesture = Gesture::None;
        let mut changed = Changed {
            selection: self
                .members
                .set_ranges(source, true, Op::Add, (0..n).into())
                | self.set_provisional(false),
            ..Changed::NONE
        };
        if self.cursor.is_none() {
            changed |= self.place_cursor(source.id_at(0), 0);
        }
        if self.anchor.is_none() {
            changed.anchor |= self.set_anchor(self.cursor);
        }
        self.finish(changed)
    }

    /// Selects exactly the listed entries that weren't selected: 10xer Ctrl+R.
    pub fn invert<S: EntrySource + ?Sized>(&mut self, source: &S) -> Changed {
        self.gesture = Gesture::None;
        if self.provisional {
            let _ = self.members.clear();
        }
        self.provisional = false;
        let changed = Changed {
            selection: self.members.invert(source),
            ..Changed::NONE
        };
        self.finish(changed)
    }

    /// Selects nothing and keeps the cursor and the anchor: Escape, or a plain press on empty space.
    pub fn clear(&mut self) -> Changed {
        self.gesture = Gesture::None;
        self.provisional = false;
        let changed = Changed {
            selection: self.members.clear(),
            ..Changed::NONE
        };
        self.finish(changed)
    }

    /// Drops a provisional pick, so the next toggle adds the cursor's entry: 10xer Space ("the first Space adds that item").
    pub fn discard_provisional(&mut self) -> Changed {
        if !self.provisional {
            return Changed::NONE;
        }
        self.gesture = Gesture::None;
        self.provisional = false;
        let changed = Changed {
            selection: self.members.clear(),
            ..Changed::NONE
        };
        self.finish(changed)
    }

    // Marquee.

    /// Starts a marquee band.
    pub fn begin_band(&mut self, mode: BandMode) -> Changed {
        self.members.commit();
        self.gesture = Gesture::Band(mode);
        let mut changed = Changed {
            selection: self.set_provisional(false),
            ..Changed::NONE
        };
        if mode == BandMode::Replace {
            changed.selection |= self.members.clear();
        }
        self.finish(changed)
    }

    /// Whether a marquee band is open: between `Selection::begin_band` and `Selection::end_band` or `Selection::cancel_band`.
    pub fn is_banding(&self) -> bool {
        matches!(self.gesture, Gesture::Band(_))
    }

    /// Applies the entries the band crosses now (indices into `source`) to the selection at press.
    pub fn band<S: EntrySource + ?Sized>(&mut self, source: &S, crossed: &IndexRanges) -> Changed {
        let Gesture::Band(mode) = self.gesture else {
            return Changed::NONE;
        };
        let op = match mode {
            BandMode::Replace | BandMode::Add => Op::Add,
            BandMode::Toggle => Op::Toggle,
        };
        let mut ranges = IndexRanges::new();
        let n = source.len();
        for r in crossed.ranges() {
            ranges.push(r.start.min(n)..r.end.min(n));
        }
        let changed = Changed {
            selection: self.members.set_ranges(source, false, op, ranges),
            ..Changed::NONE
        };
        self.finish(changed)
    }

    /// Ends a band, keeping what it selected.
    pub fn end_band(&mut self) {
        if matches!(self.gesture, Gesture::Band(_)) {
            self.gesture = Gesture::None;
        }
    }

    /// Ends a band and goes back to the selection at press (Escape during a marquee).
    pub fn cancel_band(&mut self) -> Changed {
        if !matches!(self.gesture, Gesture::Band(_)) {
            return Changed::NONE;
        }
        self.gesture = Gesture::None;
        let changed = Changed {
            selection: self.members.discard_overlay(),
            ..Changed::NONE
        };
        self.finish(changed)
    }

    // Source changes.

    /// Brings the selection in step with `source` after it changed.
    pub fn sync<S: EntrySource + ?Sized>(&mut self, source: &S, rule: SyncRule) -> Changed {
        self.members.freeze();
        let mut changed = Changed::NONE;
        if rule.listing == Listing::Loading {
            if let Some(i) = self.cursor_index(source) {
                self.cursor_hint = Some(i);
            }
            return self.finish(changed);
        }

        changed.selection = self.members.retain_listed(source);
        let removal = self.removal_hint.take();
        match self.cursor {
            Some(id) => match source.index_of(id) {
                Some(i) => self.cursor_hint = Some(i),
                None => {
                    let survivors = removal.and_then(|(at, n)| (at == id).then_some(n));
                    changed |= self.cursor_left(source, rule.cursor_gone, survivors);
                }
            },
            None => self.cursor_hint = None,
        }
        let anchor_left = self.anchor.is_some() && self.anchor_index(source).is_none();
        if self.cursor.is_none() {
            changed.anchor |= self.set_anchor(None);
        } else if anchor_left {
            changed.anchor |= self.set_anchor(self.cursor);
        }
        self.finish(changed)
    }

    fn cursor_left<S: EntrySource + ?Sized>(
        &mut self,
        source: &S,
        rule: CursorGone,
        survivors: Option<usize>,
    ) -> Changed {
        let mut changed = Changed {
            selection: self.set_provisional(false),
            cursor: true,
            ..Changed::NONE
        };
        let n = source.len();
        let neighbour = match rule {
            CursorGone::Clear => None,
            CursorGone::Neighbor | CursorGone::NeighborSelected if n == 0 => None,
            CursorGone::Neighbor | CursorGone::NeighborSelected => {
                Some(survivors.or(self.cursor_hint).unwrap_or(0).min(n - 1))
            }
        };
        match neighbour {
            Some(i) => {
                let id = source.id_at(i);
                self.cursor = Some(id);
                self.cursor_hint = Some(i);
                if rule == CursorGone::NeighborSelected {
                    changed.selection |= if self.gesture == Gesture::None {
                        self.members.insert(id)
                    } else {
                        // An open gesture keeps going over its base, which gains the neighbour.
                        self.members.insert_base(id)
                    };
                }
            }
            None => {
                self.cursor = None;
                self.cursor_hint = None;
            }
        }
        changed
    }

    /// Moves what the selection holds by `from` to `to`: a rename that gave the entry a new id.
    pub fn relocate(&mut self, from: EntryId, to: EntryId) -> Changed {
        if from == to {
            return Changed::NONE;
        }
        let mut changed = Changed {
            selection: self.members.relocate(from, to),
            ..Changed::NONE
        };
        if self.cursor == Some(from) {
            self.cursor = Some(to);
            changed.cursor = true;
        }
        if let Some((id, _)) = &mut self.removal_hint
            && *id == from
        {
            *id = to;
        }
        if self.anchor == Some(from) {
            self.anchor = Some(to);
            changed.anchor = true;
        }
        self.finish(changed)
    }

    /// Records where the cursor's neighbour is, before the host removes entries.
    pub fn before_removal<S: EntrySource + ?Sized>(
        &mut self,
        before: &S,
        gone: impl Fn(EntryId) -> bool,
    ) {
        self.removal_hint = None;
        let (Some(id), Some(i)) = (self.cursor, self.cursor_index(before)) else {
            return;
        };
        if gone(id) {
            let survivors = (0..i).filter(|&j| !gone(before.id_at(j))).count();
            self.removal_hint = Some((id, survivors));
        }
    }

    // Position restore.

    /// The cursor, anchor and selection to remember for this folder.
    pub fn position<S: EntrySource + ?Sized>(&self, source: &S) -> Position {
        Position {
            cursor: self.cursor,
            cursor_index: self.cursor_index(source).or(self.cursor_hint),
            anchor: self.anchor,
            selected: self.selected_ids(source),
            provisional: self.provisional,
            scroll: None,
        }
    }

    /// Takes back a remembered position.
    pub fn restore(&mut self, position: &Position) -> Changed {
        self.gesture = Gesture::None;
        let mut ids = IdSet::with_capacity_and_hasher(position.selected.len(), Default::default());
        ids.extend(position.selected.iter().copied());
        let same = self.members.equals(&ids);
        // A provisional pick is the cursor's entry alone.
        let provisional = position.provisional
            && ids.len() == 1
            && position.cursor.is_some_and(|c| ids.contains(&c));
        let mut changed = Changed {
            selection: !same | self.set_provisional(provisional),
            ..Changed::NONE
        };
        self.members.set_ids(ids);
        changed.cursor = self.cursor != position.cursor;
        self.cursor = position.cursor;
        self.cursor_hint = position.cursor.and(position.cursor_index);
        self.removal_hint = None;
        // The remembered anchor, which a range's is not its cursor; none without a cursor.
        let anchor = position.anchor.filter(|_| position.cursor.is_some());
        changed.anchor = self.set_anchor(anchor);
        self.finish(changed)
    }

    // Helpers.

    fn place_cursor(&mut self, id: EntryId, index: usize) -> Changed {
        self.cursor_hint = Some(index);
        self.removal_hint = None;
        let moved = self.cursor != Some(id);
        self.cursor = Some(id);
        Changed {
            cursor: moved,
            ..Changed::NONE
        }
    }

    fn set_anchor(&mut self, anchor: Option<EntryId>) -> bool {
        let moved = self.anchor != anchor;
        self.anchor = anchor;
        moved
    }

    fn set_provisional(&mut self, provisional: bool) -> bool {
        let flipped = self.provisional != provisional;
        self.provisional = provisional;
        flipped
    }

    fn finish(&mut self, changed: Changed) -> Changed {
        if changed.any() {
            self.revision = next_revision();
        }
        changed
    }
}

/// The id at `index`, if `source` lists that many entries.
fn listed<S: EntrySource + ?Sized>(source: &S, index: usize) -> Option<EntryId> {
    (index < source.len()).then(|| source.id_at(index))
}

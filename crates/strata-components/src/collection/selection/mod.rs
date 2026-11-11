//! Selection, anchor, cursor and restore memory, by entry id (C1a).
//!
//! [`Selection`] is the selection state of one collection (a list, a grid, one Miller
//! column): which entries are selected, the keyboard cursor, and the range anchor that Shift
//! extends from. It is plain data with no Rune elements, keyed by [`EntryId`], so it survives
//! re-sorts, live updates and refreshes. The pointer, keyboard and marquee controllers (C1b,
//! C1c, C1d) call its operations, and the collection element (C1e) draws from it.
//!
//! # Operations take indices
//!
//! Every operation names entries by their index in the [`EntrySource`] it is given, because
//! that is what hit-testing, keyboard moves and the marquee produce. The selection turns them
//! into ids at once. An index at or past the source's length does nothing.
//!
//! # Keeping it in step with the source
//!
//! After **every** change to the source (a re-sort, a live update, a reload batch, a filter),
//! the host calls [`Selection::sync`] before anything else reads or changes the selection
//! with that source. A range is held by index until then, so it can cover 100,000 entries in
//! well under a millisecond; `sync` turns it into ids. Debug builds panic on a missing `sync`
//! that changed the source's length. Release builds don't panic, and the ids verbs act on
//! ([`Selection::selected_ids`], [`Selection::targets`], [`Selection::position`]) fall back to
//! the range's copied ids when a cheap check (the length, the first and the last id) shows
//! the indices went stale; drawn rows may still be wrong until the `sync`.
//!
//! `sync` drops ids the source no longer lists (unless the listing is still loading) and moves
//! a cursor whose entry went to a neighbour, by the rule the caller picks ([`CursorGone`]).
//! For a delete or a move out of the folder, the host calls [`Selection::before_removal`] with
//! the listing as it was, in the same update and right before it changes the source, so the
//! neighbour is the next surviving entry, as in Strata; without it, the neighbour is the entry
//! at the cursor's old index, Strata's rule after a reload.
//! A rename that gives the entry a new id is [`Selection::relocate`], called before the `sync`
//! for that change.
//!
//! An open gesture (an extend run or a marquee band) survives a sync and a rename: the band
//! still restores the selection at press when moved back, and an extend run keeps the entries
//! [`Selection::toggle_in_run`] flipped.
//!
//! # The cursor, the anchor and the fill
//!
//! The cursor is kept apart from the selection: it may sit on an unselected entry. The
//! anchor is where Shift ranges start. Pointer and keyboard share both, as in Strata.
//!
//! After an operation, or a sync of a complete listing, the cursor and anchor each name a
//! listed entry or nothing, and there is an anchor only if there is a cursor.
//!
//! A load's automatic pick ([`Selection::auto_select`]) is *provisional*: it shows as
//! selected, but it isn't a fill. In 10xer mode, verbs act on the fill if there is one, else on
//! the cursor item ([`Selection::targets`]). The default map's verbs act on the selection only
//! ([`Selection::selected_ids`]).
//!
//! # Change notifications
//!
//! Each operation returns [`Changed`]: whether the selection, the cursor or the anchor really
//! changed. An operation that changes nothing returns [`Changed::NONE`], so the element
//! announces only real changes to assistive technology.
//!
//! [`Selection::revision`] goes up exactly when an operation reports a change, including a
//! cursor-only move, so it over-approximates a change of [`Selection::targets`]: a host that
//! must know whether the targets changed (10xer `; 1` rechecks them) compares the targets.
//! Revisions come from one process-wide counter, so a revision always names one state: a new
//! or replaced `Selection` never repeats a revision an observer already saw. Still, hosts keep one `Selection` per
//! collection and reset it with [`Selection::clear`] or [`Selection::restore`] rather than
//! replace it, so the element's change tracking follows one value.
//!
//! # Decisions
//!
//! - [`Selection::select_all`] keeps the cursor and the anchor where they are, as GTK's
//!   visible cursor does in Strata's Icons and List views; Strata's model moves them to the
//!   last entry. With no cursor yet, the cursor (and the anchor) go to the first entry.
//! - [`Selection::clear`] (Escape, or a plain press on empty space) moves the anchor to the
//!   cursor, for pointer and keyboard alike, so the next Shift range starts at the cursor.
//! - Ctrl+Shift+click: both [`Extend::Replace`] (what Strata's pointer code does) and
//!   [`Extend::Add`] exist. The pointer controller (C1b) picks.
//! - Filtering: `sync` drops the ids a filter hides. The filter bar (C18) saves
//!   [`Selection::position`] before it filters and [`Selection::restore`]s it when Escape
//!   clears the filter.
//! - A restored cursor whose entry was deleted since goes to the neighbour at its remembered
//!   index, as a reload's does. This is an assumption to check against Strata, whose restore
//!   path we haven't traced that far.
//! - Whether a marquee moves the cursor is the marquee controller's call (C1d): the band
//!   operations here leave the cursor and the anchor alone.
//! - [`RestoreMemory`] is bounded both by folders and by the ids it holds in total.
//!
//! Work package: C1a, `docs/work/phase-1-leaves/C1a-selection-model.md`.

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
///
/// Fields may be added: compare with [`Changed::NONE`] or use [`Changed::any`].
///
/// Used by: C1b, C1c, C1d, C1e (a11y announcements), C2, C18.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Changed {
    /// Which entries are selected, or whether they count as a fill (a provisional pick
    /// becoming a real one), changed.
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

/// How [`Selection::extend_to`] combines the range from the anchor with the selection.
///
/// Used by: C1b, C1c.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Extend {
    /// The range replaces the selection: Shift+click, Shift+arrow.
    Replace,
    /// The range adds to the selection as it was when this run of extends began:
    /// Ctrl+Shift+click, 10xer `v`.
    Add,
    /// The range removes from the selection as it was when this run of extends began:
    /// 10xer `V`.
    Remove,
}

/// Whether [`Selection::move_cursor`] moves the anchor with the cursor.
///
/// Used by: C1b, C1c.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnchorRule {
    /// The anchor stays.
    Keep,
    /// The anchor moves to the new cursor.
    Move,
}

/// How a marquee band combines with the selection at press.
///
/// Used by: C1d.
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
///
/// Used by: C1e, C2, and hosts that stream listings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Listing {
    /// Every entry is listed: ids the source lacks are gone.
    #[default]
    Complete,
    /// A (re)load is still delivering entries: ids the source lacks may still arrive, so the
    /// selection and the cursor keep them.
    Loading,
}

/// Where the cursor goes when its entry leaves a complete listing.
///
/// The neighbour is the entry at the cursor's index hint in the new listing, or the last entry
/// when the listing got shorter than that. The hint is:
///
/// - after [`Selection::before_removal`] for a removal that takes the cursor's entry (a
///   delete, a move out), the number of entries before the cursor that survive, so the
///   neighbour is the next surviving entry, else the previous one (Strata's removal rule);
/// - otherwise the cursor's index when last seen, so the neighbour is the first entry at or
///   after its old index, else the previous one (Strata's rule after a reload). A restored
///   position's hint is its remembered index.
///
/// How Strata's removals map onto these rules:
///
/// - A delete, a trash or a 10xer `M` (Strata's `transfer_replacing_cursor`) replaces the
///   cursor: [`Neighbor`](Self::Neighbor) in 10xer mode and
///   [`NeighborSelected`](Self::NeighborSelected) in the default map, as Strata's
///   `preserve_fill_on_removal` (on in 10xer mode) chooses.
/// - A plain move out, such as a drag-move to another folder, is one of Strata's
///   selectionless removals: [`Clear`](Self::Clear).
/// - A reload that dropped the cursor's entry: [`Neighbor`](Self::Neighbor), without
///   `before_removal`.
///
/// Used by: C1e, C2, and hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorGone {
    /// The neighbour takes the cursor, unselected: a reload, a 10xer `d` or `M`.
    #[default]
    Neighbor,
    /// The neighbour takes the cursor and joins the selection: a delete in the default map.
    ///
    /// During an open gesture it joins the gesture's base, so the gesture's range or band
    /// still applies over it. During an [`Extend::Replace`] run, whose base each step
    /// replaces, the next step drops it unless its range covers it.
    NeighborSelected,
    /// No entry takes the cursor: a Columns column whose child column is open, or a plain
    /// move out of the folder (a drag-move).
    Clear,
}

/// How [`Selection::sync`] treats the source it is given.
///
/// Used by: C1e, C2, and hosts.
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
///
/// Used by: C18, and the host's verbs.
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
///
/// See the [module docs](self) for the rules every operation follows.
///
/// Used by: C1b, C1c, C1d, C1e, C2 (one per column), C3, C4, C18.
#[derive(Clone, Debug)]
pub struct Selection {
    members: Members,
    gesture: Gesture,
    cursor: Option<EntryId>,
    /// The cursor's index when last seen, for the reload rule once its entry is gone.
    cursor_hint: Option<usize>,
    /// From [`Selection::before_removal`]: the cursor's id and how many entries before it
    /// survive, for the removal rule. Good only while the cursor is that id, and only until
    /// the next sync of a complete listing.
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

    /// How many entries are selected. While a listing loads, this counts selected ids that
    /// haven't arrived yet.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the selection is a load's automatic pick ([`Selection::auto_select`]), which
    /// shows as selected but isn't a fill.
    pub fn is_provisional(&self) -> bool {
        self.provisional
    }

    /// A value that goes up whenever an operation reports a change, unique across the
    /// process. See the [module docs](self#change-notifications).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The selected entries, in display order: what the default map's verbs and a drag act
    /// on. A provisional pick counts; an unselected cursor never does.
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

    /// Selects only the entry at `index`, and moves the cursor and the anchor to it: a plain
    /// click, or a plain move that selects.
    pub fn select_only<S: EntrySource + ?Sized>(&mut self, source: &S, index: usize) -> Changed {
        self.pick(source, index, false)
    }

    /// Selects the entry at `index` as a load's automatic pick (provisional), with the
    /// cursor and the anchor on it: the first entry of a new listing, or the folder you came
    /// from. Any explicit operation makes it a real selection or replaces it.
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

    /// Flips the entry at `index` in or out of the selection, and moves the cursor and the
    /// anchor to it: Ctrl+click, Ctrl+Space. A provisional pick counts as selected here.
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

    /// Flips the entry at `index` inside an [`Extend::Add`] or [`Extend::Remove`] run: 10xer
    /// Space inside a `v` or `V` range. The flip applies over what the range makes of the
    /// entry, and holds as later steps of the run grow or shrink the range. The run goes on,
    /// and the cursor, the anchor and the range stay where they are.
    ///
    /// Strata flips the cursor's entry, so callers pass the cursor's index. Outside such a
    /// run (none open, or an [`Extend::Replace`] run), it is [`Selection::toggle`].
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

    /// Selects the range from the anchor to the entry at `index` by `mode`, and moves the
    /// cursor there. The anchor stays; with no anchor, it starts at `index`.
    ///
    /// Consecutive calls in the same mode are one run: each replaces the last one's range, so
    /// moving back towards the anchor shrinks it. [`Extend::Add`] and [`Extend::Remove`] apply
    /// the range to the selection as it was when the run began, under the entries
    /// [`Selection::toggle_in_run`] flipped during the run.
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

    /// Moves the cursor to the entry at `index` and keeps the selection: Ctrl+arrow, 10xer
    /// `j`/`k`, Home, End and paging after a fill. If the cursor moves off a provisional
    /// pick, the pick is dropped.
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

    /// Selects every entry `source` lists now; entries listed later stay unselected. The
    /// cursor and the anchor stay; with no cursor, both go to the first entry.
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
        self.provisional = false;
        let changed = Changed {
            selection: self.members.invert(source),
            ..Changed::NONE
        };
        self.finish(changed)
    }

    /// Selects nothing, keeps the cursor, and moves the anchor to the cursor: Escape, or a
    /// plain press on empty space. Reports no change when nothing was selected.
    pub fn clear(&mut self) -> Changed {
        self.gesture = Gesture::None;
        self.provisional = false;
        let mut changed = Changed {
            selection: self.members.clear(),
            ..Changed::NONE
        };
        changed.anchor = self.set_anchor(self.cursor);
        self.finish(changed)
    }

    /// Drops a provisional pick, so the next toggle adds the cursor's entry: 10xer Space
    /// ("the first Space adds that item").
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

    /// Starts a marquee band. [`BandMode::Replace`] clears the selection now, as a plain press
    /// on empty space does; the other modes keep it as the selection at press.
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

    /// Whether a marquee band is open: between [`Selection::begin_band`] and
    /// [`Selection::end_band`] or [`Selection::cancel_band`]. Any other selecting operation
    /// also ends it.
    pub fn is_banding(&self) -> bool {
        matches!(self.gesture, Gesture::Band(_))
    }

    /// Applies the entries the band crosses now (indices into `source`) to the selection at
    /// press. Moving the band back off an entry restores that entry. With no band open
    /// ([`Selection::is_banding`]), it does nothing and reports no change.
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

    /// Ends a band and goes back to the selection at press (Escape during a marquee). For
    /// [`BandMode::Replace`] that is the cleared selection.
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

    /// Brings the selection in step with `source` after it changed. See the
    /// [module docs](self#keeping-it-in-step-with-the-source).
    ///
    /// For a complete listing: ids it lacks leave the selection; a cursor whose entry left
    /// moves by `rule.cursor_gone`, and a provisional pick ends with it; an anchor whose entry
    /// left moves to the cursor. While loading, absent ids and an absent cursor are kept.
    /// Entries new to the source are never selected by a sync (except the neighbour of
    /// [`CursorGone::NeighborSelected`]).
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
        // The first sync of a complete listing uses up the removal hint, whether or not the
        // cursor's entry went.
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
        if self.cursor.is_none() {
            changed.anchor |= self.set_anchor(None);
        } else if self.anchor_index(source).is_none() {
            changed.anchor |= self.set_anchor(self.cursor);
        }
        self.finish(changed)
    }

    /// Moves the cursor off its entry, which `source` no longer lists: to the entry at
    /// `survivors` (the removal rule), else at its last index (the reload rule).
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

    /// Moves what the selection holds by `from` to `to`: a rename that gave the entry a new
    /// id. Call it before the [`Selection::sync`] for that change, or the sync drops `from` as
    /// gone. An open gesture goes on over the renamed id.
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

    /// Records where the cursor's neighbour is, before the host removes entries: `before` is
    /// the listing as it is now, still in step with the selection, and `gone` says which of
    /// its entries are going (a delete, a move out of the folder). If the cursor's entry is
    /// among them, the next [`Selection::sync`] of a complete listing moves the cursor to the
    /// next surviving entry, else the previous one, as Strata does on a removal.
    ///
    /// Call it in the same update that changes the source, right before the change. The
    /// record lasts through syncs of a loading listing, and ends at the first sync of a
    /// complete one, at any cursor move, or at a restore. When the cursor isn't going it
    /// records nothing, so a later reload that drops the cursor's entry uses the reload rule.
    /// A host that abandons a removal after calling this syncs before anything else, or a
    /// reload that starts first would still apply the record.
    ///
    /// It changes nothing visible, and costs one `gone` call per entry up to the cursor.
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

    /// The cursor, anchor and selection to remember for this folder. The scroll position is
    /// the element's to fill in ([`Position::scroll`]).
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

    /// Takes back a remembered position. Then [`Selection::sync`] it with the folder's
    /// listing: ids gone since leave, and a cursor whose entry went moves to the neighbour at
    /// its remembered index.
    ///
    /// It reports exactly what changed, comparing by id.
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
        changed.anchor = self.set_anchor(position.anchor.and(position.cursor));
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

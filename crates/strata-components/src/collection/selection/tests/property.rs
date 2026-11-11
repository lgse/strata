//! Random operations, re-sorts, deletes, inserts, renames, streamed reloads and restores,
//! checked step by step against a plain model of the same rules: membership at every index,
//! the count, the cursor, the anchor, the fill and every reported change. After every step
//! over a complete listing, the cursor and the anchor name listed entries or nothing.
//!
//! The model is written the slow and obvious way, from the rules rather than the code: sets
//! of ids, and the two neighbour rules as walks over the listings (the next survivor in the
//! old listing for a delete, the entry at the old index for a reload).
//!
//! Deterministic: each seed replays the same run. A failure names its seed and the steps that
//! led to it. `STRATA_PROPTEST_SEEDS` overrides the seeds: a count (`5000`) or a range
//! (`1000..2000`).

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;

use super::*;

const SEEDS: Range<u64> = 0..256;
const STEPS: usize = 200;
const MAX_LEN: usize = 48;

/// The seeds to run: [`SEEDS`], or what `STRATA_PROPTEST_SEEDS` says.
fn seeds() -> Range<u64> {
    let Ok(text) = std::env::var("STRATA_PROPTEST_SEEDS") else {
        return SEEDS;
    };
    let parse = |t: &str| {
        t.trim()
            .parse::<u64>()
            .unwrap_or_else(|_| panic!("STRATA_PROPTEST_SEEDS: {text:?} is not N or A..B"))
    };
    match text.split_once("..") {
        Some((a, b)) => parse(a)..parse(b),
        None => 0..parse(&text),
    }
}

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n`, which is at least 1.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelGesture {
    None,
    Extend(Extend),
    Band(BandMode),
}

/// What a delete that takes the cursor's entry told the model before it happened.
#[derive(Clone, Debug)]
struct Removal {
    cursor: EntryId,
    before: Vec<EntryId>,
    gone: BTreeSet<EntryId>,
}

/// The rules, as sets of ids.
#[derive(Clone, Debug)]
struct Model {
    sel: BTreeSet<EntryId>,
    cursor: Option<EntryId>,
    /// The cursor's index when last seen.
    seen_at: Option<usize>,
    anchor: Option<EntryId>,
    provisional: bool,
    gesture: ModelGesture,
    /// The selection an open gesture started from.
    base: BTreeSet<EntryId>,
    /// The entries an open gesture's range or band covers.
    covered: BTreeSet<EntryId>,
    /// The entries Space flipped inside an extend run, over what its range makes of them.
    flipped: BTreeSet<EntryId>,
    removal: Option<Removal>,
}

impl Model {
    fn new() -> Self {
        Self {
            sel: BTreeSet::new(),
            cursor: None,
            seen_at: None,
            anchor: None,
            provisional: false,
            gesture: ModelGesture::None,
            base: BTreeSet::new(),
            covered: BTreeSet::new(),
            flipped: BTreeSet::new(),
            removal: None,
        }
    }

    /// The selection an open gesture makes of its base and what it covers, with the flipped
    /// entries flipped.
    fn apply_gesture(&mut self) {
        let (base, covered) = (&self.base, &self.covered);
        let ranged: BTreeSet<EntryId> = match self.gesture {
            ModelGesture::None => return,
            ModelGesture::Extend(Extend::Replace | Extend::Add)
            | ModelGesture::Band(BandMode::Replace | BandMode::Add) => {
                base.union(covered).copied().collect()
            }
            ModelGesture::Extend(Extend::Remove) => base.difference(covered).copied().collect(),
            ModelGesture::Band(BandMode::Toggle) => {
                base.symmetric_difference(covered).copied().collect()
            }
        };
        self.sel = ranged
            .symmetric_difference(&self.flipped)
            .copied()
            .collect();
    }

    fn end_gesture(&mut self) {
        self.gesture = ModelGesture::None;
    }

    fn place(&mut self, s: &ListSource, i: usize) {
        self.cursor = Some(s.id_at(i));
        self.seen_at = Some(i);
        // A cursor that moved has no removal pending.
        self.removal = None;
    }

    fn pick(&mut self, s: &ListSource, i: usize, provisional: bool) {
        let id = s.id_at(i);
        self.end_gesture();
        self.sel = BTreeSet::from([id]);
        self.provisional = provisional;
        self.place(s, i);
        self.anchor = Some(id);
    }

    fn toggle(&mut self, s: &ListSource, i: usize) {
        let id = s.id_at(i);
        self.end_gesture();
        self.provisional = false;
        if !self.sel.remove(&id) {
            self.sel.insert(id);
        }
        self.place(s, i);
        self.anchor = Some(id);
    }

    fn extend(&mut self, s: &ListSource, i: usize, mode: Extend) {
        let id = s.id_at(i);
        let a = match self.anchor.and_then(|a| s.index_of(a)) {
            Some(a) => a,
            None => {
                self.anchor = Some(id);
                i
            }
        };
        if mode == Extend::Replace {
            self.base.clear();
            self.flipped.clear();
        } else if self.gesture != ModelGesture::Extend(mode) {
            self.base = self.sel.clone();
            self.flipped.clear();
        }
        self.gesture = ModelGesture::Extend(mode);
        self.covered = (a.min(i)..=a.max(i)).map(|j| s.id_at(j)).collect();
        self.apply_gesture();
        self.provisional = false;
        self.place(s, i);
    }

    /// Space inside a `v`/`V` run flips the entry over the range; elsewhere it is a toggle.
    fn toggle_in_run(&mut self, s: &ListSource, i: usize) {
        if !matches!(
            self.gesture,
            ModelGesture::Extend(Extend::Add | Extend::Remove)
        ) {
            return self.toggle(s, i);
        }
        let id = s.id_at(i);
        if !self.flipped.remove(&id) {
            self.flipped.insert(id);
        }
        self.apply_gesture();
    }

    fn move_cursor(&mut self, s: &ListSource, i: usize, rule: AnchorRule) {
        let id = s.id_at(i);
        self.end_gesture();
        if self.provisional && self.cursor != Some(id) {
            self.provisional = false;
            self.sel.clear();
        }
        self.place(s, i);
        if rule == AnchorRule::Move {
            self.anchor = Some(id);
        }
    }

    fn select_all(&mut self, s: &ListSource) {
        if s.is_empty() {
            return;
        }
        self.end_gesture();
        self.sel = listed(s).collect();
        self.provisional = false;
        if self.cursor.is_none() {
            self.place(s, 0);
        }
        if self.anchor.is_none() {
            self.anchor = self.cursor;
        }
    }

    fn invert(&mut self, s: &ListSource) {
        self.end_gesture();
        self.provisional = false;
        self.sel = listed(s).filter(|id| !self.sel.contains(id)).collect();
    }

    fn clear(&mut self) {
        self.end_gesture();
        self.provisional = false;
        self.sel.clear();
        self.anchor = self.cursor;
    }

    fn discard_provisional(&mut self) {
        if self.provisional {
            self.end_gesture();
            self.provisional = false;
            self.sel.clear();
        }
    }

    fn begin_band(&mut self, mode: BandMode) {
        if mode == BandMode::Replace {
            self.sel.clear();
        }
        self.base = self.sel.clone();
        self.covered.clear();
        self.flipped.clear();
        self.gesture = ModelGesture::Band(mode);
        self.provisional = false;
    }

    fn band(&mut self, s: &ListSource, crossed: &[Range<usize>]) {
        if !matches!(self.gesture, ModelGesture::Band(_)) {
            return;
        }
        self.covered = crossed
            .iter()
            .flat_map(Clone::clone)
            .map(|i| s.id_at(i))
            .collect();
        self.apply_gesture();
    }

    fn end_band(&mut self) {
        if matches!(self.gesture, ModelGesture::Band(_)) {
            self.end_gesture();
        }
    }

    fn cancel_band(&mut self) {
        if matches!(self.gesture, ModelGesture::Band(_)) {
            self.end_gesture();
            self.sel = self.base.clone();
        }
    }

    fn before_removal(&mut self, before: &ListSource, gone: BTreeSet<EntryId>) {
        self.removal = self
            .cursor
            .filter(|c| gone.contains(c) && before.index_of(*c).is_some())
            .map(|cursor| Removal {
                cursor,
                before: listed(before).collect(),
                gone,
            });
    }

    /// Where a cursor whose entry left goes, by the rule for how it left.
    fn neighbour(
        &self,
        s: &ListSource,
        cursor: EntryId,
        removal: Option<&Removal>,
    ) -> Option<usize> {
        let n = s.len();
        if let Some(removal) = removal.filter(|r| r.cursor == cursor)
            && let Some(at) = removal.before.iter().position(|&id| id == cursor)
        {
            // A delete: the next entry that survived, else the previous one.
            let survives = |id: &&EntryId| !removal.gone.contains(id);
            let next = removal.before[at + 1..].iter().find(survives);
            let previous = removal.before[..at].iter().rev().find(survives);
            return next.or(previous).and_then(|&id| s.index_of(id));
        }
        // A reload: the first entry at or after the old index, else the previous one.
        let old = self.seen_at.unwrap_or(0);
        (old..n).next().or_else(|| (0..n).next_back())
    }

    fn sync(&mut self, s: &ListSource, rule: SyncRule) {
        if rule.listing == Listing::Loading {
            // A loading listing keeps a pending removal for the complete one.
            if let Some(i) = self.cursor.and_then(|c| s.index_of(c)) {
                self.seen_at = Some(i);
            }
            return;
        }
        let removal = self.removal.take();
        let is_listed = |id: &EntryId| s.index_of(*id).is_some();
        self.sel.retain(is_listed);
        self.base.retain(is_listed);
        self.covered.retain(is_listed);
        self.flipped.retain(is_listed);
        match self.cursor {
            Some(c) => match s.index_of(c) {
                Some(i) => self.seen_at = Some(i),
                None => {
                    self.provisional = false;
                    let to = match rule.cursor_gone {
                        CursorGone::Clear => None,
                        CursorGone::Neighbor | CursorGone::NeighborSelected => {
                            self.neighbour(s, c, removal.as_ref())
                        }
                    };
                    match to {
                        Some(i) => {
                            self.cursor = Some(s.id_at(i));
                            self.seen_at = Some(i);
                            if rule.cursor_gone == CursorGone::NeighborSelected {
                                let id = s.id_at(i);
                                if self.gesture == ModelGesture::None {
                                    self.sel.insert(id);
                                } else {
                                    self.base.insert(id);
                                    self.apply_gesture();
                                }
                            }
                        }
                        None => (self.cursor, self.seen_at) = (None, None),
                    }
                }
            },
            None => self.seen_at = None,
        }
        if self.cursor.is_none() {
            self.anchor = None;
        } else if self.anchor.and_then(|a| s.index_of(a)).is_none() {
            self.anchor = self.cursor;
        }
    }

    fn relocate(&mut self, from: EntryId, to: EntryId) {
        for set in [
            &mut self.sel,
            &mut self.base,
            &mut self.covered,
            &mut self.flipped,
        ] {
            if set.remove(&from) {
                set.insert(to);
            }
        }
        for held in [&mut self.cursor, &mut self.anchor] {
            if *held == Some(from) {
                *held = Some(to);
            }
        }
        if let Some(r) = &mut self.removal {
            for id in r.before.iter_mut().chain([&mut r.cursor]) {
                if *id == from {
                    *id = to;
                }
            }
            if r.gone.remove(&from) {
                r.gone.insert(to);
            }
        }
    }

    fn restore(&mut self, p: &Position) {
        self.end_gesture();
        self.removal = None;
        self.sel = p.selected.iter().copied().collect();
        self.provisional =
            p.provisional && self.sel.len() == 1 && p.cursor.is_some_and(|c| self.sel.contains(&c));
        self.cursor = p.cursor;
        self.seen_at = p.cursor.and(p.cursor_index);
        self.anchor = p.anchor.and(p.cursor);
    }

    fn selected_in_order(&self, s: &ListSource) -> Vec<EntryId> {
        listed(s).filter(|id| self.sel.contains(id)).collect()
    }

    fn targets(&self, s: &ListSource) -> Targets {
        let filled = self.selected_in_order(s);
        if !self.provisional && !filled.is_empty() {
            return Targets::Filled(filled);
        }
        match self.cursor {
            Some(c) if s.index_of(c).is_some() => Targets::Cursor(c),
            _ => Targets::Nothing,
        }
    }

    fn position(&self, s: &ListSource) -> Position {
        Position {
            cursor: self.cursor,
            cursor_index: self.cursor.and_then(|c| s.index_of(c)).or(self.seen_at),
            anchor: self.anchor,
            selected: self.selected_in_order(s),
            provisional: self.provisional,
            scroll: None,
        }
    }
}

fn listed(s: &ListSource) -> impl Iterator<Item = EntryId> + '_ {
    (0..s.len()).map(|i| s.id_at(i))
}

/// The model's change between two states, as [`Changed`] reports it.
fn diff(before: &Model, after: &Model) -> Changed {
    Changed {
        selection: before.sel != after.sel || before.provisional != after.provisional,
        cursor: before.cursor != after.cursor,
        anchor: before.anchor != after.anchor,
    }
}

/// How many times the run reached the paths that are easy to miss.
#[derive(Debug, Default)]
struct Coverage {
    /// An extend run going on over a non-empty base (Add or Remove).
    extend_runs_over_a_base: usize,
    /// Space flipping an entry inside an Add or Remove run.
    flips_inside_a_run: usize,
    /// An extend step of a run that holds a flip.
    extend_steps_over_a_flip: usize,
    /// A band move over a non-empty selection at press.
    band_moves_over_a_base: usize,
    /// A sync or a rename while a gesture was open.
    changes_mid_gesture: usize,
    /// A delete's neighbour joining an open gesture's base.
    neighbours_into_a_gesture: usize,
    /// A delete where the next survivor isn't at the cursor's old index.
    deletes_past_the_old_index: usize,
    /// A delete of the cursor's entry delivered through a loading sync first.
    deletes_through_loading: usize,
    /// A prepared delete that never landed, followed by a reload.
    unfinished_deletes_then_reload: usize,
}

struct Run {
    rng: Rng,
    s: ListSource,
    sel: Selection,
    model: Model,
    next_id: u64,
    saved: Vec<Position>,
    log: Rc<RefCell<Vec<String>>>,
    /// Whether the source holds its whole listing (not mid-reload).
    complete: bool,
    coverage: Rc<RefCell<Coverage>>,
}

impl Run {
    fn new(seed: u64, log: Rc<RefCell<Vec<String>>>, coverage: Rc<RefCell<Coverage>>) -> Self {
        let mut rng = Rng(seed);
        let n = rng.below(25) as u64;
        Self {
            rng,
            s: ListSource::new((0..n).map(entry).collect()),
            sel: Selection::new(),
            model: Model::new(),
            next_id: n,
            saved: Vec::new(),
            log,
            complete: true,
            coverage,
        }
    }

    fn fresh_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id + 1_000
    }

    fn note(&self, what: String) {
        self.log.borrow_mut().push(what);
    }

    /// Applies `op` to both, then checks they agree and that the reported change is the
    /// model's.
    fn op(
        &mut self,
        what: String,
        f: impl FnOnce(&mut Selection, &mut Model, &ListSource) -> Changed,
    ) {
        self.note(what);
        let before = self.model.clone();
        let revision = self.sel.revision();
        let changed = f(&mut self.sel, &mut self.model, &self.s);
        assert_eq!(changed, diff(&before, &self.model), "reported change");
        let bumped = self.sel.revision() != revision;
        assert_eq!(
            bumped,
            changed.any(),
            "revision follows the reported change"
        );
        self.check();
    }

    fn check(&self) {
        let (s, sel, m) = (&self.s, &self.sel, &self.model);
        for i in 0..s.len() {
            let id = s.id_at(i);
            assert_eq!(
                sel.is_selected_at(s, i),
                m.sel.contains(&id),
                "entry {id:?} at {i}"
            );
        }
        for &id in &m.sel {
            assert!(sel.contains(s, id), "{id:?} selected");
        }
        assert_eq!(sel.len(), m.sel.len(), "count");
        assert_eq!(sel.cursor(), m.cursor, "cursor");
        assert_eq!(sel.anchor(), m.anchor, "anchor");
        assert_eq!(sel.is_provisional(), m.provisional, "provisional");
        assert_eq!(
            sel.is_banding(),
            matches!(m.gesture, ModelGesture::Band(_)),
            "banding"
        );
        assert_eq!(sel.selected_ids(s), m.selected_in_order(s), "selected ids");
        assert_eq!(sel.targets(s), m.targets(s), "targets");
        if !self.complete {
            return;
        }
        let is_listed = |id: EntryId| s.index_of(id).is_some();
        assert!(sel.cursor().is_none_or(is_listed), "cursor on a missing id");
        assert!(sel.anchor().is_none_or(is_listed), "anchor on a missing id");
        assert!(
            sel.anchor().is_none() || sel.cursor().is_some(),
            "anchor without cursor"
        );
        assert!(
            m.sel.iter().all(|&id| is_listed(id)),
            "selected id not listed"
        );
        if sel.is_provisional() {
            assert_eq!(
                m.sel.iter().copied().collect::<Vec<_>>(),
                Vec::from_iter(m.cursor)
            );
        }
    }

    fn index(&mut self) -> Option<usize> {
        (!self.s.is_empty()).then(|| self.rng.below(self.s.len()))
    }

    fn extend(&mut self, mode: Extend) {
        let Some(i) = self.index() else { return };
        let m = &self.model;
        if mode != Extend::Replace && m.gesture == ModelGesture::Extend(mode) && !m.base.is_empty()
        {
            self.coverage.borrow_mut().extend_runs_over_a_base += 1;
        }
        if m.gesture == ModelGesture::Extend(mode) && !m.flipped.is_empty() {
            self.coverage.borrow_mut().extend_steps_over_a_flip += 1;
        }
        self.op(format!("extend_to {i} {mode:?}"), |sel, m, s| {
            m.extend(s, i, mode);
            sel.extend_to(s, i, mode)
        });
    }

    /// Space during a run: mostly on the cursor, as Strata does, sometimes elsewhere.
    fn toggle_in_run(&mut self) {
        let here = self.model.cursor.and_then(|c| self.s.index_of(c));
        let Some(i) = here
            .filter(|_| self.rng.chance(70))
            .or_else(|| self.index())
        else {
            return;
        };
        if matches!(
            self.model.gesture,
            ModelGesture::Extend(Extend::Add | Extend::Remove)
        ) {
            self.coverage.borrow_mut().flips_inside_a_run += 1;
        }
        self.op(format!("toggle_in_run {i}"), |sel, m, s| {
            m.toggle_in_run(s, i);
            sel.toggle_in_run(s, i)
        });
    }

    fn band_move(&mut self) {
        if matches!(self.model.gesture, ModelGesture::Band(_)) && !self.model.base.is_empty() {
            self.coverage.borrow_mut().band_moves_over_a_base += 1;
        }
        let crossed = self.random_ranges();
        let mut set = IndexRanges::new();
        for r in &crossed {
            set.push(r.clone());
        }
        self.op(format!("band {crossed:?}"), |sel, m, s| {
            m.band(s, &crossed);
            sel.band(s, &set)
        });
    }

    fn random_op(&mut self) {
        // Gestures go on most of the time, so runs and bands get long enough to matter.
        match self.model.gesture {
            ModelGesture::Band(_) if self.rng.chance(70) => return self.band_move(),
            ModelGesture::Extend(_) if self.rng.chance(10) => return self.toggle_in_run(),
            ModelGesture::Extend(mode) if self.rng.chance(60) => return self.extend(mode),
            _ => {}
        }
        match self.rng.below(16) {
            0 | 1 => {
                if let Some(i) = self.index() {
                    self.op(format!("select_only {i}"), |sel, m, s| {
                        m.pick(s, i, false);
                        sel.select_only(s, i)
                    });
                }
            }
            2 => {
                if let Some(i) = self.index() {
                    self.op(format!("auto_select {i}"), |sel, m, s| {
                        m.pick(s, i, true);
                        sel.auto_select(s, i)
                    });
                }
            }
            3 | 4 => {
                if let Some(i) = self.index() {
                    self.op(format!("toggle {i}"), |sel, m, s| {
                        m.toggle(s, i);
                        sel.toggle(s, i)
                    });
                }
            }
            5..=7 => {
                let mode = self
                    .rng
                    .pick(&[Extend::Replace, Extend::Add, Extend::Remove]);
                self.extend(mode);
            }
            8 => {
                if let Some(i) = self.index() {
                    let rule = self.rng.pick(&[AnchorRule::Keep, AnchorRule::Move]);
                    self.op(format!("move_cursor {i} {rule:?}"), |sel, m, s| {
                        m.move_cursor(s, i, rule);
                        sel.move_cursor(s, i, rule)
                    });
                }
            }
            9 => self.op("select_all".into(), |sel, m, s| {
                m.select_all(s);
                sel.select_all(s)
            }),
            10 => self.op("invert".into(), |sel, m, s| {
                m.invert(s);
                sel.invert(s)
            }),
            11 => {
                if self.rng.chance(50) {
                    self.op("clear".into(), |sel, m, _| {
                        m.clear();
                        sel.clear()
                    });
                } else {
                    self.op("discard_provisional".into(), |sel, m, _| {
                        m.discard_provisional();
                        sel.discard_provisional()
                    });
                }
            }
            12 | 13 => match self.rng.below(4) {
                0 => self.op("end_band".into(), |sel, m, _| {
                    m.end_band();
                    sel.end_band();
                    Changed::NONE
                }),
                1 => self.op("cancel_band".into(), |sel, m, _| {
                    m.cancel_band();
                    sel.cancel_band()
                }),
                2 => self.band_move(),
                _ => {
                    let mode = self
                        .rng
                        .pick(&[BandMode::Replace, BandMode::Toggle, BandMode::Add]);
                    self.op(format!("begin_band {mode:?}"), |sel, m, _| {
                        m.begin_band(mode);
                        sel.begin_band(mode)
                    });
                }
            },
            14 => {
                let p = self.sel.position(&self.s);
                assert_eq!(p, self.model.position(&self.s), "position");
                self.note("save position".into());
                self.saved.push(p);
            }
            _ => {
                if !self.saved.is_empty() && self.complete {
                    let p = self.saved[self.rng.below(self.saved.len())].clone();
                    // Until the sync below, the restored ids may name entries gone since.
                    self.complete = false;
                    self.op("restore".into(), |sel, m, _| {
                        m.restore(&p);
                        sel.restore(&p)
                    });
                    self.sync(Listing::Complete);
                }
            }
        }
    }

    /// Ascending, disjoint ranges within the source.
    fn random_ranges(&mut self) -> Vec<Range<usize>> {
        let n = self.s.len();
        let mut out = Vec::new();
        let mut at = 0;
        while at < n && out.len() < 3 {
            let start = at + self.rng.below(n - at + 1);
            if start >= n {
                break;
            }
            let end = start + 1 + self.rng.below(n - start);
            out.push(start..end);
            at = end + 1;
        }
        out
    }

    fn mid_gesture(&self) -> bool {
        self.model.gesture != ModelGesture::None
    }

    fn sync(&mut self, listing: Listing) {
        let cursor_gone = self.rng.pick(&[
            CursorGone::Neighbor,
            CursorGone::NeighborSelected,
            CursorGone::Clear,
        ]);
        let rule = SyncRule {
            listing,
            cursor_gone,
        };
        self.complete = listing == Listing::Complete;
        if self.mid_gesture() {
            let mut coverage = self.coverage.borrow_mut();
            coverage.changes_mid_gesture += 1;
            let cursor_goes = self
                .model
                .cursor
                .is_some_and(|c| self.s.index_of(c).is_none());
            if cursor_goes
                && listing == Listing::Complete
                && cursor_gone == CursorGone::NeighborSelected
            {
                coverage.neighbours_into_a_gesture += 1;
            }
        }
        self.op(format!("sync {rule:?}"), |sel, m, s| {
            m.sync(s, rule);
            sel.sync(s, rule)
        });
    }

    /// A delete: the host tells the selection, removes the entries, then syncs, sometimes
    /// through a loading sync first.
    fn delete(&mut self, gone: BTreeSet<EntryId>) {
        let through_loading = self.rng.chance(25);
        self.note(format!(
            "delete {gone:?}, through loading: {through_loading}"
        ));
        if through_loading && self.model.cursor.is_some_and(|c| gone.contains(&c)) {
            self.coverage.borrow_mut().deletes_through_loading += 1;
        }
        if let Some(at) = self.model.cursor.and_then(|c| self.s.index_of(c))
            && gone.contains(&self.s.id_at(at))
        {
            let before = (0..at)
                .filter(|&i| !gone.contains(&self.s.id_at(i)))
                .count();
            let next = (at..self.s.len()).find(|&i| !gone.contains(&self.s.id_at(i)));
            if next.is_some_and(|_| before != at) {
                self.coverage.borrow_mut().deletes_past_the_old_index += 1;
            }
        }
        self.model.before_removal(&self.s, gone.clone());
        self.sel.before_removal(&self.s, |id| gone.contains(&id));
        self.s.retain(|e| !gone.contains(&e.id));
        if through_loading {
            self.sync(Listing::Loading);
        }
        self.sync(Listing::Complete);
    }

    /// A delete the host prepares but that never lands, then a reload that drops entries,
    /// maybe the cursor's: the reload rule applies. A host that abandons a removal of the
    /// cursor's entry syncs before anything else, as `before_removal` asks; one that spares
    /// the cursor records nothing, so it may go straight into the reload.
    fn unfinished_delete(&mut self) {
        let mut gone = self.model.sel.clone();
        let takes_cursor = self.rng.chance(50);
        match self.model.cursor {
            Some(c) if takes_cursor => {
                gone.insert(c);
            }
            Some(c) => {
                gone.remove(&c);
            }
            None => {}
        }
        self.note(format!("prepare delete {gone:?}, which fails"));
        self.model.before_removal(&self.s, gone.clone());
        self.sel.before_removal(&self.s, |id| gone.contains(&id));
        if takes_cursor && self.rng.chance(50) {
            // Abandoned by moving the cursor, often onto its own entry: that ends the record.
            let here = self.model.cursor.and_then(|c| self.s.index_of(c));
            let i = here
                .filter(|_| self.rng.chance(70))
                .or_else(|| self.index());
            if let Some(i) = i {
                self.op(format!("move_cursor {i} Keep"), |sel, m, s| {
                    m.move_cursor(s, i, AnchorRule::Keep);
                    sel.move_cursor(s, i, AnchorRule::Keep)
                });
            }
        } else if takes_cursor || self.rng.chance(50) {
            self.sync(Listing::Complete);
        }
        if self.model.cursor.is_some() {
            self.coverage.borrow_mut().unfinished_deletes_then_reload += 1;
        }
        self.reload();
    }

    fn random_change(&mut self) {
        let mut entries = self.s.entries().to_vec();
        match self.rng.below(7) {
            6 => return self.unfinished_delete(),
            0 => {
                let keys: HashMap<EntryId, u64> =
                    entries.iter().map(|e| (e.id, self.rng.next())).collect();
                self.note("shuffle".into());
                self.s.sort_by(|a, b| keys[&a.id].cmp(&keys[&b.id]));
            }
            1 => {
                // A delete of the selection, the cursor with it, as `d` or Delete does.
                let mut gone = self.model.sel.clone();
                gone.extend(self.model.cursor);
                return self.delete(gone);
            }
            2 => {
                let percent = self.rng.pick(&[10, 30, 100]);
                let gone: BTreeSet<EntryId> = entries
                    .iter()
                    .map(|e| e.id)
                    .filter(|_| self.rng.chance(percent))
                    .collect();
                if self.rng.chance(50) {
                    return self.delete(gone);
                }
                // A reload that comes back without them.
                self.note(format!("drop {gone:?}"));
                self.s.retain(|e| !gone.contains(&e.id));
            }
            3 => {
                let count = 1 + self.rng.below(4);
                for _ in 0..count {
                    if entries.len() < MAX_LEN {
                        let at = self.rng.below(entries.len() + 1);
                        let raw = self.fresh_id();
                        entries.insert(at, entry(raw));
                    }
                }
                self.note(format!("insert to {}", entries.len()));
                self.s.replace(entries);
            }
            4 if !entries.is_empty() => {
                let at = self.rng.below(entries.len());
                let from = entries[at].id;
                let to = EntryId::new(self.fresh_id());
                entries[at] = entry(to.get());
                self.s.replace(entries);
                if self.mid_gesture() {
                    self.coverage.borrow_mut().changes_mid_gesture += 1;
                }
                self.op(format!("relocate {from:?} -> {to:?}"), |sel, m, _| {
                    m.relocate(from, to);
                    sel.relocate(from, to)
                });
            }
            _ => return self.reload(),
        }
        self.sync(Listing::Complete);
    }

    /// A streamed reload: the listing empties, comes back in batches (with operations in
    /// between), and ends without some of its entries.
    fn reload(&mut self) {
        let mut full = self.s.entries().to_vec();
        full.retain(|_| !self.rng.chance(20));
        self.note(format!("reload to {}", full.len()));
        self.s.replace(Vec::new());
        self.sync(Listing::Loading);
        let mut shown = 0;
        while shown < full.len() {
            shown += 1 + self.rng.below(full.len() - shown);
            self.s.replace(full[..shown].to_vec());
            self.sync(Listing::Loading);
            for _ in 0..self.rng.below(3) {
                self.random_op();
            }
        }
        self.s.replace(full);
        self.sync(Listing::Complete);
    }

    fn step(&mut self) {
        if self.rng.chance(25) {
            self.random_change();
        } else {
            self.random_op();
        }
    }
}

#[test]
fn random_operations_keep_cursor_and_anchor_on_listed_entries() {
    let coverage = Rc::new(RefCell::new(Coverage::default()));
    let seeds = seeds();
    for seed in seeds.clone() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut run = Run::new(seed, Rc::clone(&log), Rc::clone(&coverage));
            run.check();
            for _ in 0..STEPS {
                run.step();
            }
        }));
        if let Err(panic) = result {
            let log = log.borrow();
            let mut steps = String::new();
            for line in log.iter().rev().take(30).rev() {
                let _ = writeln!(steps, "  {line}");
            }
            eprintln!("seed {seed} failed; its last steps:\n{steps}");
            resume_unwind(panic);
        }
    }
    let coverage = coverage.borrow();
    eprintln!("selection property test, seeds {seeds:?}: {coverage:#?}");
    // Each path the model exists to check must come up, or the weights have drifted.
    for (what, count) in [
        ("extend runs over a base", coverage.extend_runs_over_a_base),
        ("flips inside a run", coverage.flips_inside_a_run),
        (
            "extend steps over a flip",
            coverage.extend_steps_over_a_flip,
        ),
        ("band moves over a base", coverage.band_moves_over_a_base),
        ("changes mid-gesture", coverage.changes_mid_gesture),
        (
            "neighbours into a gesture",
            coverage.neighbours_into_a_gesture,
        ),
        (
            "deletes past the old index",
            coverage.deletes_past_the_old_index,
        ),
        ("deletes through loading", coverage.deletes_through_loading),
        (
            "unfinished deletes then a reload",
            coverage.unfinished_deletes_then_reload,
        ),
    ] {
        assert!(count > 0, "the property run never reached {what}");
    }
}

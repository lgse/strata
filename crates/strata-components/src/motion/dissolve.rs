//! The permanent-delete dissolve: rows break into fragments that scatter and fade (Strata's
//! `dissolve_delete.rs`, its budget, tiles, curves and seeded motion).
//!
//! Each fragment is a clipped piece of a ghost of its row (the host's look for that entry,
//! [`MotionLayer::ghost`](super::MotionLayer::ghost)), moved by its own `Live` offset and
//! opacity, which [`Effects::tick`](super::Effects::tick) sets each frame: nothing commits
//! while it plays. With no ghost, fragments are flat shards drawn with `canvas`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rune::{Color, El, Live, Shape, canvas, el};
use strata_kit::look::Look;

use super::model::{EffectId, Event};

/// How long a dissolve plays (Strata's `DURATION`).
pub const DURATION: f64 = 0.56;
const MIN_FRAGMENT_BUDGET: usize = 80;
/// The top of the fragment budget (Strata's `MAX_FRAGMENT_BUDGET`): tiles are sized for it,
/// and rounding each row up to whole tiles can add a few more (15 rows make 225).
pub const MAX_FRAGMENTS: usize = 192;
const FRAGMENTS_PER_ROW: usize = 48;
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// A dissolve to start with [`Effects::dissolve`](super::Effects::dissolve): the rows by key
/// (C1e's `row_path`), each with a token the host's ghost builder hears.
#[derive(Clone, Debug, Default)]
pub struct Dissolve {
    pub(super) rows: Vec<(String, u64)>,
    pub(super) within: Option<String>,
}

impl Dissolve {
    /// The rows keyed `rows`, each with the host's token for its ghost.
    pub fn rows(rows: impl IntoIterator<Item = (String, u64)>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
            within: None,
        }
    }

    /// The scroll viewport the rows show in (C1e's `viewport_path`): only rows that show in
    /// it break up, and their fragments are clipped to it.
    pub fn within(mut self, viewport: impl Into<String>) -> Self {
        self.within = Some(viewport.into());
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Phase {
    /// The fragments mount at rest; play starts at the first tick after a view planned them.
    Mount {
        ticks: u8,
    },
    Play {
        t0: f64,
    },
}

pub(super) struct DissolveRun {
    pub id: EffectId,
    pub rows: Vec<(String, u64)>,
    pub within: Option<String>,
    pub phase: Phase,
    /// Planned by the first view that runs (a view reads layout and the tick doesn't); the
    /// tick then moves its fragments.
    pub plan: RefCell<Option<Rc<Plan>>>,
    /// How many plans the views have made: the fragments' memo version.
    pub plans: Cell<u64>,
}

/// The rows that break up and their fragments.
pub(super) struct Plan {
    /// The window rect the fragments are clipped to, if any.
    pub clip: Option<[f32; 4]>,
    /// Each row that breaks up: its index in the run's rows, and its window rect.
    pub rows: Vec<(usize, [f32; 4])>,
    pub pieces: Vec<Piece>,
}

pub(super) struct Piece {
    /// Its row's place in [`Plan::rows`].
    pub row: usize,
    /// The tile it shows, in its row's points.
    pub tile: [f32; 4],
    offset: [f64; 2],
    delay: f64,
    pub off: Live<[f32; 2]>,
    pub op: Live<f32>,
}

impl DissolveRun {
    pub fn new(id: EffectId, d: Dissolve) -> Self {
        Self {
            id,
            rows: d.rows,
            within: d.within,
            phase: Phase::Mount { ticks: 0 },
            plan: RefCell::new(None),
            plans: Cell::new(0),
        }
    }

    /// One tick at `now`: pushes the run's events, lowers `next` to its next deadline, and says
    /// whether it plays on.
    pub fn step(&mut self, now: f64, events: &mut Vec<Event>, next: &mut Option<f64>) -> bool {
        let due = |next: &mut Option<f64>, at: f64| *next = Some(next.map_or(at, |n| n.min(at)));
        match self.phase {
            // Until a view has planned and drawn the fragments, or for 3 ticks with no layer.
            Phase::Mount { ticks } if self.plan.borrow().is_none() && ticks < 3 => {
                self.phase = Phase::Mount { ticks: ticks + 1 };
                due(next, now);
                true
            }
            Phase::Mount { .. } => {
                let empty = self
                    .plan
                    .borrow()
                    .as_ref()
                    .is_none_or(|p| p.pieces.is_empty());
                if empty {
                    events.push(Event::Done(self.id));
                    return false;
                }
                self.phase = Phase::Play { t0: now };
                due(next, now);
                true
            }
            Phase::Play { t0 } => {
                let p = ((now - t0) / DURATION).clamp(0.0, 1.0);
                if let Some(plan) = self.plan.borrow().as_ref() {
                    for piece in &plan.pieces {
                        piece.at(p);
                    }
                }
                if p >= 1.0 {
                    events.push(Event::Done(self.id));
                    return false;
                }
                // Every frame while it plays: one poll a frame, no commit.
                due(next, now);
                true
            }
        }
    }
}

impl Piece {
    /// Where it is at progress `p` of the whole dissolve (Strata's `snapshot_fragment`).
    fn at(&self, p: f64) -> (f32, [f32; 2]) {
        let local = ((p - self.delay) / (1.0 - self.delay)).clamp(0.0, 1.0);
        let movement = 1.0 - (1.0 - local).powi(3);
        let opacity = 1.0 - local.powi(3);
        let lift = self.offset[1] * movement + 14.0 * local * local;
        let off = [(self.offset[0] * movement) as f32, lift as f32];
        self.off.set(off);
        self.op.set(opacity as f32);
        (opacity as f32, off)
    }
}

/// Plans the fragments of the rows at `rects` (each a row index and its window rect), in a
/// clip rect if any: Strata's budget of 48 a row between 80 and 192, square tiles sized to
/// it, and seeded motion that sweeps from the left and rises.
pub(super) fn plan(rects: Vec<(usize, [f32; 4])>, clip: Option<[f32; 4]>) -> Plan {
    let mut pieces = Vec::new();
    if !rects.is_empty() {
        let budget = (rects.len() * FRAGMENTS_PER_ROW).clamp(MIN_FRAGMENT_BUDGET, MAX_FRAGMENTS);
        let area: f64 = rects.iter().map(|(_, r)| f64::from(r[2] * r[3])).sum();
        let tile = (area / budget as f64).sqrt().max(4.0) as f32;
        let mut random = Random(SEED);
        for (place, (_, r)) in rects.iter().enumerate() {
            fragments(place, r[2], r[3], tile, &mut random, &mut pieces);
        }
    }
    Plan {
        clip,
        rows: rects,
        pieces,
    }
}

fn fragments(
    row: usize,
    width: f32,
    height: f32,
    tile: f32,
    random: &mut Random,
    out: &mut Vec<Piece>,
) {
    let columns = (width / tile).ceil().max(1.0) as usize;
    let lines = (height / tile).ceil().max(1.0) as usize;
    let row_delay = (row as f64 * 0.018).min(0.06);
    for line in 0..lines {
        for column in 0..columns {
            let x = column as f32 * tile;
            let y = line as f32 * tile;
            let w = tile.min(width - x);
            let h = tile.min(height - y);
            let horizontal = (f64::from(x) + f64::from(w) / 2.0) / f64::from(width);
            let outward = (horizontal - 0.5) * 32.0;
            let offset_x = outward + random.centered(34.0);
            let offset_y = if random.next() < 0.18 {
                3.0 + random.next() * 13.0
            } else {
                -10.0 - random.next() * 38.0
            };
            out.push(Piece {
                row,
                tile: [x, y, w, h],
                offset: [offset_x, offset_y],
                delay: (row_delay + horizontal * 0.36 + random.next() * 0.13).min(0.55),
                off: Live::new([0.0, 0.0]),
                op: Live::new(1.0),
            });
        }
    }
}

/// Strata's LCG, so fragments scatter the same way every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 32) as f64 / f64::from(u32::MAX)
    }

    fn centered(&mut self, spread: f64) -> f64 {
        (self.next() - 0.5) * spread
    }
}

/// Fragment `i` of `plan`, at `origin` (where the clip's top-left sits in the layer), showing
/// a piece of `ghost`, or a flat shard when there is none.
pub(super) fn piece(
    plan: &Plan,
    i: usize,
    origin: [f32; 2],
    ghost: Option<&El>,
    look: &Look,
) -> El {
    let p = &plan.pieces[i];
    let r = plan.rows[p.row].1;
    let [tx, ty, tw, th] = p.tile;
    let at = el()
        .absolute(r[0] - origin[0] + tx, r[1] - origin[1] + ty)
        .size(tw, th)
        .live_offset(&p.off)
        .live_opacity(&p.op);
    match ghost {
        Some(ghost) => at.child(
            el().w_full()
                .h_full()
                .clip_soft(0.0)
                .child(ghost.clone().absolute(-tx, -ty).size(r[2], r[3])),
        ),
        None => at.child(
            canvas(shard(i, tw, th, shard_color(look, i)))
                .w_full()
                .h_full(),
        ),
    }
}

/// A shard's colour: mostly the selection fill the deleted rows wear, with some in the text's
/// and the icon's colours.
fn shard_color(look: &Look, i: usize) -> Color {
    let c = &look.kit.c;
    match i % 7 {
        2 | 5 => c.text_base.alpha(0.55),
        4 => c.icon_base.alpha(0.7),
        _ => rune_kit::TableStyle::from_kit(&look.kit).selected,
    }
}

/// A four-sided shard filling most of a `w` by `h` tile, its corners pulled in by seeded
/// amounts so no two match.
fn shard(i: usize, w: f32, h: f32, color: Color) -> Shape {
    let mut random = Random(SEED ^ (i as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
    let mut j = || (random.next() as f32) * 0.25 * w.min(h);
    let points = vec![
        [j(), j()],
        [w - j(), j()],
        [w - j(), h - j()],
        [j(), h - j()],
    ];
    Shape::new().fill(points, color)
}

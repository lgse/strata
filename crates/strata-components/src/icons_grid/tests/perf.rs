//! Perf: 100,000 entries in the grid, held to C4's ListTable and C1e's plain list over the
//! same entries, measured side by side in the same run.
//!
//! `cargo test --profile perf -p strata-components --lib icons_grid::tests::perf -- --ignored --nocapture`
//!
//! As C1e's and C4's: a 900 pt window at scale 2, the first frame, then 600 frames at 120 Hz
//! each scrolled 48 pt by the wheel (thumbnails served as they are asked for, as a host's poll
//! hands them over), then 30 clicks on tiles. The grid runs at 32, 64 and 256 pt tiles. C3
//! adds a size step (16 steps up the ladder and 16 back, the cursor where the clicks left it) and a
//! reflow (the width 900 → 780 → 900, ten times). Each figure is the best of three runs.
//!
//! The run also reports C1e's lines packed as the grid packs them with C1e's default cells
//! (an icon and a name), the floor under the grid's own tiles.
//!
//! The bar (owner-agreed, work package C3):
//! - no view run while scrolling, and none while thumbnails land;
//! - scroll frames within 2 × the ListTable's p95 in the same run, and none over 16.7 ms;
//! - a click commits within 4 ms at p95 and rebuilds at most 4 tiles on average;
//! - a size step and a reflow each commit within 16.7 ms at p95.

use std::cell::Cell as StdCell;
use std::rc::Rc;
use std::time::Instant;

use rune::{Align, Cx, El, Env, Runtime, col};
use strata_kit::look::Look;

use crate::collection::element::{Collection, CollectionState};
use crate::contract::{Entry, EntryId, EntryKind, EntrySource, ThumbKey};
use crate::icons_grid::{IconsGrid, IconsGridState, IconsParts, SizeCommand, SizeRequest};
use crate::list_table::{
    Cell, Column, ColumnLayout, ColumnWidths, ListParts, ListTable, ListTableState,
};
use crate::thumbnails::{Outcome, ThumbnailStore, ViewerId};

const N: usize = 100_000;
const SIDE: f32 = 900.0;
const RUNS: usize = 3;
const SCOPE: &str = "files";
const FRAME_BUDGET: f64 = 1000.0 / 60.0;

thread_local! {
    static VIEWS: StdCell<usize> = const { StdCell::new(0) };
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn views() -> usize {
    VIEWS.with(StdCell::get)
}

/// `n` entries with dense ids; every third a thumbnail.
struct Source {
    names: Vec<String>,
}

impl EntrySource for Source {
    fn len(&self) -> usize {
        self.names.len()
    }
    fn id_at(&self, index: usize) -> EntryId {
        EntryId::new(index as u64)
    }
    fn index_of(&self, id: EntryId) -> Option<usize> {
        let i = id.get() as usize;
        (i < self.names.len()).then_some(i)
    }
    fn entry(&self, index: usize) -> Entry<'_> {
        let id = EntryId::new(index as u64);
        let kind = if index.is_multiple_of(25) {
            EntryKind::Folder
        } else {
            EntryKind::File
        };
        let e = Entry::new(id, &self.names[index], kind, "");
        if index % 3 == 1 {
            e.thumbnail(ThumbKey::new(index as u64))
        } else {
            e
        }
    }
}

struct Host {
    src: Source,
    coll: CollectionState,
    grid: IconsGridState,
    table: ListTableState,
    layout: ColumnLayout,
    widths: ColumnWidths,
    look: Look,
    store: ThumbnailStore,
    viewer: ViewerId,
    size: f32,
    width: f32,
    /// Tiles built: the description's reads.
    built: Rc<StdCell<usize>>,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Grid,
    Table,
    List,
    /// C1e's own lines packed as the grid packs them, with its default cells: what the grid
    /// costs before its tiles.
    Packed,
}

impl Host {
    fn new(kind: Kind, size: f32) -> Self {
        let mut coll = CollectionState::new();
        coll.controllers.pointer = Some(Box::new(crate::collection::pointer::PointerIntent::new()));
        let mut store = ThumbnailStore::default();
        let viewer = store.viewer();
        Self {
            src: Source {
                names: (0..N).map(|i| format!("entry {i:06}.txt")).collect(),
            },
            coll,
            grid: IconsGridState::new(),
            table: ListTableState::new(),
            layout: ColumnLayout::new(columns().iter().map(|c| c.id().clone())),
            widths: ColumnWidths::new(),
            look: Look::fallback(true),
            store,
            viewer,
            size,
            width: SIDE,
            built: Rc::default(),
            kind,
        }
    }

    /// Hands over every thumbnail asked for, as a host's poll does.
    fn serve(&mut self) {
        while let Some(r) = self.store.pop(0.0) {
            let px = r.step.px();
            self.store.complete(
                r.ticket,
                Outcome::Ready {
                    width: px,
                    height: px,
                    rgba: vec![160; (px * px * 4) as usize],
                },
                0.0,
            );
        }
    }
}

fn base(_: &Host) -> Collection<Host> {
    Collection::new(
        SCOPE,
        |s: &Host| (&s.src, &s.coll),
        |s: &mut Host| (&s.src, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
}

/// The grid's base: tile builds counted through the description, which C1e reads once a
/// build (the details, which the hit regions read too, would count presses).
fn grid_base(s: &Host) -> Collection<Host> {
    let built = s.built.clone();
    base(s).describe(crate::collection::element::Describe::pure(
        move |_: &Host, _| {
            built.set(built.get() + 1);
            None
        },
    ))
}

fn grid(_: &Host) -> IconsGrid<Host> {
    IconsGrid::new(
        SCOPE,
        grid_base,
        |s: &Host| IconsParts {
            source: &s.src,
            collection: &s.coll,
            grid: &s.grid,
            size: s.size,
            listing: 1,
            look: &s.look,
            thumbs: Some((&s.store, s.viewer)),
        },
        |s: &mut Host| &mut s.grid,
    )
    .thumbs_mut(|s: &mut Host| Some((&mut s.store, s.viewer, &s.src as &dyn EntrySource)))
    .details(|_: &Host, _| 1, |_: &Host, _| Some("12 kB".into()))
    .on_size(|s: &mut Host, r, _| {
        if let SizeRequest::Step { to, .. } = r {
            s.size = to;
        }
    })
}

/// C4's five columns, as its own perf test has them.
fn columns() -> Vec<Column<Host>> {
    vec![
        Column::primary("name", "Name")
            .width(160.0)
            .min(160.0)
            .sortable(),
        Column::new("mode", "Mode", |_: &Host, rcx| {
            Cell::text(if rcx.entry.kind.is_folder_like() {
                "drwxr-xr-x  755"
            } else {
                "-rw-r--r--  644"
            })
            .mono()
            .muted()
        })
        .width(160.0)
        .min(80.0),
        Column::new("size", "Size", |_: &Host, rcx| {
            Cell::text(format!("{} kB", rcx.entry.id.get() * 7919 % 100_000)).muted()
        })
        .width(90.0)
        .min(70.0)
        .align(Align::End),
        Column::new("type", "Type", |_: &Host, _| {
            Cell::text("Plain text").muted()
        })
        .width(120.0)
        .min(80.0),
        Column::new("modified", "Modified", |_: &Host, rcx| {
            Cell::text(format!("2026-01-{:02} 12:00", 1 + rcx.entry.id.get() % 28)).muted()
        })
        .width(150.0)
        .min(110.0),
    ]
}

fn table() -> ListTable<Host> {
    ListTable::new(
        SCOPE,
        columns(),
        |s: &Host| ListParts {
            source: &s.src,
            collection: &s.coll,
            table: &s.table,
            layout: &s.layout,
            widths: &s.widths,
            sort: None,
            look: &s.look,
        },
        |s: &mut Host| &mut s.table,
    )
}

fn view(s: &Host, cx: &mut Cx<Host>) -> El {
    VIEWS.with(|v| v.set(v.get() + 1));
    let body = match s.kind {
        Kind::Grid => grid(s).scale(cx.env().scale).build(s),
        Kind::Table => {
            let mut layer = Vec::new();
            let el = table().build(s, base(s), &mut layer);
            rune_kit::root(el, layer)
        }
        Kind::List => base(s).build(s),
        Kind::Packed => {
            let m = crate::icons_grid::TileMetrics::new(&s.look, s.size, 2, true);
            base(s)
                .lines(crate::icons_grid::GridLines::new(m))
                .row_height(move |_: &Host, _: &Look| m.height)
                .roles(crate::collection::element::Roles::GRID_CELLS)
                .build(s)
        }
    };
    col().size(s.width, SIDE).child(body)
}

#[derive(Clone, Copy, Debug, Default)]
struct Run {
    first: f64,
    scroll_p50: f64,
    scroll_p95: f64,
    scroll_max: f64,
    scroll_views: usize,
    click_p95: f64,
    built_per_click: f64,
    size_p95: f64,
    size_max: f64,
    reflow_p95: f64,
    reflow_max: f64,
}

impl Run {
    fn best(runs: &[Run]) -> Run {
        let min = |f: fn(&Run) -> f64| runs.iter().map(f).fold(f64::INFINITY, f64::min);
        Run {
            first: min(|r| r.first),
            scroll_p50: min(|r| r.scroll_p50),
            scroll_p95: min(|r| r.scroll_p95),
            scroll_max: min(|r| r.scroll_max),
            scroll_views: runs.iter().map(|r| r.scroll_views).max().unwrap_or(0),
            click_p95: min(|r| r.click_p95),
            built_per_click: min(|r| r.built_per_click),
            size_p95: min(|r| r.size_p95),
            size_max: min(|r| r.size_max),
            reflow_p95: min(|r| r.reflow_p95),
            reflow_max: min(|r| r.reflow_max),
        }
    }
}

fn one(kind: Kind, size: f32) -> Run {
    let mut run = Run::default();
    let t = Instant::now();
    let mut rt = Runtime::new(
        Host::new(kind, size),
        view,
        (SIDE, SIDE),
        Env::new(true, 2.0),
    );
    rt.frame(0.0);
    run.first = ms(t);
    for _ in 0..30 {
        rt.state.serve();
        rt.frame(1.0 / 120.0);
    }
    // Scrolling, with thumbnails landing as they are asked for.
    let before = views();
    let mut frames = Vec::with_capacity(600);
    for _ in 0..600 {
        rt.wheel(SIDE / 2.0, SIDE / 2.0, -48.0);
        rt.state.serve();
        let t = Instant::now();
        rt.frame(1.0 / 120.0);
        frames.push(ms(t));
    }
    run.scroll_views = views() - before;
    run.scroll_p50 = percentile(&mut frames, 0.5);
    run.scroll_p95 = percentile(&mut frames, 0.95);
    run.scroll_max = frames.iter().copied().fold(0.0, f64::max);
    for _ in 0..30 {
        rt.frame(1.0 / 120.0);
    }
    // Clicks on tiles across the viewport.
    let built = rt.state.built.get();
    let mut clicks = Vec::with_capacity(30);
    for k in 0..30 {
        let (x, y) = (60.0 + (k % 6) as f32 * 130.0, 120.0 + (k / 6) as f32 * 90.0);
        let t = Instant::now();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 120.0);
        clicks.push(ms(t));
    }
    run.click_p95 = percentile(&mut clicks, 0.95);
    run.built_per_click = (rt.state.built.get() - built) as f64 / 30.0;
    if kind != Kind::Grid {
        return run;
    }
    // The size ladder up from 32 and back, the cursor where the clicks left it.
    let mut steps = Vec::new();
    for dir in [1, -1] {
        for _ in 0..16 {
            let t = Instant::now();
            rt.update(|s: &mut Host, tx| grid(s).apply_size(s, SizeCommand::Step(dir), tx));
            rt.frame(1.0 / 120.0);
            steps.push(ms(t));
        }
    }
    run.size_max = steps.iter().copied().fold(0.0, f64::max);
    run.size_p95 = percentile(&mut steps, 0.95);
    // Reflows: a narrower width and back.
    let mut reflows = Vec::new();
    for k in 0..20 {
        let t = Instant::now();
        rt.update(|s: &mut Host, _| s.width = if k % 2 == 0 { 780.0 } else { SIDE });
        rt.frame(1.0 / 120.0);
        rt.frame(1.0 / 120.0);
        reflows.push(ms(t));
    }
    run.reflow_max = reflows.iter().copied().fold(0.0, f64::max);
    run.reflow_p95 = percentile(&mut reflows, 0.95);
    run
}

fn best(kind: Kind, size: f32) -> Run {
    let runs: Vec<Run> = (0..RUNS).map(|_| one(kind, size)).collect();
    Run::best(&runs)
}

/// The bar above, in absolute milliseconds where it isn't relative to C4.
///
/// Load-sensitive: the absolute bars, the size step's 16.7 ms above all, hold against wall
/// time, not against C4 in the same run, so a loaded machine can fail them on its own. A size
/// step is 6 to 8 ms at p95 at this machine's usual load (C3 review round 1) and failed once
/// under a 4 × slowdown (C3 review round 2: 22 ms against 16.7). Rerun a failure on an idle
/// machine before reading it as a regression; the bar stays.
#[test]
#[ignore = "perf: run with --profile perf -- --ignored --nocapture"]
fn a_100k_grid_scrolls_zooms_and_reflows_within_the_bar() {
    let list = best(Kind::List, 64.0);
    let table = best(Kind::Table, 64.0);
    eprintln!("C1e list:  {list:?}");
    eprintln!("C4 table:  {table:?}");
    for size in [32.0, 64.0] {
        let p = best(Kind::Packed, size);
        eprintln!(
            "C1e packed lines at {size} pt (default cells): p50 {:.3} p95 {:.3} max {:.3}",
            p.scroll_p50, p.scroll_p95, p.scroll_max
        );
    }
    let ceiling = 2.0 * table.scroll_p95;
    for size in [32.0, 64.0, 256.0] {
        let g = best(Kind::Grid, size);
        eprintln!("C3 grid {size:>3} pt: {g:?}");
        eprintln!(
            "  scroll p95 {:.3} ms = {:.2} × C4 (ceiling {:.3}), {:.2} × C1e",
            g.scroll_p95,
            g.scroll_p95 / table.scroll_p95,
            ceiling,
            g.scroll_p95 / list.scroll_p95
        );
        assert_eq!(g.scroll_views, 0, "no view run while scrolling at {size}");
        assert!(g.scroll_p95 <= ceiling, "scroll p95 at {size}");
        assert!(
            g.scroll_max <= FRAME_BUDGET,
            "a scroll frame over 16.7 ms at {size}"
        );
        assert!(g.click_p95 <= 4.0, "click p95 at {size}");
        assert!(g.built_per_click <= 4.0, "tiles built a click at {size}");
        assert!(g.size_p95 <= FRAME_BUDGET, "size step p95 at {size}");
        assert!(g.reflow_p95 <= FRAME_BUDGET, "reflow p95 at {size}");
    }
}

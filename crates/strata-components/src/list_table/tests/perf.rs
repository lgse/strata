//! Perf: 100,000 entries in five columns, held to rune-kit's Table with the same five columns,
//! measured side by side in the same run.
//!
//! `cargo test --profile perf -p strata-components --lib list_table::tests::perf -- --ignored --nocapture`
//!
//! As C1e's (`collection/element/tests/perf.rs`): a 900 pt window at scale 2, the first frame,
//! 600 frames at 120 Hz each scrolled 48 pt by the wheel, then 30 clicks on rows; a refresh, a
//! re-sort and a live insert. Each figure is the best of three runs. C4 adds:
//!
//! - 300 frames of sideways scrolling in a window narrower than the columns;
//! - a resize: 60 moves of a heading's edge, each a commit that rebuilds the rows on screen
//!   (Rune draft R-C4-4);
//! - a reorder: 60 moves of a heading past the threshold that keep their drop point;
//! - grouped: 100 groups of 1,000, built with `groups_from_runs`.
//!
//! The bar (plan §4 C1 and C4, work package C4: "100k rows, sharing C1e's bar"):
//! - no view run while scrolling, either way (grouped: once per group crossed), nor for
//!   reorder moves that keep their drop point;
//! - scroll frames within C1e's ceilings: max(1.5 × Table's, 0.5 ms) at p50 and
//!   max(1.5 × Table's, 1.5 ms) at p95, none over 16.7 ms; sideways frames within the same p95
//!   ceiling, none over 16.7 ms. The run also prints whether
//!   ListTable stays at or under Table's own p50 and p95, the target C1e met with one column
//!   (0.59 ms against 0.88 at p95). With five columns it doesn't quite (a few percent over at
//!   the pinned rev, with an icon per row that Table's rows don't draw), so that line is a
//!   report, not an assertion: see the work package's report;
//! - a click commits within 4 ms at p95 and rebuilds at most 4 rows on average;
//! - a refresh commits within 8 ms; a resize move within 8 ms at p95;
//! - a re-sort, a live insert and a regroup within the 60 ms guard (Rune issue #203).

use std::borrow::Cow;
use std::cell::Cell as StdCell;
use std::rc::Rc;
use std::time::Instant;

use rune::{Align, Cx, El, Env, Runtime, col};
use rune_kit::{Cell as KitCell, Kit, Table, TableColumn, TableState};
use strata_kit::look::Look;

use super::super::*;
use crate::collection::element::{Collection, CollectionState};
use crate::collection::selection::{CursorGone, SyncRule};
use crate::contract::{Entry, EntryId, EntryKind, EntrySource};

const N: usize = 100_000;
const SIDE: f32 = 900.0;
const NARROW: f32 = 500.0;
const RUNS: usize = 3;
const SCOPE: &str = "files";

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

/// `n` entries with dense ids, so `index_of` is a lookup in a vector.
struct Source {
    ids: Vec<EntryId>,
    names: Vec<String>,
    index: Vec<usize>,
}

impl Source {
    fn new(n: usize) -> Self {
        let mut s = Self {
            ids: (0..n as u64).map(EntryId::new).collect(),
            names: (0..n).map(|i| format!("entry {i:06}.txt")).collect(),
            index: Vec::new(),
        };
        s.reindex();
        s
    }

    fn reindex(&mut self) {
        self.index = vec![usize::MAX; self.names.len()];
        for (i, id) in self.ids.iter().enumerate() {
            self.index[id.get() as usize] = i;
        }
    }

    fn reverse(&mut self) {
        self.ids.reverse();
        self.reindex();
    }

    fn insert_top(&mut self, count: usize) {
        let first = self.names.len() as u64;
        for k in 0..count as u64 {
            self.names.push(format!("new {}.txt", first + k));
            self.ids.insert(0, EntryId::new(first + k));
        }
        self.reindex();
    }
}

impl EntrySource for Source {
    fn len(&self) -> usize {
        self.ids.len()
    }
    fn id_at(&self, index: usize) -> EntryId {
        self.ids[index]
    }
    fn index_of(&self, id: EntryId) -> Option<usize> {
        self.index
            .get(id.get() as usize)
            .copied()
            .filter(|&i| i != usize::MAX)
    }
    fn entry(&self, index: usize) -> Entry<'_> {
        let id = self.ids[index];
        let kind = if id.get().is_multiple_of(25) {
            EntryKind::Folder
        } else {
            EntryKind::File
        };
        Entry::new(id, &self.names[id.get() as usize], kind, "")
    }
}

struct Host {
    src: Source,
    coll: CollectionState,
    table: ListTableState,
    layout: ColumnLayout,
    widths: ColumnWidths,
    look: Look,
    version: u64,
    side: f32,
    /// Rows built: the label slot's calls.
    built: Rc<StdCell<usize>>,
}

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
        .align(Align::End)
        .sortable(),
        Column::new("type", "Type", |_: &Host, _| {
            Cell::text("Plain text").muted()
        })
        .width(120.0)
        .min(80.0)
        .sortable(),
        Column::new("modified", "Modified", |_: &Host, rcx| {
            Cell::text(format!("2026-01-{:02} 12:00", 1 + rcx.entry.id.get() % 28)).muted()
        })
        .width(150.0)
        .min(110.0)
        .sortable(),
    ]
}

impl Host {
    fn new(side: f32) -> Self {
        let mut coll = CollectionState::new();
        coll.controllers.pointer = Some(Box::new(crate::collection::pointer::PointerIntent::new()));
        Self {
            src: Source::new(N),
            coll,
            table: ListTableState::new(),
            layout: ColumnLayout::new(columns().iter().map(|c| c.id().clone())),
            widths: ColumnWidths::new(),
            look: Look::fallback(true),
            version: 1,
            side,
            built: Rc::default(),
        }
    }
}

fn table(s: &Host) -> ListTable<Host> {
    let built = s.built.clone();
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
    .label(crate::collection::element::Slot::pure(
        move |_: &Host, rcx| {
            built.set(built.get() + 1);
            rune::text(rcx.entry.name.to_owned()).truncate_middle()
        },
    ))
    .on_widths(|s: &mut Host, w, _| s.widths = w)
    .on_layout(|s: &mut Host, c, _| s.layout = c.layout)
}

fn base() -> Collection<Host> {
    Collection::new(
        SCOPE,
        |s: &Host| (&s.src, &s.coll),
        |s: &mut Host| (&s.src, &mut s.coll),
        |s: &Host| &s.look,
        |s: &Host| s.version,
    )
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    VIEWS.with(|v| v.set(v.get() + 1));
    let mut layer = Vec::new();
    let el = col()
        .size(s.side, SIDE)
        .child(table(s).build(s, base(), &mut layer));
    rune_kit::root(el, layer)
}

#[derive(Clone, Copy, Debug, Default)]
struct Run {
    first: f64,
    scroll_p50: f64,
    scroll_p95: f64,
    scroll_max: f64,
    scroll_views: usize,
    crossed: usize,
    click_p50: f64,
    click_p95: f64,
    rebuilt_per_click: f64,
    refresh: f64,
    resort: f64,
    insert: f64,
    hscroll_p95: f64,
    hscroll_max: f64,
    hscroll_views: usize,
    resize_p95: f64,
    reorder_views: usize,
    regroup: f64,
    groups_built: f64,
}

impl Run {
    fn best(runs: &[Run]) -> Run {
        let min = |f: fn(&Run) -> f64| runs.iter().map(f).fold(f64::INFINITY, f64::min);
        let max = |f: fn(&Run) -> usize| runs.iter().map(f).max().unwrap_or(0);
        Run {
            first: min(|r| r.first),
            scroll_p50: min(|r| r.scroll_p50),
            scroll_p95: min(|r| r.scroll_p95),
            scroll_max: min(|r| r.scroll_max),
            scroll_views: max(|r| r.scroll_views),
            crossed: max(|r| r.crossed),
            click_p50: min(|r| r.click_p50),
            click_p95: min(|r| r.click_p95),
            rebuilt_per_click: min(|r| r.rebuilt_per_click),
            refresh: min(|r| r.refresh),
            resort: min(|r| r.resort),
            insert: min(|r| r.insert),
            hscroll_p95: min(|r| r.hscroll_p95),
            hscroll_max: min(|r| r.hscroll_max),
            hscroll_views: max(|r| r.hscroll_views),
            resize_p95: min(|r| r.resize_p95),
            reorder_views: max(|r| r.reorder_views),
            regroup: min(|r| r.regroup),
            groups_built: min(|r| r.groups_built),
        }
    }
}

fn scroll<S: 'static>(rt: &mut Runtime<S>) -> Vec<f64> {
    let mut frames = Vec::with_capacity(600);
    for _ in 0..600 {
        rt.wheel(SIDE / 2.0, SIDE / 2.0, -48.0);
        let t = Instant::now();
        rt.frame(1.0 / 120.0);
        frames.push(ms(t));
    }
    frames
}

fn clicks<S: 'static>(rt: &mut Runtime<S>) -> Vec<f64> {
    let mut times = Vec::with_capacity(30);
    for k in 0..30 {
        let y = 200.0 + (k % 15) as f32 * 31.0;
        let t = Instant::now();
        rt.pointer_down(60.0, y);
        rt.pointer_up(60.0, y);
        rt.frame(1.0 / 120.0);
        times.push(ms(t));
    }
    times
}

fn views() -> usize {
    VIEWS.with(StdCell::get)
}

/// The group of `size` that entry `i` is in, as `groups_from_runs` reads it.
fn group_key(i: usize, size: usize) -> (u64, std::sync::Arc<str>) {
    let g = (i / size) as u64;
    (g, std::sync::Arc::from(format!("Type {g}")))
}

fn list_run(group: Option<usize>) -> Run {
    let mut run = Run::default();
    let mut host = Host::new(SIDE);
    if let Some(size) = group {
        let t = Instant::now();
        let groups = groups_from_runs(N, |i| group_key(i, size));
        run.groups_built = ms(t);
        host.coll.set_groups(groups);
    }
    let t = Instant::now();
    let mut rt = Runtime::new(host, view, (SIDE, SIDE), Env::new(true, 2.0));
    rt.frame(0.0);
    rt.frame(1.0 / 120.0);
    run.first = ms(t);

    let h = strata_kit::slots::list::row_height(&rt.state.look);
    let top_group = |offset: f32| group.map_or(0, |size| (offset / h) as usize / (size + 1));
    let start = top_group(rt.state.coll.scroll_offset());
    let v = views();
    let mut frames = scroll(&mut rt);
    run.scroll_views = views() - v;
    run.crossed = top_group(rt.state.coll.scroll_offset()) - start;
    run.scroll_max = frames.iter().copied().fold(0.0, f64::max);
    run.scroll_p50 = percentile(&mut frames, 0.5);
    run.scroll_p95 = percentile(&mut frames, 0.95);
    assert!(
        rt.state.coll.scroll_offset() > 20_000.0,
        "the list scrolled"
    );
    for _ in 0..30 {
        rt.frame(1.0 / 120.0);
    }
    let built = rt.state.built.get();
    let mut times = clicks(&mut rt);
    run.rebuilt_per_click = (rt.state.built.get() - built) as f64 / times.len() as f64;
    run.click_p50 = percentile(&mut times, 0.5);
    run.click_p95 = percentile(&mut times, 0.95);
    assert!(!rt.state.coll.selection.is_empty(), "the clicks selected");

    // A resize: 60 moves of Size's edge, each a commit.
    let head = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(heading_key("size").as_str()))
        .expect("Size's heading");
    let [x, y, w, hh] = head.visible;
    let (px, py) = (x + w - 3.0, y + hh / 2.0);
    rt.pointer_down(px, py);
    rt.frame(1.0 / 120.0);
    let mut moves = Vec::with_capacity(60);
    for k in 1..=60 {
        rt.pointer_move(px + k as f32, py);
        let t = Instant::now();
        rt.frame(1.0 / 120.0);
        moves.push(ms(t));
    }
    rt.pointer_up(px + 60.0, py);
    rt.frame(1.0 / 120.0);
    run.resize_p95 = percentile(&mut moves, 0.95);

    // A reorder: Mode's heading past the threshold, then 60 moves inside one drop point.
    let head = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(heading_key("mode").as_str()))
        .expect("Mode's heading");
    let [x, y, w, hh] = head.visible;
    let (px, py) = (x + w / 2.0, y + hh / 2.0);
    rt.pointer_down(px, py);
    rt.frame(1.0 / 120.0);
    rt.pointer_move(px + 12.0, py);
    rt.frame(1.0 / 120.0);
    let v = views();
    for k in 0..60 {
        rt.pointer_move(px + 12.0 + (k % 3) as f32, py);
        rt.frame(1.0 / 120.0);
    }
    run.reorder_views = views() - v;
    rt.key(rune::KeyEvent {
        key: rune::Key::Escape,
        mods: rune::Mods::default(),
    });
    rt.pointer_up(px + 12.0, py);
    rt.frame(1.0 / 120.0);

    let t = Instant::now();
    rt.update(|s, tx| {
        s.version += 1;
        s.coll
            .sync(SCOPE, &s.src, SyncRule::complete(CursorGone::Neighbor), tx);
    });
    rt.frame(1.0 / 120.0);
    run.refresh = ms(t);

    let t = Instant::now();
    rt.update(|s, _| {
        s.src.reverse();
        s.version += 1;
    });
    rt.frame(1.0 / 120.0);
    run.resort = ms(t);

    let t = Instant::now();
    rt.update(move |s, tx| {
        s.src.insert_top(10);
        if let Some(size) = group {
            let n = s.src.len();
            s.coll
                .set_groups(groups_from_runs(n, |i| group_key(i, size)));
        }
        s.coll
            .sync(SCOPE, &s.src, SyncRule::complete(CursorGone::Neighbor), tx);
    });
    rt.frame(1.0 / 120.0);
    run.insert = ms(t);

    if let Some(size) = group {
        let n = rt.state.src.len();
        let t = Instant::now();
        rt.update(move |s, _| {
            s.coll
                .set_groups(groups_from_runs(n, |i| group_key(i, size + 1)));
        });
        rt.frame(1.0 / 120.0);
        run.regroup = ms(t);
    }

    // Sideways, in a window narrower than the columns.
    let mut rt = Runtime::new(Host::new(NARROW), view, (NARROW, SIDE), Env::new(true, 2.0));
    rt.frame(0.0);
    for _ in 0..10 {
        rt.frame(1.0 / 120.0);
    }
    let v = views();
    let mut frames = Vec::with_capacity(300);
    for k in 0..300 {
        let dx = if (k / 30) % 2 == 0 { -4.0 } else { 4.0 };
        rt.wheel_event(rune::WheelEvent::new(dx, 0.0).at(NARROW / 2.0, SIDE / 2.0));
        let t = Instant::now();
        rt.frame(1.0 / 120.0);
        frames.push(ms(t));
    }
    run.hscroll_views = views() - v;
    run.hscroll_max = frames.iter().copied().fold(0.0, f64::max);
    run.hscroll_p95 = percentile(&mut frames, 0.95);
    run
}

// rune-kit's Table with the same five columns.

struct File {
    id: String,
    name: String,
    size: u64,
}

struct TableApp {
    files: Vec<File>,
    table: TableState,
}

fn table_view(s: &TableApp, _: &mut Cx<TableApp>) -> El {
    let k = Kit::neutral(true);
    col().size(SIDE, SIDE).child(
        Table::new(
            |s: &TableApp| &s.table,
            |s: &mut TableApp| &mut s.table,
            |s: &TableApp| s.files.len(),
            |s: &TableApp, i| Cow::Borrowed(s.files[i].id.as_str()),
        )
        .name("Files")
        .column(
            TableColumn::new("name", "Name")
                .flex(1.0)
                .min(160.0)
                .sortable(),
        )
        .column(TableColumn::new("mode", "Mode").width(160.0).min(80.0))
        .column(
            TableColumn::new("size", "Size")
                .width(90.0)
                .min(70.0)
                .align(Align::End)
                .sortable(),
        )
        .column(
            TableColumn::new("type", "Type")
                .width(120.0)
                .min(80.0)
                .sortable(),
        )
        .column(
            TableColumn::new("modified", "Modified")
                .width(150.0)
                .min(110.0)
                .sortable(),
        )
        .cell(|s: &TableApp, i, col| match col {
            "name" => KitCell::text(&s.files[i].name),
            "mode" => KitCell::text("-rw-r--r--  644").mono().muted(),
            "size" => KitCell::text(format!("{} kB", s.files[i].size)).muted(),
            "type" => KitCell::text("Plain text").muted(),
            _ => KitCell::text(format!("2026-01-{:02} 12:00", 1 + i % 28)).muted(),
        })
        .build(s, &k)
        .h(SIDE),
    )
}

fn table_run() -> Run {
    let files = (0..N)
        .map(|i| File {
            id: format!("f{i}"),
            name: format!("entry {i:06}.txt"),
            size: (i as u64 * 7919) % 100_000,
        })
        .collect();
    let app = TableApp {
        files,
        table: TableState::new().multi_select(true),
    };
    let mut run = Run::default();
    let t = Instant::now();
    let mut rt = Runtime::new(app, table_view, (SIDE, SIDE), Env::new(true, 2.0));
    rt.frame(0.0);
    run.first = ms(t);
    let mut frames = scroll(&mut rt);
    run.scroll_max = frames.iter().copied().fold(0.0, f64::max);
    run.scroll_p50 = percentile(&mut frames, 0.5);
    run.scroll_p95 = percentile(&mut frames, 0.95);
    for _ in 0..30 {
        rt.frame(1.0 / 120.0);
    }
    let mut times = clicks(&mut rt);
    run.click_p50 = percentile(&mut times, 0.5);
    run.click_p95 = percentile(&mut times, 0.95);
    assert!(rt.state.table.selected_len() > 0, "the clicks selected");
    run
}

#[test]
#[ignore = "perf"]
fn perf_hundred_thousand_entries_in_five_columns_against_rune_kit_table() {
    let (mut plain, mut grouped, mut table) = (vec![], vec![], vec![]);
    for _ in 0..RUNS {
        table.push(table_run());
        plain.push(list_run(None));
        grouped.push(list_run(Some(N / 100)));
    }
    let t = Run::best(&table);
    let runs = [
        ("ListTable", Run::best(&plain)),
        ("100 groups", Run::best(&grouped)),
    ];
    println!("100k entries, five columns, 900 pt at scale 2, best of {RUNS}:");
    println!(
        "  {:28} {:>12} {:>12} {:>10}",
        "", runs[0].0, runs[1].0, "Table"
    );
    let row = |name: &str, f: fn(&Run) -> f64, table: Option<f64>| {
        let t = table.map_or_else(String::new, |v| format!("{v:10.3}"));
        println!(
            "  {name:28} {:12.3} {:12.3} {t}",
            f(&runs[0].1),
            f(&runs[1].1)
        );
    };
    row("first frame ms", |r| r.first, Some(t.first));
    row("scroll p50 ms", |r| r.scroll_p50, Some(t.scroll_p50));
    row("scroll p95 ms", |r| r.scroll_p95, Some(t.scroll_p95));
    row("scroll max ms", |r| r.scroll_max, Some(t.scroll_max));
    row("view runs scrolling", |r| r.scroll_views as f64, None);
    row("groups crossed", |r| r.crossed as f64, None);
    row("click commit p50 ms", |r| r.click_p50, Some(t.click_p50));
    row("click commit p95 ms", |r| r.click_p95, Some(t.click_p95));
    row("rows rebuilt/click", |r| r.rebuilt_per_click, None);
    row("refresh commit ms", |r| r.refresh, None);
    row("re-sort (#203) ms", |r| r.resort, None);
    row("live insert (#203) ms", |r| r.insert, None);
    row("sideways scroll p95 ms", |r| r.hscroll_p95, None);
    row("sideways scroll max ms", |r| r.hscroll_max, None);
    row("view runs sideways", |r| r.hscroll_views as f64, None);
    row("resize move commit p95 ms", |r| r.resize_p95, None);
    row(
        "view runs, 60 reorder moves",
        |r| r.reorder_views as f64,
        None,
    );
    row("groups_from_runs 100k ms", |r| r.groups_built, None);
    row("regroup ms", |r| r.regroup, None);

    for (name, c) in &runs {
        assert_eq!(
            c.scroll_views, c.crossed,
            "{name}: scrolling ran the view other than once per group crossed"
        );
        assert_eq!(
            c.hscroll_views, 0,
            "{name}: sideways scrolling ran the view"
        );
        assert_eq!(c.reorder_views, 0, "{name}: reorder moves ran the view");
        println!(
            "  {name}: scroll p50 {:.2}× and p95 {:.2}× Table's: {}",
            c.scroll_p50 / t.scroll_p50,
            c.scroll_p95 / t.scroll_p95,
            if c.scroll_p50 <= t.scroll_p50 && c.scroll_p95 <= t.scroll_p95 {
                "at or under Table"
            } else {
                "over Table (target missed)"
            }
        );
        assert!(
            c.scroll_p50 <= (1.5 * t.scroll_p50).max(0.5),
            "{name}: scroll p50 {:.3} ms over the bar (Table {:.3} ms)",
            c.scroll_p50,
            t.scroll_p50
        );
        assert!(
            c.scroll_p95 <= (1.5 * t.scroll_p95).max(1.5),
            "{name}: scroll p95 {:.3} ms over the bar (Table {:.3} ms)",
            c.scroll_p95,
            t.scroll_p95
        );
        assert!(
            c.hscroll_p95 <= (1.5 * t.scroll_p95).max(1.5),
            "{name}: sideways p95"
        );
        assert!(
            c.hscroll_max < 16.7,
            "{name}: a sideways scroll frame took {:.2} ms",
            c.hscroll_max
        );
        assert!(
            c.scroll_max < 16.7,
            "{name}: a scroll frame took {:.2} ms",
            c.scroll_max
        );
        assert!(
            c.click_p95 < 4.0,
            "{name}: click commit p95 {:.3} ms",
            c.click_p95
        );
        assert!(
            c.rebuilt_per_click <= 4.0,
            "{name}: {:.2} rows rebuilt per click",
            c.rebuilt_per_click
        );
        assert!(
            c.refresh < 8.0,
            "{name}: refresh commit {:.2} ms",
            c.refresh
        );
        assert!(
            c.resize_p95 < 8.0,
            "{name}: resize move p95 {:.2} ms",
            c.resize_p95
        );
        assert!(c.resort < 60.0, "{name}: re-sort commit {:.2} ms", c.resort);
        assert!(
            c.insert < 60.0,
            "{name}: live insert commit {:.2} ms",
            c.insert
        );
        assert!(
            c.regroup < 60.0,
            "{name}: regroup commit {:.2} ms",
            c.regroup
        );
    }
    assert_eq!(runs[0].1.crossed, 0);
    assert!(runs[1].1.crossed >= 1, "the grouped run crossed a group");
}

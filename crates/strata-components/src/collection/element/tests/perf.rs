//! Perf: 100,000 entries, held to rune-kit's Table, measured side by side in the same run.
//!
//! `cargo test --profile perf -p strata-components --lib collection::element::tests::perf -- --ignored --nocapture`
//!
//! Both lists fill a 900 pt window at scale 2, as rune-kit's own `table_time` does
//! (`crates/rune-kit/tests/table.rs` at the pinned rev): the first frame, then 600 frames at
//! 120 Hz each scrolled 48 pt by the wheel, then 30 clicks on rows. The element also times a
//! refresh (a new listing version, the same entries in the same order), a re-sort (a new
//! version and a new order) and a live update that adds rows (a new length). Each figure is
//! the best of three runs.
//!
//! A re-sort and a change of length cost Rune O(rows) at the pinned rev: it re-reads, copies
//! and re-maps every row's key (Rune issue #203). They happen once per user action, not per
//! frame, so they are reported and guarded against regressions (60 ms), not held to the frame
//! budget.
//!
//! The bar (plan §4 C1, "no view commit per frame"; work package C1e):
//! - no view run while scrolling;
//! - no view run for the moves of a drag that changes nothing;
//! - scroll frames at p50 within max(1.5 × Table's, 0.5 ms) and p95 within max(1.5 × Table's,
//!   1.5 ms), and none over a 60 Hz frame (16.7 ms);
//! - a click that changes the selection commits within 4 ms at p95, and rebuilds at most 4
//!   rows on average;
//! - a refresh commits within 8 ms (`RUNE_COMMIT_BUDGET_MS=8`).
//!
//! Grouped (C1f) runs the same in 100 groups of 1,000 and in 2,000 groups of 50, held to the
//! same bars except that scrolling builds the view once per group crossed (the stuck header's
//! label, see the element's Decisions) and never otherwise; each crosses at least one. A
//! regroup (new groups, the same entries) is reported against the re-sort's guard.

use std::borrow::Cow;
use std::cell::Cell;
use std::time::Instant;

use rune::{Align, Cx, El, Env, Runtime, col};
use rune_kit::{Cell as KitCell, Kit, Table, TableColumn, TableState};

use super::*;

const N: usize = 100_000;
const SIDE: f32 = 900.0;
const RUNS: usize = 3;

thread_local! {
    static VIEWS: Cell<usize> = const { Cell::new(0) };
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

#[derive(Clone, Copy, Debug, Default)]
struct Run {
    first: f64,
    scroll_p50: f64,
    scroll_p95: f64,
    scroll_max: f64,
    scroll_views: usize,
    drag_views: usize,
    click_p50: f64,
    click_p95: f64,
    rebuilt_per_click: f64,
    refresh: f64,
    resort: f64,
    insert: f64,
    /// Groups crossed while scrolling.
    crossed: usize,
    regroup: f64,
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
            drag_views: runs.iter().map(|r| r.drag_views).max().unwrap_or(0),
            click_p50: min(|r| r.click_p50),
            click_p95: min(|r| r.click_p95),
            rebuilt_per_click: min(|r| r.rebuilt_per_click),
            refresh: min(|r| r.refresh),
            resort: min(|r| r.resort),
            insert: min(|r| r.insert),
            crossed: runs.iter().map(|r| r.crossed).max().unwrap_or(0),
            regroup: min(|r| r.regroup),
        }
    }
}

/// Scrolls 600 frames; returns the frame times.
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

/// Clicks 30 rows down the middle of the window; returns the commit times.
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

// The element.

fn element_view(s: &Host, cx: &mut Cx<Host>) -> El {
    VIEWS.with(|v| v.set(v.get() + 1));
    let _ = cx;
    col().size(SIDE, SIDE).child(collection(s).build(s))
}

/// The element over `N` entries, in groups of `group` when given.
fn element_run(group: Option<usize>) -> Run {
    let mut run = Run::default();
    let mut host = Host::new(N);
    if let Some(size) = group {
        host.coll.set_groups(super::groups_of(N, size, "Group"));
    }
    let t = Instant::now();
    let mut rt = Runtime::new(host, element_view, (SIDE, SIDE), Env::new(true, 2.0));
    rt.frame(0.0);
    run.first = ms(t);

    let h = rt.state.look.row.list;
    let top_group = |offset: f32| group.map_or(0, |size| (offset / h) as usize / (size + 1));
    let start = top_group(rt.state.coll.scroll_offset());
    let views = VIEWS.with(Cell::get);
    let mut frames = scroll(&mut rt);
    run.scroll_views = VIEWS.with(Cell::get) - views;
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

    // A press dragged across rows: with no controller taking it, its moves change nothing.
    rt.pointer_down(300.0, 300.0);
    rt.frame(1.0 / 120.0);
    let views = VIEWS.with(Cell::get);
    for k in 1..=60 {
        rt.pointer_move(300.0, 300.0 + k as f32 * 5.0);
        rt.frame(1.0 / 120.0);
    }
    run.drag_views = VIEWS.with(Cell::get) - views;
    rt.pointer_up(300.0, 600.0);
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
            s.coll
                .set_groups(super::groups_of(s.src.len(), size, "Group"));
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
            s.coll.set_groups(super::groups_of(n, size + 1, "Regroup"));
        });
        rt.frame(1.0 / 120.0);
        run.regroup = ms(t);
    }
    run
}

// rune-kit's Table, as its own benchmark builds it.

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
                .min(120.0)
                .sortable(),
        )
        .column(
            TableColumn::new("size", "Size")
                .width(100.0)
                .min(60.0)
                .max(180.0)
                .align(Align::End)
                .sortable(),
        )
        .column(
            TableColumn::new("kind", "Kind")
                .width(80.0)
                .resizable(false),
        )
        .cell(|s: &TableApp, i, col| match col {
            "name" => KitCell::text(&s.files[i].name),
            "size" => KitCell::text(s.files[i].size.to_string()).mono(),
            _ => KitCell::text("Rust").muted(),
        })
        .zebra(true)
        .build(s, &k)
        .h(SIDE),
    )
}

fn table_run() -> Run {
    let files = (0..N)
        .map(|i| File {
            id: format!("f{i}"),
            name: format!("file {i} with a name long enough to cut short.rs"),
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
fn perf_hundred_thousand_entries_against_rune_kit_table() {
    let (mut plain, mut hundred, mut dense, mut table) = (vec![], vec![], vec![], vec![]);
    for _ in 0..RUNS {
        table.push(table_run());
        plain.push(element_run(None));
        hundred.push(element_run(Some(N / 100)));
        dense.push(element_run(Some(50)));
    }
    let t = Run::best(&table);
    let runs = [
        ("C1e", Run::best(&plain)),
        ("100 groups", Run::best(&hundred)),
        ("2,000 groups", Run::best(&dense)),
    ];
    println!("100k entries, 900 pt at scale 2, best of {RUNS}:");
    println!(
        "  {:24} {:>12} {:>12} {:>12} {:>10}",
        "", runs[0].0, runs[1].0, runs[2].0, "Table"
    );
    let row = |name: &str, f: fn(&Run) -> f64, table: Option<f64>| {
        let t = table.map_or_else(String::new, |v| format!("{v:10.3}"));
        println!(
            "  {name:24} {:12.3} {:12.3} {:12.3} {t}",
            f(&runs[0].1),
            f(&runs[1].1),
            f(&runs[2].1)
        );
    };
    row("first frame ms", |r| r.first, Some(t.first));
    row("scroll p50 ms", |r| r.scroll_p50, Some(t.scroll_p50));
    row("scroll p95 ms", |r| r.scroll_p95, Some(t.scroll_p95));
    row("scroll max ms", |r| r.scroll_max, Some(t.scroll_max));
    row("view runs scrolling", |r| r.scroll_views as f64, None);
    row("groups crossed", |r| r.crossed as f64, None);
    row("view runs, 60 drag moves", |r| r.drag_views as f64, None);
    row("click commit p50 ms", |r| r.click_p50, Some(t.click_p50));
    row("click commit p95 ms", |r| r.click_p95, Some(t.click_p95));
    row("rows rebuilt/click", |r| r.rebuilt_per_click, None);
    row("refresh commit ms", |r| r.refresh, None);
    row("re-sort (#203) ms", |r| r.resort, None);
    row("live insert (#203) ms", |r| r.insert, None);
    row("regroup ms", |r| r.regroup, None);

    for (name, c) in &runs {
        assert_eq!(
            c.scroll_views, c.crossed,
            "{name}: scrolling ran the view other than once per group crossed"
        );
        assert_eq!(c.drag_views, 0, "{name}: drag moves ran the view");
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
    assert!(runs[1].1.crossed >= 1, "the 100-group run crossed a group");
    assert!(runs[2].1.crossed > 5, "the dense run crossed groups");
}

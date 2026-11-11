//! Perf on the sheets Strata's budget lets through: 100,000 rows of four columns (more values
//! than the budget's 100,000, as rune-kit's Table measures), 390 rows of 256 columns (the most
//! columns at that budget) and an 18,000-row sheet arriving. `#[ignore]`d; run with the `perf`
//! profile, and Rune's commit budget to hear slow commits:
//!
//! ```sh
//! RUNE_COMMIT_BUDGET_MS=8 cargo test --profile perf -p strata-components --lib \
//!     sheet_grid::tests::perf -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Each claim prints one line: open (hand-over and first frame), the wheel (600 frames at
//! 120 Hz, 48 pt each), Ctrl+End and Ctrl+Home, a cell click, sorts (a text column, its flip,
//! a number column), Ctrl+C of every row, a held range drag's autoscroll, and on the wide sheet
//! the wheel down and sideways.

use std::cell::Cell as StdCell;
use std::time::Instant;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, clipboard};
use strata_kit::look::Look;

use super::super::*;

thread_local! {
    static VIEWS: StdCell<usize> = const { StdCell::new(0) };
}

fn release_only() {
    if cfg!(debug_assertions) {
        panic!("perf tests measure release builds: run them with --profile perf");
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn stats(v: &mut [f64]) -> String {
    v.sort_by(f64::total_cmp);
    let at = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    format!(
        "p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms, {} over 8 ms",
        at(0.5),
        at(0.95),
        at(1.0),
        v.iter().filter(|&&f| f > 8.0).count()
    )
}

struct Host {
    grid: SheetGridState,
    look: Look,
    now: f64,
}

fn view(h: &Host, cx: &mut Cx<Host>) -> El {
    VIEWS.set(VIEWS.get() + 1);
    SheetGrid::new(|h: &Host| &h.grid, |h: &mut Host| &mut h.grid).build(h, &h.look, cx)
}

const W: f32 = 960.0;
const H: f32 = 640.0;

fn frame(rt: &mut Runtime<Host>, dt: f64) -> f64 {
    let t = Instant::now();
    rt.update_if(|h, tx| {
        h.now += dt;
        let (now, look) = (h.now, h.look.clone());
        h.grid.tick(now, &look, tx)
    });
    rt.frame(dt);
    ms(t)
}

fn settle(rt: &mut Runtime<Host>) {
    for _ in 0..90 {
        frame(rt, 1.0 / 60.0);
    }
}

fn mount(sheet: Sheet) -> (Runtime<Host>, f64, f64) {
    let t = Instant::now();
    let mut grid = SheetGridState::new("g");
    grid.set_single(sheet);
    let handed = ms(t);
    let host = Host {
        grid,
        look: Look::fallback(false),
        now: 0.0,
    };
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    let t = Instant::now();
    rt.frame(0.0);
    let first = ms(t);
    settle(&mut rt);
    (rt, handed, first)
}

fn rows(n: usize, cols: usize) -> Sheet {
    let mut r = vec![(0..cols).map(|j| format!("col {j}")).collect::<Vec<_>>()];
    for i in 0..n {
        r.push(
            (0..cols)
                .map(|j| match j {
                    0 => format!("{i}"),
                    1 => ["info", "warn", "error", "debug"][(i * 7) % 4].to_owned(),
                    2 => format!("event {}", (i * 7919) % 100_000),
                    _ => format!("{}", (i * 31 + j * 17) % 997),
                })
                .collect(),
        );
    }
    Sheet::new("perf", r)
}

fn at(rt: &Runtime<Host>, end: &str) -> (f32, f32) {
    let r = rt
        .elements()
        .into_iter()
        .find(|e| e.path.ends_with(end))
        .unwrap_or_else(|| panic!("no {end}"))
        .rect;
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

fn tap(rt: &mut Runtime<Host>, p: (f32, f32)) -> f64 {
    rt.pointer_move(p.0, p.1);
    rt.pointer_down(p.0, p.1);
    let a = frame(rt, 1.0 / 120.0);
    rt.pointer_up(p.0, p.1);
    a + frame(rt, 1.0 / 120.0)
}

fn key(rt: &mut Runtime<Host>, key: Key, mods: Mods) -> f64 {
    rt.key(KeyEvent { key, mods });
    frame(rt, 1.0 / 120.0)
}

const CMD: Mods = Mods {
    cmd: true,
    ctrl: true,
    shift: false,
    alt: false,
    logo: false,
};

fn wheel(rt: &mut Runtime<Host>, dx: f32, dy: f32) -> (String, usize) {
    VIEWS.set(0);
    let mut frames = Vec::new();
    for _ in 0..600 {
        let t = Instant::now();
        rt.wheel_xy(W / 2.0, H / 2.0, dx, dy);
        frame(rt, 1.0 / 120.0);
        frames.push(ms(t));
    }
    (stats(&mut frames), VIEWS.get())
}

#[test]
#[ignore]
fn a_100k_row_sheet() {
    release_only();
    let (mut rt, handed, first) = mount(rows(100_000, 4));
    println!(
        "c11b perf: open 100k x 4: hand-over (initial sort) {handed:.1} ms, first frame {first:.1} ms"
    );

    let (w, views) = wheel(&mut rt, 0.0, -48.0);
    println!("c11b perf: wheel 600 frames: {w}, {views} view runs");

    let mut jumps = Vec::new();
    // A cell on screen after the wheel, to focus the grid.
    let shown = rt
        .elements()
        .into_iter()
        .find(|e| e.path.ends_with("-c0") && e.rect[1] > H / 3.0 && e.rect[1] < H / 2.0)
        .expect("a cell on screen")
        .rect;
    tap(&mut rt, (shown[0] + 4.0, shown[1] + 4.0));
    for i in 0..20 {
        let k = if i % 2 == 0 { Key::End } else { Key::Home };
        let t = Instant::now();
        key(&mut rt, k, CMD);
        frame(&mut rt, 1.0 / 120.0);
        jumps.push(ms(t));
        settle(&mut rt);
    }
    println!(
        "c11b perf: Ctrl+End and Ctrl+Home (key and next frame): {}",
        stats(&mut jumps)
    );

    let mut clicks = Vec::new();
    for i in 0..20 {
        let p = at(&rt, &format!("g-r{}-c1", i % 8));
        clicks.push(tap(&mut rt, p));
    }
    println!(
        "c11b perf: a cell click (press and release frames): {}",
        stats(&mut clicks)
    );

    let title = |rt: &mut Runtime<Host>, j: usize| {
        let p = at(rt, &format!("g-h{j}"));
        let t = tap(rt, p);
        settle(rt);
        t
    };
    let text = title(&mut rt, 2);
    let flip = title(&mut rt, 2);
    let number = title(&mut rt, 3);
    println!(
        "c11b perf: sort (title click frames): text column {text:.1} ms, its flip {flip:.1} ms, number column {number:.1} ms"
    );

    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let t = Instant::now();
    rt.key(KeyEvent {
        key: Key::Command('c'),
        mods: CMD,
    });
    frame(&mut rt, 1.0 / 120.0);
    let copied = ms(t);
    let bytes = clip.text().map_or(0, |t| t.len());
    println!("c11b perf: Ctrl+C every row: {copied:.1} ms, {bytes} bytes");

    let from = at(&rt, "g-r0-c1");
    rt.pointer_move(from.0, from.1);
    rt.pointer_down(from.0, from.1);
    frame(&mut rt, 1.0 / 120.0);
    rt.pointer_move(from.0, H - 3.0);
    let mut held = Vec::new();
    for _ in 0..240 {
        held.push(frame(&mut rt, 1.0 / 120.0));
    }
    rt.pointer_up(from.0, H - 3.0);
    let reached = rt.state.grid.cursor().map_or(0, |c| c.row);
    println!(
        "c11b perf: autoscroll, 240 frames held at the bottom: {}, the range reached row {reached}",
        stats(&mut held)
    );
}

#[test]
#[ignore]
fn a_390_by_256_sheet() {
    release_only();
    let (mut rt, handed, first) = mount(rows(390, 256));
    println!("c11b perf: open 390 x 256: hand-over {handed:.1} ms, first frame {first:.1} ms");
    let (down, views) = wheel(&mut rt, 0.0, -24.0);
    println!("c11b perf: wide wheel down 600 frames: {down}, {views} view runs");
    let (side, views) = wheel(&mut rt, -48.0, 0.0);
    println!("c11b perf: wide wheel sideways 600 frames: {side}, {views} view runs");
}

#[test]
#[ignore]
fn an_18k_row_sheet_arriving() {
    release_only();
    let (mut rt, _, _) = mount(rows(10, 4));
    let sheet = rows(18_000, 4);
    let t = Instant::now();
    rt.update(|h, _| h.grid.set_sheet(0, sheet));
    let handed = ms(t);
    let first = frame(&mut rt, 1.0 / 60.0);
    println!("c11b perf: 18k x 4 arriving: set_sheet {handed:.1} ms, next frame {first:.1} ms");
}

//! Perf tests on the S4 fixtures (`tests/fixtures/code/`), against S4's budgets: a viewport
//! within 8 ms at p95 (the reading N = 64 is sized for), and an edit's foreground within
//! 2 ms. They time the engine on the test thread, which is the work the highlighter thread
//! does. `#[ignore]`d; run them with the `perf` profile, one at a time:
//!
//! ```sh
//! CARGO_BUILD_JOBS=8 cargo test --profile perf -p strata-components --lib \
//!     code_view::highlight::tests::perf -- --ignored --test-threads=1 --nocapture
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::engine::{Engine, OpenArgs, Reply};
use super::super::{CHECKPOINT_EVERY, LONG_LINE, LineEdit, Mode, OVERSCAN, Options, Source};
use super::{fixture, light, reference, syntax_for};

/// S4's viewport: 80 visible lines plus the overscan.
const VISIBLE: usize = 80;
const POSITIONS: usize = 200;

fn release_only() {
    if cfg!(debug_assertions) {
        panic!("perf tests measure release builds: run them with --profile perf");
    }
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// p50, p95 and max, in milliseconds.
fn summary(mut ms: Vec<f64>) -> (f64, f64, f64) {
    ms.sort_by(f64::total_cmp);
    let at = |q: f64| ms[((ms.len() as f64 * q) as usize).min(ms.len() - 1)];
    (at(0.5), at(0.95), ms[ms.len() - 1])
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// Runs every pass to its end, dropping the replies.
fn drain(engine: &mut Engine) {
    while engine.step() {
        engine.take_out();
    }
    engine.take_out();
}

fn open(engine: &mut Engine, name: &str, text: &str, mode: Mode) {
    engine.open(
        1,
        1,
        OpenArgs {
            text: text.into(),
            file_name: Some(name.into()),
            syntax: None,
            mode,
        },
    );
}

fn viewport(name: &str) {
    release_only();
    let text = fixture(name);
    let mut engine = Engine::new(light(), Options::default());
    open(&mut engine, name, &text, Mode::Preview);
    let t = Instant::now();
    drain(&mut engine);
    let pass = t.elapsed();
    let lines = text.split_inclusive('\n').count() + 1;
    let window = VISIBLE + OVERSCAN;
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut times = Vec::with_capacity(POSITIONS);
    for i in 0..POSITIONS {
        // The worst case: a viewport starting just before the next checkpoint, so the
        // resume parses N - 1 lines before it. The last position is the file's end.
        let start = if i + 1 == POSITIONS {
            lines - window
        } else {
            let k = 1 + rng.below(lines / CHECKPOINT_EVERY - 3);
            k * CHECKPOINT_EVERY + CHECKPOINT_EVERY - 1
        };
        let t = Instant::now();
        engine.show(1, 2, start..start + window);
        engine.step();
        let out = engine.take_out();
        times.push(ms(t.elapsed()));
        assert!(out.iter().any(|(_, r)| matches!(r, Reply::Lines { .. })));
    }
    let (p50, p95, max) = summary(times);
    println!(
        "viewport {name}: p50 {p50:.2} ms, p95 {p95:.2} ms, max {max:.2} ms \
         (N = {CHECKPOINT_EVERY}, {window} lines; checkpoint pass {:.0} ms)",
        ms(pass)
    );
    assert!(
        p95 <= 8.0,
        "{name}: viewport p95 {p95:.2} ms is over the 8 ms budget"
    );
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn viewport_rust() {
    viewport("rust-50k.rs");
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn viewport_c() {
    viewport("c-50k.c");
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn viewport_js() {
    viewport("js-50k.js");
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn viewport_python() {
    viewport("python-50k.py");
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn viewport_markdown() {
    viewport("markdown-50k.md");
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn viewport_shell() {
    viewport("shell-5k.sh");
}

/// The kinds of edit S4 measured on the 5,000-line script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Rename,
    UnmatchedQuote,
    Heredoc,
    Case,
}

fn edited(lines: &[&str], at: usize, kind: Kind) -> Option<(String, LineEdit)> {
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    let edit = match kind {
        Kind::Rename => {
            let line = &lines[at];
            let word = line
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .find(|w| w.len() >= 3 && w.chars().all(|c| c.is_ascii_lowercase() || c == '_'))?;
            out[at] = line.replacen(word, &format!("{word}_x"), 1);
            LineEdit {
                first: at,
                removed: 1,
                inserted: 1,
            }
        }
        Kind::UnmatchedQuote => {
            out[at] = format!("\"{}", lines[at]);
            LineEdit {
                first: at,
                removed: 1,
                inserted: 1,
            }
        }
        Kind::Heredoc => {
            out.insert(at, "cat <<EOF\n".into());
            LineEdit {
                first: at,
                removed: 0,
                inserted: 1,
            }
        }
        Kind::Case => {
            out.insert(at, "case \"$x\" in\n".into());
            LineEdit {
                first: at,
                removed: 0,
                inserted: 1,
            }
        }
    };
    Some((out.concat(), edit))
}

/// The inverse of an edit made by [`edited`].
fn undo(edit: LineEdit) -> LineEdit {
    LineEdit {
        first: edit.first,
        removed: edit.inserted,
        inserted: edit.removed,
    }
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn edit_shell() {
    release_only();
    let name = "shell-5k.sh";
    let text = fixture(name);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let n = lines.len();
    let mut engine = Engine::new(light(), Options::default());
    open(&mut engine, name, &text, Mode::Editable);
    drain(&mut engine);
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let plan: Vec<Kind> = std::iter::repeat_n(Kind::Rename, 200)
        .chain(std::iter::repeat_n(Kind::UnmatchedQuote, 5))
        .chain(std::iter::repeat_n(Kind::Heredoc, 5))
        .chain(std::iter::repeat_n(Kind::Case, 5))
        .collect();
    let original: Arc<str> = text.as_str().into();
    // Palette entries arrive as deltas, so one palette follows the whole run.
    let mut palette = Vec::new();
    let keep = |out: Vec<(u64, Reply)>, palette: &mut Vec<_>| super::engine_lines(out, palette);
    let mut fore = Vec::new();
    let mut back: Vec<(Kind, f64)> = Vec::new();
    let mut seq = 2;
    let mut checked = 0;
    for (i, kind) in plan.into_iter().enumerate() {
        let (new, edit) = loop {
            let at = 25 + rng.below(n - 150);
            if let Some(e) = edited(&lines, at, kind) {
                break e;
            }
        };
        let at = edit.first;
        let visible = at - 20..at + VISIBLE - 20;
        seq += 1;
        let t = Instant::now();
        engine.edit(1, seq, seq, new.as_str().into(), edit, visible.clone());
        engine.step();
        let out = engine.take_out();
        fore.push(ms(t.elapsed()));
        keep(out, &mut palette);
        let t = Instant::now();
        let mut out = Vec::new();
        while engine.step() {
            out.extend(engine.take_out());
        }
        back.push((kind, ms(t.elapsed())));
        keep(out, &mut palette);
        if i % 20 == 0 {
            // Exact once the background pass has ended.
            let count = new.split_inclusive('\n').count() + 1;
            engine.show(1, seq, 0..count);
            engine.step();
            let got = keep(engine.take_out(), &mut palette);
            let want = reference(&new, syntax_for(name, &new), &light(), LONG_LINE);
            assert_eq!(got.len(), want.len());
            for (line, runs) in got {
                assert_eq!(runs, want[line], "edit {i} ({kind:?}): line {line}");
            }
            checked += 1;
        }
        seq += 1;
        engine.edit(1, seq, seq, original.clone(), undo(edit), visible);
        while engine.step() {
            keep(engine.take_out(), &mut palette);
        }
        keep(engine.take_out(), &mut palette);
    }
    let (p50, p95, max) = summary(fore);
    println!(
        "edit {name}: foreground p50 {p50:.3} ms, p95 {p95:.3} ms, max {max:.3} ms ({checked} checked exact)"
    );
    for kind in [
        Kind::Rename,
        Kind::UnmatchedQuote,
        Kind::Heredoc,
        Kind::Case,
    ] {
        let times: Vec<f64> = back
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, t)| *t)
            .collect();
        let (p50, p95, max) = summary(times);
        println!("  background after {kind:?}: p50 {p50:.3} ms, p95 {p95:.3} ms, max {max:.3} ms");
    }
    // An edit that lands while an unmatched quote's cascade runs in the background.
    let (quoted, e1) = edited(&lines, 100, Kind::UnmatchedQuote).expect("edit");
    engine.edit(1, 1000, 1000, quoted.as_str().into(), e1, 80..160);
    engine.step();
    engine.step();
    engine.take_out();
    let qlines: Vec<&str> = quoted.split_inclusive('\n').collect();
    let (renamed, e2) = (3000..)
        .find_map(|at| edited(&qlines, at, Kind::Rename))
        .expect("edit");
    let t = Instant::now();
    engine.edit(
        1,
        1001,
        1001,
        renamed.as_str().into(),
        e2,
        e2.first - 20..e2.first + 60,
    );
    engine.step();
    engine.take_out();
    println!(
        "  an edit during a background cascade: {:.3} ms foreground",
        ms(t.elapsed())
    );
    assert!(
        p95 <= 2.0,
        "edit foreground p95 {p95:.3} ms is over the 2 ms budget"
    );
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn theme_switch_markdown() {
    release_only();
    let name = "markdown-50k.md";
    let text = fixture(name);
    let mut engine = Engine::new(light(), Options::default());
    open(&mut engine, name, &text, Mode::Preview);
    drain(&mut engine);
    let t = Instant::now();
    engine.set_theme(super::dark());
    let out = engine.take_out();
    let colors = out
        .iter()
        .map(|(_, r)| match r {
            Reply::Palette { colors, .. } => colors.len(),
            _ => 0,
        })
        .sum::<usize>();
    println!(
        "theme switch {name}: {:.2} ms for {colors} classes, no re-parse",
        ms(t.elapsed())
    );
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn viewport_through_the_thread() {
    release_only();
    let name = "rust-50k.rs";
    let text = fixture(name);
    let h = super::super::Highlighter::new(light());
    let mut doc = h.open(Source::new(text.as_str()).file_name(name));
    assert!(doc.settle(Duration::from_secs(60)));
    let lines = doc.line_count();
    let mut rng = Rng(7);
    let mut times = Vec::new();
    for _ in 0..50 {
        let start = rng.below(lines - VISIBLE - OVERSCAN);
        let t = Instant::now();
        doc.show(start..start + VISIBLE);
        assert!(doc.settle(Duration::from_secs(10)));
        times.push(ms(t.elapsed()));
    }
    let (p50, p95, max) = summary(times);
    println!(
        "viewport {name} through the thread (request to reply): p50 {p50:.2} ms, p95 {p95:.2} ms, max {max:.2} ms"
    );
}

/// The UI thread's side of the block cache (review 5, SF5): one build asking `n` blocks, then
/// the trims at the next bumps once the host has moved to a small document. The blocks name a
/// language with no grammar, so the thread answers them at once and only the cache's own work
/// is timed.
fn block_cache_costs(n: usize, block: impl Fn(usize) -> String) {
    let mut pump = super::Pump::new(light());
    let mut blocks = pump.blocks();
    let codes: Vec<String> = (0..n).map(block).collect();
    let t = Instant::now();
    for c in &codes {
        blocks.highlight(Some("zzz"), c);
    }
    let ask = t.elapsed();
    pump.run();
    let t = Instant::now();
    blocks.poll();
    let first = t.elapsed();
    let mut trims = Vec::new();
    for k in 0..3 {
        blocks.highlight(Some("zzz"), &format!("g({k});\n"));
        pump.run();
        let t = Instant::now();
        blocks.poll();
        trims.push(ms(t.elapsed()));
    }
    println!(
        "block cache, {n} blocks: one build asks them in {:.1} ms; landing poll {:.1} ms; next trims {:?} ms",
        ms(ask),
        ms(first),
        trims.iter().map(|t| format!("{t:.1}")).collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn block_cache_trims_and_misses() {
    release_only();
    // The reviewer's cases: 15k blocks of 1 KiB (a document over the cap), then 80k tiny ones.
    block_cache_costs(15_000, |i| {
        format!("// {i}\n{}\n", "let x = 1;".repeat(100))
    });
    block_cache_costs(80_000, |i| format!("f({i});\n"));
}

/// A build of `new` blocks of 8 KiB while an older document of `old` blocks of 30 KiB is
/// still cached: together past the 16 MiB ceiling, so its misses evict on the way.
fn past_the_ceiling(old: usize, new: usize) {
    let mut pump = super::Pump::new(light());
    let mut blocks = pump.blocks();
    for i in 0..old {
        blocks.highlight(
            Some("zzz"),
            &format!("// old {i}\n{}\n", "x".repeat(30_000)),
        );
    }
    pump.run();
    blocks.poll();
    let codes: Vec<String> = (0..new)
        .map(|i| format!("// new {i}\n{}\n", "y".repeat(8_000)))
        .collect();
    let t = Instant::now();
    let refused = codes
        .iter()
        .filter(|c| blocks.highlight(Some("zzz"), c).is_some())
        .count();
    println!(
        "block cache past the ceiling: {old} old blocks of 30 KiB cached, a build of {new} new \
         blocks of 8 KiB asks them in {:.1} ms, {refused} refused, {} bytes cached",
        ms(t.elapsed()),
        blocks.cached_bytes()
    );
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn block_cache_misses_past_the_ceiling() {
    release_only();
    past_the_ceiling(500, 1_500);
    past_the_ceiling(500, 15_000);
}

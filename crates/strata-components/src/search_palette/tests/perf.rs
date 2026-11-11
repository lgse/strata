//! Perf: the palette at 100 (Strata's cap), 1,000 and 10,000 results.
//!
//! ```text
//! cargo test --profile perf -p strata-components --lib search_palette::tests::perf \
//!     -- --ignored --nocapture
//! ```
//!
//! A 960 × 640 window at scale 2 with the palette open on a typed query. For each size: the
//! frame that lands the first answer (with the palette taking it), a stream of ten batches growing to the full list (each
//! a frame that lands one), 60 arrow steps, and 30 keystrokes (each asking for a new query
//! while the old results stay). The list is built by index (`virtual_rows`), so only the
//! mounted rows build. Each figure is the best of three runs.
//!
//! The bars, held at 1,000 and 10,000: an arrow step within 4 ms at p95; a delivery, a batch
//! and a keystroke within 8 ms at p95.

use std::time::Instant;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime};
use strata_kit::look::Look;

use super::super::*;

const SIZES: [usize; 3] = [100, 1_000, 10_000];
const RUNS: usize = 3;
const BATCHES: usize = 10;
const FRAME: f64 = 1.0 / 60.0;

struct Host {
    look: Look,
    palette: PaletteState,
}

fn view(h: &Host, _cx: &mut Cx<Host>) -> El {
    let mut layer = Vec::new();
    SearchPalette::new("p", |h: &Host| &h.palette, |h: &mut Host| &mut h.palette)
        .build(&h.look, h, &mut layer);
    rune_kit::root(rune::col().w_full().h_full(), layer)
}

fn results(n: usize) -> Vec<PaletteItem> {
    (0..n)
        .map(|i| {
            let name = format!("report-{i:05}.pdf");
            let path = format!("/home/demo/Documents/Reports/{name}");
            PaletteItem::new(path.clone(), name, path)
                .matched(std::iter::once(0..3))
                .detail_matched(std::iter::once(29..32))
        })
        .collect()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

#[derive(Clone, Copy, Debug)]
struct Figures {
    deliver: f64,
    batch_p95: f64,
    step_p50: f64,
    step_p95: f64,
    key_p50: f64,
    key_p95: f64,
}

fn key(rt: &mut Runtime<Host>, key: Key) -> f64 {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    let t = Instant::now();
    rt.frame(FRAME);
    ms(t)
}

fn measure(n: usize) -> Figures {
    let all = results(n);
    let host = Host {
        look: Look::fallback(false),
        palette: PaletteState::new(),
    };
    let mut rt = Runtime::new(host, view, (960.0, 640.0), Env::new(false, 2.0));
    rt.frame(0.0);
    rt.update(|h, _| {
        let _ = h.palette.open(PaletteMode::search(&["/home/demo"]));
        let _ = h.palette.set_query("rep");
    });
    for _ in 0..20 {
        rt.frame(FRAME);
    }
    let generation = rt.state.palette.generation();

    // The first answer, then the stream growing to the whole list.
    let first = all[..n / BATCHES].to_vec();
    let t = Instant::now();
    rt.update(|h, tx| {
        let _ = h.palette.deliver(Answer::new(generation, first), tx);
    });
    rt.frame(FRAME);
    let deliver = ms(t);
    let mut batches = Vec::new();
    for k in 2..=BATCHES {
        let mut answer = Answer::new(generation, all[..n * k / BATCHES].to_vec());
        if k == BATCHES {
            answer = answer.done();
        }
        let t = Instant::now();
        rt.update(|h, tx| {
            let _ = h.palette.deliver(answer, tx);
        });
        rt.frame(FRAME);
        batches.push(ms(t));
    }
    assert_eq!(rt.state.palette.items().len(), n);
    for _ in 0..10 {
        rt.frame(FRAME);
    }

    let mut steps: Vec<f64> = (0..60).map(|_| key(&mut rt, Key::Down)).collect();
    assert_eq!(rt.state.palette.highlighted(), Some(60.min(n - 1)));
    let mut keys = Vec::new();
    for i in 0..30 {
        keys.push(key(
            &mut rt,
            Key::Text(if i % 2 == 0 { "x" } else { "y" }.into()),
        ));
        assert_eq!(rt.state.palette.items().len(), n, "the old results stay");
    }
    Figures {
        deliver,
        batch_p95: percentile(&mut batches, 0.95),
        step_p50: percentile(&mut steps.clone(), 0.5),
        step_p95: percentile(&mut steps, 0.95),
        key_p50: percentile(&mut keys.clone(), 0.5),
        key_p95: percentile(&mut keys, 0.95),
    }
}

#[test]
#[ignore = "perf: run with --profile perf -- --ignored"]
fn the_palette_stays_within_its_budget() {
    for n in SIZES {
        let runs: Vec<Figures> = (0..RUNS).map(|_| measure(n)).collect();
        let best = |f: fn(&Figures) -> f64| runs.iter().map(f).fold(f64::INFINITY, f64::min);
        let fig = Figures {
            deliver: best(|r| r.deliver),
            batch_p95: best(|r| r.batch_p95),
            step_p50: best(|r| r.step_p50),
            step_p95: best(|r| r.step_p95),
            key_p50: best(|r| r.key_p50),
            key_p95: best(|r| r.key_p95),
        };
        println!(
            "{n:>6} results: delivery {:.2} ms; batch p95 {:.2} ms; arrow p50 {:.2} p95 {:.2} ms; \
             keystroke p50 {:.2} p95 {:.2} ms",
            fig.deliver, fig.batch_p95, fig.step_p50, fig.step_p95, fig.key_p50, fig.key_p95
        );
        if n >= 1_000 {
            assert!(fig.step_p95 <= 4.0, "{n}: arrow p95 {:.2} ms", fig.step_p95);
            assert!(fig.deliver <= 8.0, "{n}: delivery {:.2} ms", fig.deliver);
            assert!(
                fig.batch_p95 <= 8.0,
                "{n}: batch p95 {:.2} ms",
                fig.batch_p95
            );
            assert!(
                fig.key_p95 <= 8.0,
                "{n}: keystroke p95 {:.2} ms",
                fig.key_p95
            );
        }
    }
}

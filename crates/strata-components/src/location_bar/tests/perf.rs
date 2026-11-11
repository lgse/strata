//! Perf: the suggestion list at 50 (Strata's cap), 1,000 and 10,000 candidates.
//!
//! ```text
//! cargo test --profile perf -p strata-components --lib location_bar::tests::perf \
//!     -- --ignored --nocapture
//! ```
//!
//! A 900 pt window at scale 2 with the bar in its header and the entry open. For each size: the
//! frame that lands a delivery (the list's first draw), 60 arrow steps (each a commit moving the
//! highlight), and 30 keystrokes with the list open (each a commit that asks for a new listing
//! while the old list stays). Rune builds every row's element on each commit (its README:
//! "View construction still visits every row"), so the cost grows with the list; only the
//! mounted rows lay out and shape text. Tab's common prefix over the list is timed apart. Each
//! figure is the best of three runs.
//!
//! The bar at 1,000 candidates: an arrow step commits within 4 ms at p95; a delivery and a
//! keystroke within 8 ms at p95; the common prefix within 1 ms. 10,000 is reported, not held.

use std::time::Instant;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, Tx, col, row};
use strata_kit::look::Look;

use super::super::completion::longest_common_prefix;
use super::super::*;

const SIZES: [usize; 3] = [50, 1_000, 10_000];
const RUNS: usize = 3;
const FRAME: f64 = 1.0 / 60.0;

struct Host {
    look: Look,
    bar: LocationBarState,
    trail: Trail,
    n: usize,
    last: Option<CompletionQuery>,
}

fn candidates(n: usize, text: &str) -> Vec<Candidate> {
    (0..n)
        .map(|i| {
            let name = format!("folder-{i:05}");
            Candidate::new(format!("{name}/"), format!("{text}{name}/"))
                .matched(std::iter::once(0..3))
                .hint("~/somewhere")
        })
        .collect()
}

fn view(h: &Host, cx: &mut Cx<Host>) -> El {
    let mut layer = Vec::new();
    let bar = LocationBar::new(
        "loc",
        &h.look,
        &h.trail,
        &h.bar,
        |h: &mut Host| &mut h.bar,
        |h: &mut Host, i, _: &mut Tx| {
            if let LocationIntent::Complete(q) = i {
                h.last = Some(q);
            }
        },
    )
    .build(cx, &mut layer);
    rune_kit::root(
        col()
            .w_full()
            .h_full()
            .child(row().w_full().h(40.0).child(bar)),
        layer,
    )
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
    step_p50: f64,
    step_p95: f64,
    key_p50: f64,
    key_p95: f64,
    prefix: f64,
}

fn deliver(rt: &mut Runtime<Host>) {
    let q = rt.state.last.take().expect("a query");
    let list = candidates(rt.state.n, &q.text);
    rt.update(|h, _| {
        let _ = h.bar.deliver(Completions::new(q.generation, list));
    });
}

fn measure(n: usize) -> Figures {
    let host = Host {
        look: Look::fallback(false),
        bar: LocationBarState::new(),
        trail: Trail::new(vec![Crumb::new("/home/demo", "~")], "~"),
        n,
        last: None,
    };
    let mut rt = Runtime::new(host, view, (900.0, 600.0), Env::new(false, 2.0));
    rt.frame(0.0);
    rt.update(|h, _| {
        let polled = h.bar.begin_edit("~/");
        for i in polled.intents {
            if let LocationIntent::Complete(q) = i {
                h.last = Some(q);
            }
        }
    });
    for _ in 0..10 {
        rt.frame(FRAME);
    }
    deliver(&mut rt);
    let t = Instant::now();
    rt.frame(FRAME);
    let deliver_ms = ms(t);
    for _ in 0..10 {
        rt.frame(FRAME);
    }
    assert!(rt.state.bar.suggestions_open());

    let mut steps = Vec::new();
    for _ in 0..60 {
        rt.key(KeyEvent {
            key: Key::Down,
            mods: Mods::default(),
        });
        let t = Instant::now();
        rt.frame(FRAME);
        steps.push(ms(t));
    }
    let mut keys = Vec::new();
    for i in 0..30 {
        rt.key(KeyEvent {
            key: Key::Text(if i % 2 == 0 { "x" } else { "y" }.into()),
            mods: Mods::default(),
        });
        let t = Instant::now();
        rt.frame(FRAME);
        keys.push(ms(t));
        assert!(rt.state.bar.suggestions_open(), "the old list stays");
    }
    let list = candidates(n, "~/");
    let t = Instant::now();
    let common = longest_common_prefix(list.iter().map(|c| c.replacement.as_str()));
    let prefix = ms(t);
    assert!(common.is_some_and(|c| c.starts_with("~/folder-")));
    Figures {
        deliver: deliver_ms,
        step_p50: percentile(&mut steps.clone(), 0.5),
        step_p95: percentile(&mut steps, 0.95),
        key_p50: percentile(&mut keys.clone(), 0.5),
        key_p95: percentile(&mut keys, 0.95),
        prefix,
    }
}

#[test]
#[ignore = "perf: run with --profile perf -- --ignored"]
fn the_suggestion_list_stays_within_its_budget() {
    for n in SIZES {
        let runs: Vec<Figures> = (0..RUNS).map(|_| measure(n)).collect();
        let best = |f: fn(&Figures) -> f64| runs.iter().map(f).fold(f64::INFINITY, f64::min);
        let fig = Figures {
            deliver: best(|r| r.deliver),
            step_p50: best(|r| r.step_p50),
            step_p95: best(|r| r.step_p95),
            key_p50: best(|r| r.key_p50),
            key_p95: best(|r| r.key_p95),
            prefix: best(|r| r.prefix),
        };
        println!(
            "{n:>6} candidates: delivery {:.2} ms; arrow p50 {:.2} p95 {:.2} ms; \
             keystroke p50 {:.2} p95 {:.2} ms; common prefix {:.3} ms",
            fig.deliver, fig.step_p50, fig.step_p95, fig.key_p50, fig.key_p95, fig.prefix
        );
        if n == 1_000 {
            assert!(fig.step_p95 <= 4.0, "arrow p95 {:.2} ms", fig.step_p95);
            assert!(fig.deliver <= 8.0, "delivery {:.2} ms", fig.deliver);
            assert!(fig.key_p95 <= 8.0, "keystroke p95 {:.2} ms", fig.key_p95);
            assert!(fig.prefix <= 1.0, "common prefix {:.3} ms", fig.prefix);
        }
    }
}

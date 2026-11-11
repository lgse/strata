//! Perf: 100,000 entries on C1e's element, with and without thumbnails landing.
//!
//! ```text
//! cargo test --profile perf -p strata-components --lib thumbnails::tests::perf \
//!     -- --ignored --nocapture
//! ```
//!
//! A 900 pt window at scale 2, as C1e's perf test: the first frame, then 600 frames at
//! 120 Hz, each scrolled 48 pt by the wheel. Three runs side by side: C1e's default icon, the
//! thumbnail slot with nothing landing, and the slot with a worker landing up to 8 images a
//! frame at the slot's step (64 px for 32 pt at scale 2). Each figure is the best of three.
//! Then the store alone: `sync` for a 200-tile grid viewport stepping a line at a time, and
//! `pop` plus `complete` for each request it makes.
//!
//! The bar (work package C25, plan §4 C25):
//! - no view run while scrolling with thumbnails landing;
//! - scroll frames at p50 and p95 within max(1.25 × the default icon's, + 0.5 ms), and none
//!   over a 60 Hz frame (16.7 ms);
//! - `sync` within 50 µs at p95, `pop` plus `complete` within 20 µs on average;
//! - with 3,600 tiles on screen (a 4K grid), draining the queue is one pass (within 20 ms) and
//!   an idle `pop` within 1 µs (174 µs before review round 1).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use rune::{Cx, El, Env, Runtime, col};
use strata_kit::look::Look;

use super::image;
use crate::collection::element::{Collection, CollectionState};
use crate::contract::{EntryId, EntryKind, ListEntry, ListSource, ThumbKey};
use crate::thumbnails::{Step, ThumbnailStore, ViewerId, Viewport, icon_slot};

const N: usize = 100_000;
const W: f32 = 900.0;
const H: f32 = 900.0;
const PT: f32 = 32.0;
const SCALE: f32 = 2.0;
const DT: f64 = 1.0 / 120.0;
const PER_FRAME: usize = 8;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    DefaultIcon,
    Slot,
    Landing,
}

struct Host {
    src: ListSource,
    coll: CollectionState,
    look: Look,
    store: ThumbnailStore,
    viewer: ViewerId,
    mode: Mode,
    views: Rc<Cell<usize>>,
}

fn listing() -> ListSource {
    ListSource::new(
        (0..N)
            .map(|i| {
                let mut e = ListEntry::new(
                    EntryId::new(i as u64 + 1),
                    format!("IMG_{i:06}.jpg"),
                    EntryKind::File,
                    "file_image",
                );
                e.thumbnail = Some(ThumbKey::new(i as u64 + 1));
                e
            })
            .collect(),
    )
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    s.views.set(s.views.get() + 1);
    let step = Step::for_tile(&s.look, PT, SCALE);
    let mut c = Collection::new(
        "list",
        |s: &Host| (&s.src, &s.coll),
        |s: &mut Host| (&s.src, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
    .row_height(|_, _| 40.0);
    if s.mode != Mode::DefaultIcon {
        c = c
            .icon(icon_slot(|s: &Host| &s.store, step, PT))
            .on_visible(move |s: &mut Host, r| {
                s.store
                    .sync(s.viewer, &s.src, Viewport::new(r, step).listing(1))
            });
    }
    col().size(W, H).child(c.build(s))
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// (p50, p95, max) of the scroll frames in ms, and the view runs while scrolling.
fn run(src: &ListSource, mode: Mode) -> (f64, f64, f64, usize) {
    let mut store = ThumbnailStore::default();
    let viewer = store.viewer();
    let host = Host {
        src: src.clone(),
        coll: CollectionState::new(),
        look: Look::fallback(false),
        store,
        viewer,
        mode,
        views: Rc::default(),
    };
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, SCALE));
    rt.frame(0.0);
    for _ in 0..10 {
        rt.frame(DT);
    }
    let views = rt.state.views.get();
    let mut times = Vec::with_capacity(600);
    for _ in 0..600 {
        rt.wheel(450.0, 450.0, -48.0);
        let t = Instant::now();
        if mode == Mode::Landing {
            let s = &mut rt.state;
            for _ in 0..PER_FRAME {
                let Some(r) = s.store.pop(0.0) else { break };
                s.store.complete(r.ticket, image(r.step), 0.0);
            }
        }
        rt.frame(DT);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let max = times.iter().copied().fold(0.0, f64::max);
    (
        pct(&mut times, 0.5),
        pct(&mut times, 0.95),
        max,
        rt.state.views.get() - views,
    )
}

fn best(src: &ListSource, mode: Mode) -> (f64, f64, f64, usize) {
    let runs: Vec<_> = (0..3).map(|_| run(src, mode)).collect();
    let pick = |f: fn(&(f64, f64, f64, usize)) -> f64| runs.iter().map(f).fold(f64::MAX, f64::min);
    (
        pick(|r| r.0),
        pick(|r| r.1),
        pick(|r| r.2),
        runs.iter().map(|r| r.3).max().unwrap_or(0),
    )
}

#[test]
#[ignore = "perf: run with --profile perf -- --ignored --nocapture"]
fn thumbnails_at_100k() {
    let src = listing();
    let base = best(&src, Mode::DefaultIcon);
    let slot = best(&src, Mode::Slot);
    let landing = best(&src, Mode::Landing);
    for (name, r) in [
        ("default icon", base),
        ("slot, empty", slot),
        ("slot, landing", landing),
    ] {
        println!(
            "{name:>14}: p50 {:.3} ms, p95 {:.3} ms, max {:.2} ms, view runs {}",
            r.0, r.1, r.2, r.3
        );
    }
    assert_eq!(landing.3, 0, "no view run while thumbnails land");
    for r in [slot, landing] {
        assert!(
            r.0 <= (base.0 * 1.25).max(base.0 + 0.5),
            "p50 {:.3} vs {:.3}",
            r.0,
            base.0
        );
        assert!(
            r.1 <= (base.1 * 1.25).max(base.1 + 0.5),
            "p95 {:.3} vs {:.3}",
            r.1,
            base.1
        );
        assert!(r.2 < 16.7, "a frame over 16.7 ms: {:.2}", r.2);
    }

    // The store alone: a grid of 10 a line, 20 lines on screen, stepping a line a sync.
    let mut store = ThumbnailStore::default();
    let v = store.viewer();
    let step = Step::new(128);
    let mut syncs = Vec::new();
    let mut served = 0usize;
    let mut serve_time = 0.0;
    for line in 0..2000 {
        let start = line * 10;
        let t = Instant::now();
        store.sync(
            v,
            &src,
            Viewport::new(start..start + 200, step).per_line(10),
        );
        syncs.push(t.elapsed().as_secs_f64() * 1e6);
        // The images are made outside the timing: they stand for the worker's output.
        let t = Instant::now();
        let requests: Vec<_> = std::iter::from_fn(|| store.pop(0.0)).collect();
        serve_time += t.elapsed().as_secs_f64() * 1e6;
        let results: Vec<_> = requests.iter().map(|r| (r.ticket, image(r.step))).collect();
        let t = Instant::now();
        for (ticket, outcome) in results {
            store.complete(ticket, outcome, 0.0);
            served += 1;
        }
        serve_time += t.elapsed().as_secs_f64() * 1e6;
    }
    let per = serve_time / served.max(1) as f64;
    let p95 = pct(&mut syncs, 0.95);
    println!(
        "store: sync p50 {:.1} µs, p95 {:.1} µs; pop + complete {per:.2} µs each \
         ({served} served), {} bytes held",
        pct(&mut syncs, 0.5),
        p95,
        store.stats().bytes
    );
    assert!(p95 <= 50.0, "sync p95 {p95:.1} µs");
    assert!(per <= 20.0, "pop + complete {per:.2} µs");

    // A 4K grid: 80 tiles a line, 45 lines on screen, small images so all of it fits.
    let mut store = ThumbnailStore::new(crate::thumbnails::Settings::default().budget(1 << 30));
    let v = store.viewer();
    store.sync(v, &src, Viewport::new(0..3600, step).per_line(80));
    let small = || crate::thumbnails::Outcome::Ready {
        width: 4,
        height: 4,
        rgba: vec![0; 64],
    };
    let t = Instant::now();
    let mut drained = 0;
    while let Some(r) = store.pop(0.0) {
        store.complete(r.ticket, small(), 0.0);
        drained += 1;
    }
    let drain_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    for _ in 0..1000 {
        assert!(store.pop(0.0).is_none());
    }
    let idle = t.elapsed().as_secs_f64() * 1e6 / 1000.0;
    println!("4K grid: drained {drained} requests in {drain_ms:.2} ms; an idle pop {idle:.3} µs");
    assert!(drain_ms <= 20.0, "drain {drain_ms:.2} ms");
    assert!(idle <= 1.0, "idle pop {idle:.3} µs");
}

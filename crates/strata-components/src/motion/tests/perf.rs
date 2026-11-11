//! Frame costs while the effects play, against a frame at rest (ignored; run in release:
//! `cargo test -p strata-components --release --lib motion::tests::perf -- --ignored --nocapture`).

use std::time::Instant;

use super::*;

fn p95(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[(v.len() * 95 / 100).min(v.len() - 1)]
}

fn time(rt: &mut Runtime<Host>, frames: usize) -> Vec<f64> {
    (0..frames)
        .map(|_| {
            let t = Instant::now();
            rt.frame(DT);
            t.elapsed().as_secs_f64() * 1e3
        })
        .collect()
}

/// A dissolve of 15 rows, at Strata's full budget of 192 fragments.
fn dissolve_run(ghost: bool) {
    let mut host = Host::new(15);
    host.ghost = ghost;
    let mut rt = runtime(host);
    for _ in 0..30 {
        rt.frame(DT);
    }
    let rest = p95(time(&mut rt, 60));
    let rows: Vec<u64> = (0..15).collect();
    dissolve(&mut rt, &rows).unwrap();
    let pieces = rt
        .state
        .fx
        .dissolve
        .as_ref()
        .unwrap()
        .plan
        .borrow()
        .as_ref()
        .unwrap()
        .pieces
        .len();
    rt.frame(DT);
    let instances = rt.instances.len();
    let start = time(&mut rt, 1)[0];
    let playing = time(&mut rt, 30);
    let label = if ghost {
        "A ghost pieces"
    } else {
        "A' canvas shards"
    };
    println!(
        "{label}: {pieces} fragments, {instances} instances; rest p95 {rest:.3} ms; \
         playing p95 {:.3} ms, max {:.3} ms; mount frame {start:.3} ms",
        p95(playing.clone()),
        playing.iter().copied().fold(0.0, f64::max),
    );
}

#[test]
#[ignore = "perf: run in release with --ignored"]
fn perf_dissolve_ghost_pieces() {
    dissolve_run(true);
}

#[test]
#[ignore = "perf: run in release with --ignored"]
fn perf_dissolve_canvas_shards() {
    dissolve_run(false);
}

#[test]
#[ignore = "perf: run in release with --ignored"]
fn perf_flight_of_seven() {
    let mut rt = runtime(Host::new(15));
    for _ in 0..30 {
        rt.frame(DT);
    }
    let rest = p95(time(&mut rt, 60));
    let rows: Vec<u64> = (0..15).collect();
    trash(&mut rt, &rows).unwrap();
    let flying = time(&mut rt, 30);
    println!(
        "flight of 7: rest p95 {rest:.3} ms; flying p95 {:.3} ms, max {:.3} ms",
        p95(flying.clone()),
        flying.iter().copied().fold(0.0, f64::max),
    );
}

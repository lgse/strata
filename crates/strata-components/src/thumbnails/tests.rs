//! The store and its queue: eviction order, the budget, what is protected, request order
//! after scrolling each way, cancellation, failures, stops, the on/off switch, relocation,
//! steps and pruning; and in a headless runtime, that thumbnails land with no view run.

mod perf;
mod runtime;

use super::*;
use crate::contract::{EntryId, EntryKind, ListEntry, ListSource, ThumbKey};

const S4: Step = Step::new(4);
const S8: Step = Step::new(8);

/// `n` image entries keyed `1000 + i`, every entry index in `folders` a folder with no key.
fn source(n: usize, folders: &[usize]) -> ListSource {
    ListSource::new(
        (0..n)
            .map(|i| {
                let id = EntryId::new(i as u64 + 1);
                if folders.contains(&i) {
                    ListEntry::new(id, format!("folder {i}"), EntryKind::Folder, "folder")
                } else {
                    let mut e =
                        ListEntry::new(id, format!("img {i}"), EntryKind::File, "file_image");
                    e.thumbnail = Some(key(i));
                    e
                }
            })
            .collect(),
    )
}

fn key(i: usize) -> ThumbKey {
    ThumbKey::new(1000 + i as u64)
}

fn index(k: ThumbKey) -> usize {
    (k.get() - 1000) as usize
}

fn store(budget: usize) -> ThumbnailStore {
    ThumbnailStore::new(Settings {
        budget_bytes: budget,
        ..Settings::default()
    })
}

fn image(step: Step) -> Outcome {
    let px = step.px();
    Outcome::Ready {
        width: px,
        height: px,
        rgba: vec![255; (px * px * 4) as usize],
    }
}

/// Pops every request, in order, completing each with an image at its step. A queue that
/// doesn't drain within 10,000 requests fails the test.
fn serve(st: &mut ThumbnailStore, now: f64) -> Vec<usize> {
    let mut order = Vec::new();
    while let Some(r) = st.pop(now) {
        assert!(order.len() < 10_000, "the queue never drains");
        order.push(index(r.key));
        st.complete(r.ticket, image(r.step), now);
    }
    order
}

/// Pops every request without completing any.
fn drain(st: &mut ThumbnailStore, now: f64) -> Vec<Request> {
    std::iter::from_fn(|| st.pop(now)).collect()
}

fn ready(st: &ThumbnailStore, i: usize) -> bool {
    matches!(st.state(key(i)), State::Ready { .. })
}

fn view(r: std::ops::Range<usize>) -> Viewport {
    Viewport::new(r, S4)
}

#[test]
fn eviction_takes_the_least_recently_shown_first() {
    // Room for four 4 px images; no margin, so only what is shown loads.
    let mut st = ThumbnailStore::new(Settings {
        budget_bytes: 4 * 64,
        margin: 0.0,
        ..Settings::default()
    });
    let src = source(20, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..2));
    serve(&mut st, 0.0);
    st.sync(v, &src, view(2..4));
    serve(&mut st, 0.0);
    // 0..4 held. Building row 0 again (drawn) makes it the most recent.
    let _ = st.thumb(key(0));
    st.sync(v, &src, view(4..5));
    st.sync(v, &src, view(5..6));
    serve(&mut st, 0.0);
    // Protected: 4 (previous sync) and 5. One had to go: 1, the least recently shown.
    assert!(!ready(&st, 1), "1 evicted first");
    assert!(ready(&st, 0) && ready(&st, 2) && ready(&st, 3));
    st.sync(v, &src, view(6..7));
    serve(&mut st, 0.0);
    assert!(!ready(&st, 2), "then 2");
    assert!(ready(&st, 0), "row 0 was drawn after 2 and 3");
}

#[test]
fn an_evicted_image_empties_its_handle_in_place_and_shows_the_icon() {
    let mut st = ThumbnailStore::new(Settings {
        budget_bytes: 64,
        margin: 0.0,
        ..Settings::default()
    });
    let src = source(4, &[]);
    let v = st.viewer();
    let before = st.thumb(key(0));
    st.sync(v, &src, view(0..1));
    serve(&mut st, 0.0);
    assert_eq!(before.pixels().size(), (4, 4));
    assert_eq!(
        before.icon_alpha().get(),
        0.0,
        "the icon fades under a held image"
    );
    st.sync(v, &src, view(1..2));
    st.sync(v, &src, view(2..3));
    serve(&mut st, 0.0);
    let after = st.thumb(key(0));
    assert_eq!(after.epoch(), before.epoch(), "the same handles");
    assert_eq!(before.pixels().size(), (0, 0), "emptied in place");
    assert_eq!(before.icon_alpha().get(), 1.0);
}

/// Checks the store's own bookkeeping against a count from scratch.
fn check_counts(st: &ThumbnailStore, op: usize) {
    let inner = st.inner.borrow();
    let held = |k: &ThumbKey| {
        inner
            .entries
            .get(k)
            .and_then(|e| e.held)
            .map_or(0, |h| h.bytes())
    };
    let sum: usize = inner
        .entries
        .values()
        .filter_map(|e| e.held)
        .map(|h| h.bytes())
        .sum();
    assert_eq!(sum, inner.bytes, "op {op}: bytes add up");
    assert_eq!(
        inner.lru.len(),
        inner.entries.values().filter(|e| e.held.is_some()).count()
    );
    let mut pinned: std::collections::HashSet<ThumbKey> = inner.wanted.keys().copied().collect();
    pinned.extend(inner.protected.iter().copied());
    assert_eq!(
        pinned.iter().map(held).sum::<usize>(),
        inner.pinned,
        "op {op}: pinned bytes"
    );
    let out: usize = inner.margin_out.values().sum();
    assert_eq!(out, inner.margin_out_bytes, "op {op}: margin bytes out");
    for k in inner.margin_out.keys() {
        assert!(
            inner.in_flight.contains(k),
            "op {op}: margin room for no request"
        );
    }
    for k in &inner.in_flight {
        assert!(
            matches!(inner.entries[k].job, store::Job::Requested { .. }),
            "op {op}: in flight with no request"
        );
    }
    let protected = inner.protected_bytes();
    assert!(
        inner.bytes <= inner.settings.budget_bytes || inner.bytes == protected,
        "op {op}: {} bytes over a {} budget with {protected} protected",
        inner.bytes,
        inner.settings.budget_bytes
    );
}

/// A fixed pseudo-random run of `ops` operations from `seed`: syncs on two viewers, pops,
/// results of every kind, new steps, relocations, toggles, requeues and new budgets. After
/// each, the bookkeeping matches a count from scratch, the bytes stay in budget unless the
/// protected images alone are over it, and no image protected before the operation and still
/// is was evicted by it. Every 50 operations the view stands still: everything out comes back,
/// the queue drains in a bounded number of requests, and every wanted key is held at the step
/// it is wanted at.
fn fuzz(seed: u64, ops: usize) {
    let mut rng = seed;
    let mut next = |m: u64| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng % m
    };
    let src = source(400, &[3, 50, 51]);
    let mut st = store(40 * 64);
    let viewers = [st.viewer(), st.viewer()];
    let mut out: Vec<Ticket> = Vec::new();
    let mut now = 0.0;
    for op in 0..ops {
        now += 0.05;
        let protected_before: Vec<ThumbKey> = {
            let inner = st.inner.borrow();
            inner
                .protected
                .iter()
                .copied()
                .filter(|k| ready(&st, index(*k)))
                .collect()
        };
        let mut relocated = false;
        match next(10) {
            0..=2 => {
                let v = viewers[next(2) as usize];
                let start = next(380) as usize;
                let len = 1 + next(30) as usize;
                let s = [S4, S4, S8, S16][next(4) as usize];
                let view = Viewport::new(start..start + len, s).per_line(1 + next(3) as usize);
                st.sync(v, &src, view);
            }
            3..=4 => out.extend(st.pop(now).map(|r| r.ticket)),
            5..=6 if !out.is_empty() => {
                let t = out.swap_remove(next(out.len() as u64) as usize);
                let outcome = match next(8) {
                    0 => Outcome::Failed,
                    1 => Outcome::Stopped,
                    2 => Outcome::Ready {
                        width: 3,
                        height: 3,
                        rgba: vec![0; 5],
                    },
                    3 => {
                        // Smaller than the step, and not square.
                        let w = (t.step().px() / 2).max(1);
                        Outcome::Ready {
                            width: w,
                            height: 1,
                            rgba: vec![0; w as usize * 4],
                        }
                    }
                    _ => image(t.step()),
                };
                st.complete(t, outcome, now);
            }
            7 => {
                let _ = st.thumb(key(next(400) as usize));
            }
            8 => match next(6) {
                0 => {
                    st.set_enabled(next(2) == 0);
                }
                1 => {
                    // Replaces what the target held by design: not an eviction. Targets include
                    // keys no viewer shows (400..500).
                    relocated = st.relocate(key(next(400) as usize), key(next(500) as usize));
                }
                2 => st.requeue_stopped(),
                3 => {
                    let v = viewers[next(2) as usize];
                    st.set_step(v, &src, [S4, S8, S16][next(3) as usize]);
                }
                4 => {
                    let margin = [0.0, 0.25, 0.5, 1.0][next(4) as usize];
                    let settings = st.settings().margin(margin);
                    st.set_settings(settings);
                }
                _ => {
                    let budget = (8 + next(200) as usize) * 64;
                    let settings = st.settings().budget(budget);
                    st.set_settings(settings);
                }
            },
            _ => {
                let _ = st.take_cancelled();
            }
        }
        check_counts(&st, op);
        if !relocated {
            for k in protected_before {
                let still = st.inner.borrow().protected.contains(&k);
                let there = st.inner.borrow().entries.contains_key(&k);
                if still && there {
                    assert!(ready(&st, index(k)), "op {op}: protected {k:?} evicted");
                }
            }
        }
        if op % 50 == 49 {
            // Still, at the real budget and the same time: a poll takes work before the old
            // results come back, then the queue drains. Ranking again from scratch (and with
            // the budget's deferrals forgotten) must then find nothing that would land: a key
            // that would is one the still view never asks for.
            st.set_enabled(true);
            out.extend(std::iter::from_fn(|| st.pop(now)).map(|r| r.ticket));
            for t in out.drain(..) {
                st.complete(t, image(t.step()), now);
            }
            let mut requests = 0;
            while let Some(r) = st.pop(now) {
                requests += 1;
                assert!(
                    requests <= 10_000,
                    "seed {seed:x} op {op}: the queue never drains"
                );
                st.complete(r.ticket, image(r.step), now);
            }
            for forget_deferrals in [false, true] {
                {
                    let mut inner = st.inner.borrow_mut();
                    inner.dirty = true;
                    if forget_deferrals {
                        inner.deferred.clear();
                    }
                }
                let mut stalled = Vec::new();
                while let Some(r) = st.pop(now) {
                    let landed = st.complete(r.ticket, image(r.step), now);
                    if landed != Landed::Dropped(Why::Budget) {
                        stalled.push((index(r.key), r.step, r.tier, landed));
                    }
                    assert!(stalled.len() < 50);
                }
                assert!(
                    stalled.is_empty(),
                    "seed {seed:x} op {op}: a still view never asks for {stalled:?} \
                     (deferrals forgotten: {forget_deferrals})"
                );
            }
            check_counts(&st, op);
            // Then thumbnails on, stops lifted, room for everything, past any wait.
            st.set_enabled(true);
            st.requeue_stopped();
            let budget = st.settings().budget_bytes;
            let roomy = st.settings().budget(1 << 30);
            st.set_settings(roomy);
            out.extend(std::iter::from_fn(|| st.pop(now)).map(|r| r.ticket));
            for t in out.drain(..) {
                st.complete(t, image(t.step()), now);
            }
            now += 31.0;
            let wanted = st.inner.borrow().wanted.len();
            let mut requests = 0;
            while let Some(r) = st.pop(now) {
                requests += 1;
                assert!(
                    requests <= 2 * wanted + 1,
                    "op {op}: the queue never drains ({wanted} wanted)"
                );
                st.complete(r.ticket, image(r.step), now);
            }
            check_counts(&st, op);
            let inner = st.inner.borrow();
            let stuck: Vec<String> = inner
                .wanted
                .iter()
                .filter(|(k, want)| !inner.entries[k].held.is_some_and(|h| h.step == **want))
                .map(|(k, want)| {
                    let e = &inner.entries[k];
                    format!("{k:?} wanted at {want:?}: held {:?}, {:?}", e.held, e.job)
                })
                .collect();
            assert!(
                stuck.is_empty(),
                "seed {seed:x} op {op}: stuck keys {stuck:?}"
            );
            drop(inner);
            let tight = st.settings().budget(budget);
            st.set_settings(tight);
        }
    }
}

#[test]
fn the_budget_holds_and_a_still_view_loads_every_wanted_key() {
    for seed in [
        0x9e37_79b9_7f4a_7c15_u64,
        0x1234_5678_9abc_def1,
        0xdead_beef_cafe_f00d,
    ] {
        fuzz(seed, 6000);
    }
}

#[test]
fn rows_on_screen_are_never_evicted_and_win_over_the_budget() {
    // Room for two images, ten on screen: all ten load and stay.
    let mut st = store(2 * 64);
    let src = source(40, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..10));
    let order = serve(&mut st, 0.0);
    assert_eq!(
        order,
        (0..10).collect::<Vec<_>>(),
        "no margin while the screen fills the budget"
    );
    assert!((0..10).all(|i| ready(&st, i)));
    assert_eq!(st.stats().bytes, 10 * 64);
    // Scrolled by one: the previous sync's rows stay protected for a sync (a grid's rows
    // come a frame late after a reflow, U-C1f-5), so nothing shown last time goes yet.
    st.sync(v, &src, view(10..20));
    assert!(
        (0..10).all(|i| ready(&st, i)),
        "the previous sync's rows stay"
    );
    serve(&mut st, 0.0);
    st.sync(v, &src, view(11..21));
    assert!(
        (0..10).all(|i| !ready(&st, i)),
        "two syncs on, the old rows go"
    );
    assert!((10..20).all(|i| ready(&st, i)));
    assert!(st.stats().bytes <= st.stats().protected_bytes);
}

#[test]
fn after_scrolling_down_the_screen_loads_in_reading_order_then_the_margin_below() {
    let mut st = store(1 << 20);
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..8));
    serve(&mut st, 0.0);
    st.sync(v, &src, view(40..48));
    assert_eq!(st.direction(v), Direction::Down);
    let order = serve(&mut st, 0.0);
    // 8 rows: a 25% margin is 2 rows a side. Below (ahead) nearest first, then above.
    assert_eq!(order, vec![40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 39, 38]);
}

#[test]
fn after_scrolling_up_the_margin_above_loads_first_nearest_first() {
    let mut st = store(1 << 20);
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(60..68));
    serve(&mut st, 0.0);
    st.sync(v, &src, view(20..28));
    assert_eq!(st.direction(v), Direction::Up);
    let requests = drain(&mut st, 0.0);
    let order: Vec<usize> = requests.iter().map(|r| index(r.key)).collect();
    assert_eq!(order, vec![20, 21, 22, 23, 24, 25, 26, 27, 19, 18, 28, 29]);
    let tiers: Vec<Tier> = requests.iter().map(|r| r.tier).collect();
    assert_eq!(
        tiers[8..],
        [Tier::Ahead, Tier::Ahead, Tier::Behind, Tier::Behind]
    );
}

#[test]
fn scrolling_re_ranks_queued_work_toward_the_new_viewport() {
    let mut st = store(1 << 20);
    let src = source(200, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..8));
    let first = st.pop(0.0).expect("row 0");
    assert_eq!(index(first.key), 0);
    // Scrolled before the rest was asked for: the next request is the new viewport's top.
    st.sync(v, &src, view(100..108));
    assert_eq!(st.pop(0.0).map(|r| index(r.key)), Some(100));
}

#[test]
fn a_grid_margin_is_whole_lines_nearest_line_first() {
    let mut st = store(1 << 20);
    let src = source(200, &[]);
    let v = st.viewer();
    // Four lines of four on screen: a margin of one line a side.
    st.sync(v, &src, view(100..116).per_line(4));
    serve(&mut st, 0.0);
    st.sync(v, &src, view(80..96).per_line(4));
    let order: Vec<usize> = drain(&mut st, 0.0).iter().map(|r| index(r.key)).collect();
    let mut want: Vec<usize> = (80..96).collect();
    want.extend([76, 77, 78, 79]); // the line above, in reading order
    // The line below (96..100) was already the earlier view's margin above: held.
    assert_eq!(order, want);
}

#[test]
fn nothing_past_the_margin_is_asked_for() {
    let mut st = store(1 << 20);
    let src = source(1000, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(500..520));
    let asked: Vec<usize> = serve(&mut st, 0.0);
    assert_eq!(asked.iter().min(), Some(&495));
    assert_eq!(asked.iter().max(), Some(&524));
    assert_eq!(asked.len(), 30);
}

#[test]
fn scrolling_away_cancels_pending_work_and_drops_its_late_result() {
    let mut st = store(1 << 20);
    let src = source(1000, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..10));
    let out = drain(&mut st, 0.0);
    assert_eq!(out.len(), 13);
    st.sync(v, &src, view(500..510));
    let cancelled = st.take_cancelled();
    assert_eq!(
        cancelled.len(),
        13,
        "every request for rows past the new margin"
    );
    assert!(st.take_cancelled().is_empty(), "taken once");
    let late = st.complete(out[0].ticket, image(S4), 0.1);
    assert_eq!(late, Landed::Dropped(Why::Cancelled));
    assert_eq!(
        st.state(key(0)),
        State::Empty,
        "the late result is not held"
    );
    // Scrolled back: asked again.
    st.sync(v, &src, view(0..10));
    assert_eq!(st.pop(0.2).map(|r| index(r.key)), Some(0));
}

#[test]
fn a_rebound_row_draws_only_its_own_entry() {
    // A filter replaces the listing under the same rows: requests for entries gone from the
    // viewer are cancelled, and each key's handles stay its own.
    let mut st = store(1 << 20);
    let mut src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, Viewport::new(0..10, S8));
    let out = drain(&mut st, 0.0);
    src.retain(|e| e.id.get() % 2 == 0);
    st.resync(v, &src, 2);
    let cancelled: Vec<usize> = st.take_cancelled().iter().map(|t| index(t.key())).collect();
    assert!(
        cancelled.iter().all(|i| i % 2 == 0),
        "only keys no longer near: {cancelled:?}"
    );
    // Each result is as wide as its own entry's index says (1 to 8 px at the 8 px step), so
    // a handle showing another entry's image would have another width.
    let tagged = |i: usize| {
        let w = 1 + (i % 8) as u32;
        Outcome::Ready {
            width: w,
            height: 1,
            rgba: vec![0; w as usize * 4],
        }
    };
    for r in out {
        st.complete(r.ticket, tagged(index(r.key)), 0.1);
    }
    for i in [1, 3, 5, 7] {
        let w = 1 + (i % 8) as u32;
        assert_eq!(
            st.thumb(key(i)).pixels().size(),
            (w, 1),
            "entry {i} drew its own image"
        );
    }
}

#[test]
fn a_failed_render_is_not_asked_for_again_for_thirty_seconds() {
    let mut st = store(1 << 20);
    let src = source(10, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..1));
    let r = st.pop(0.0).expect("asked");
    assert_eq!(st.complete(r.ticket, Outcome::Failed, 1.0), Landed::Failed);
    assert_eq!(st.state(key(0)), State::Failed { until: 31.0 });
    assert_eq!(
        st.thumb(key(0)).icon_alpha().get(),
        1.0,
        "it keeps its icon"
    );
    // Scrolled away and back inside the wait: the ranking runs again, and still skips it.
    st.sync(v, &src, view(5..6));
    st.sync(v, &src, view(0..1));
    let nothing: Vec<usize> = drain(&mut st, 10.0).iter().map(|r| index(r.key)).collect();
    assert!(!nothing.contains(&0), "asked again inside the wait");
    let nothing: Vec<usize> = drain(&mut st, 30.9).iter().map(|r| index(r.key)).collect();
    assert!(!nothing.contains(&0));
    assert_eq!(
        st.pop(31.0).map(|r| index(r.key)),
        Some(0),
        "asked again after 30 s"
    );
}

#[test]
fn a_stopped_render_is_not_failed_and_is_asked_again_once_the_operation_ends() {
    let mut st = store(1 << 20);
    let src = source(10, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..1));
    let r = st.pop(0.0).expect("asked");
    assert_eq!(
        st.complete(r.ticket, Outcome::Stopped, 0.0),
        Landed::Stopped
    );
    assert_eq!(st.state(key(0)), State::Stopped);
    assert!(
        drain(&mut st, 100.0).iter().all(|r| index(r.key) != 0),
        "not before it ends"
    );
    st.requeue_stopped();
    assert_eq!(st.pop(100.0).map(|r| index(r.key)), Some(0));
}

#[test]
fn turning_thumbnails_off_cancels_and_asks_nothing_and_on_restores_without_decoding() {
    let mut st = store(1 << 20);
    let src = source(40, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..8));
    serve(&mut st, 0.0);
    st.sync(v, &src, view(8..16));
    let pending = drain(&mut st, 0.0);
    assert!(!pending.is_empty());
    let version = st.version(key(0));
    assert!(st.set_enabled(false));
    assert!(!st.set_enabled(false), "no change, no commit");
    assert_ne!(st.version(key(0)), version, "rows build again, as icons");
    assert_eq!(st.take_cancelled().len(), pending.len());
    assert!(st.pop(0.0).is_none());
    st.sync(v, &src, view(0..8));
    assert!(st.pop(0.0).is_none(), "a scroll asks nothing while off");
    assert!(st.set_enabled(true));
    assert!((0..8).all(|i| ready(&st, i)), "held images draw again");
    let asked: Vec<usize> = drain(&mut st, 0.0).iter().map(|r| index(r.key)).collect();
    assert!(
        asked.iter().all(|i| *i >= 8),
        "nothing on screen decodes again: {asked:?}"
    );
}

#[test]
fn a_relocated_key_keeps_its_handles_and_image() {
    let mut st = store(1 << 20);
    let src = source(10, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..2));
    serve(&mut st, 0.0);
    let before = st.thumb(key(0));
    let renamed = ThumbKey::new(77);
    assert!(st.relocate(key(0), renamed));
    let after = st.thumb(renamed);
    assert_eq!(after.epoch(), before.epoch());
    assert_eq!(after.pixels().size(), (4, 4));
    assert!(matches!(st.state(renamed), State::Ready { .. }));
    assert_eq!(st.state(key(0)), State::Empty);
    assert!(!st.relocate(key(0), renamed), "nothing left to move");
    assert_eq!(st.stats().bytes, 3 * 64, "0, 1 and the margin's 2");
}

#[test]
fn a_new_step_replaces_the_image_in_place_after_first_loads() {
    let mut st = ThumbnailStore::new(Settings {
        margin: 0.0,
        ..Settings::default()
    });
    let src = source(40, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..4));
    serve(&mut st, 0.0);
    let handle = st.thumb(key(0));
    // Zoomed in, and scrolled so 4 and 5 show for the first time.
    st.sync(v, &src, Viewport::new(2..6, S8));
    let requests = drain(&mut st, 0.0);
    let order: Vec<usize> = requests.iter().map(|r| index(r.key)).collect();
    assert_eq!(order[..4], [4, 5, 2, 3], "first loads before new steps");
    assert!(requests.iter().all(|r| r.step == S8));
    assert_eq!(
        handle.pixels().size(),
        (4, 4),
        "the old image draws meanwhile"
    );
    st.sync(v, &src, Viewport::new(0..4, S8));
    let r = drain(&mut st, 0.0);
    let zero = r.iter().find(|r| index(r.key) == 0).expect("row 0 at 8 px");
    st.complete(zero.ticket, image(S8), 0.0);
    assert_eq!(handle.pixels().size(), (8, 8), "replaced in place");
    assert_eq!(st.thumb(key(0)).epoch(), handle.epoch());
}

#[test]
fn two_viewers_load_both_screens_first_at_the_larger_step() {
    let mut st = store(1 << 20);
    let src = source(200, &[]);
    let (a, b) = (st.viewer(), st.viewer());
    st.sync(a, &src, Viewport::new(0..4, S4));
    st.sync(b, &src, Viewport::new(100..104, S8));
    st.sync(b, &src, Viewport::new(2..6, S8));
    let requests = drain(&mut st, 0.0);
    let order: Vec<usize> = requests.iter().map(|r| index(r.key)).collect();
    assert_eq!(
        order[..6],
        [0, 1, 2, 3, 4, 5],
        "both screens, each key once"
    );
    let two = requests.iter().find(|r| index(r.key) == 2).expect("2");
    assert_eq!(two.step, S8, "shown by both: the larger step");
    st.drop_viewer(b);
    assert!(st.take_cancelled().iter().all(|t| index(t.key()) >= 4));
}

#[test]
fn bad_images_fail_the_key_without_a_panic() {
    let mut st = store(1 << 20);
    let src = source(10, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..4));
    let bad = [
        Outcome::Ready {
            width: 0,
            height: 4,
            rgba: Vec::new(),
        },
        Outcome::Ready {
            width: 4,
            height: 4,
            rgba: vec![0; 3],
        },
        Outcome::Ready {
            width: 16,
            height: 2,
            rgba: vec![0; 128],
        },
        Outcome::Ready {
            width: u32::MAX,
            height: u32::MAX,
            rgba: Vec::new(),
        },
    ];
    for outcome in bad {
        let r = st.pop(0.0).expect("asked");
        assert_eq!(
            st.complete(r.ticket, outcome, 0.0),
            Landed::Dropped(Why::Invalid)
        );
        assert!(matches!(st.state(r.key), State::Failed { .. }));
    }
    assert_eq!(st.stats().bytes, 0);
}

#[test]
fn entries_without_a_thumbnail_key_are_never_asked_for() {
    let mut st = store(1 << 20);
    let src = source(10, &[0, 2, 4]);
    let v = st.viewer();
    st.sync(v, &src, view(0..6));
    let asked = serve(&mut st, 0.0);
    assert_eq!(asked, vec![1, 3, 5, 6, 7]);
}

#[test]
fn an_unchanged_viewport_returns_at_once() {
    let mut st = store(1 << 20);
    let src = source(10, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..4));
    let r = st.pop(0.0).expect("asked");
    st.sync(v, &src, view(0..4));
    assert_eq!(
        st.pop(0.0).map(|r| index(r.key)),
        Some(1),
        "the queue carried on"
    );
    assert!(st.take_cancelled().is_empty());
    st.complete(r.ticket, image(S4), 0.0);
}

#[test]
fn pruning_keeps_handles_near_any_viewer_and_asks_for_one_build() {
    let mut st = store(1 << 20);
    st.set_prune_slack(50);
    let src = source(2000, &[]);
    let v = st.viewer();
    assert!(!st.sync(v, &src, view(0..10)));
    let near = st.thumb(key(40)); // within five viewports of 20..30
    let far = st.thumb(key(900));
    let far_version = st.version(key(900));
    // Many empty handles, as a long scroll builds rows that never load.
    for i in 0..200 {
        let _ = st.thumb(key(1500 + i));
    }
    assert!(
        st.sync(v, &src, view(20..30)),
        "a prune asks the host for one build"
    );
    assert!(!st.sync(v, &src, view(30..40)), "and only once");
    assert_eq!(
        st.thumb(key(40)).epoch(),
        near.epoch(),
        "kept: near the viewport"
    );
    let fresh = st.thumb(key(900));
    assert_ne!(
        fresh.epoch(),
        far.epoch(),
        "pruned, and seen again with new handles"
    );
    assert_ne!(
        st.version(key(900)),
        far_version,
        "a row drawing the old handles builds again"
    );
}

// Review round 1.

/// The scans `pop` has made so far.
fn scans(st: &ThumbnailStore) -> usize {
    st.inner.borrow().scans
}

#[test]
fn a_still_view_drains_its_queue_when_the_margin_overflows_the_budget() {
    // Room for 10 images: the 8 on screen and 2 of the 4 in the margin.
    let mut st = store(10 * 64);
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(20..28));
    let mut requests = 0;
    while let Some(r) = st.pop(0.0) {
        requests += 1;
        st.complete(r.ticket, image(r.step), 0.0);
        if requests > 100 {
            break;
        }
    }
    assert!(
        requests <= 12,
        "{requests} requests for 12 keys and no scroll"
    );
    assert!((20..28).all(|i| ready(&st, i)), "the screen loaded");
    let stats = st.stats();
    assert!(stats.bytes <= stats.budget);
}

#[test]
fn a_result_dropped_for_the_budget_evicts_nothing() {
    let small = || Outcome::Ready {
        width: 4,
        height: 4,
        rgba: vec![0; 64],
    };
    // Room for 256 bytes: one 8 px image, or four 4 px ones. A one-line margin.
    let mut st = ThumbnailStore::new(Settings::default().budget(256).margin(1.0));
    let src = source(100, &[]);
    let a = st.viewer();
    st.sync(a, &src, Viewport::new(10..11, S8));
    let shown = st.pop(0.0).expect("row 10");
    let margin = st
        .pop(0.0)
        .expect("row 11, the margin, while the budget is empty");
    assert_eq!(margin.tier, Tier::Ahead);
    // Another view holds a small image, then goes: held, and no one wants it.
    let b = st.viewer();
    st.sync(b, &src, Viewport::new(50..51, S8));
    let other = st.pop(0.0).expect("row 50");
    assert_eq!(index(other.key), 50);
    st.complete(other.ticket, small(), 0.0);
    st.drop_viewer(b);
    st.complete(shown.ticket, small(), 0.0);
    // The margin's 8 px image can't fit even with row 50 evicted: it is dropped, and row 50
    // stays.
    assert_eq!(
        st.complete(margin.ticket, image(S8), 0.0),
        Landed::Dropped(Why::Budget)
    );
    assert!(
        ready(&st, 50),
        "nothing evicted for a result dropped anyway"
    );
}

#[test]
fn relocating_a_key_on_screen_keeps_it_protected_and_the_budget_whole() {
    let mut st = store(2 * 64);
    let src = source(10, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..4));
    serve(&mut st, 0.0);
    let before = st.stats();
    assert!(
        before.bytes > before.budget,
        "the screen alone is over budget"
    );
    assert!(st.relocate(key(0), ThumbKey::new(5000)));
    let after = st.stats();
    assert_eq!(
        after.protected_bytes, before.protected_bytes,
        "still on screen, under its new key"
    );
    assert!(after.bytes <= after.budget || after.bytes == after.protected_bytes);
}

#[test]
fn an_unchanged_queue_pops_nothing_without_scanning_and_drains_in_one_pass() {
    let mut st = ThumbnailStore::new(Settings::default().budget(1 << 30));
    let src = source(20_000, &[]);
    let v = st.viewer();
    // 4K at 48 pt: 80 tiles a line, 45 lines on screen.
    st.sync(v, &src, Viewport::new(0..3600, S4).per_line(80));
    let start = scans(&st);
    let served = serve(&mut st, 0.0).len();
    let drained = scans(&st) - start;
    assert!(
        drained <= 3 * served,
        "{drained} scans to drain {served} requests"
    );
    let idle = scans(&st);
    for _ in 0..100 {
        assert!(st.pop(0.0).is_none());
    }
    assert_eq!(scans(&st), idle, "an idle pop looks at nothing");
}

#[test]
fn failed_keys_far_from_every_viewer_are_pruned() {
    let mut st = store(1 << 20);
    st.set_prune_slack(50);
    let src = source(3000, &[]);
    let v = st.viewer();
    // A fast scroll through a folder of files that all fail, well within the 30 s.
    for start in (0..2900).step_by(10) {
        st.sync(v, &src, view(start..start + 10));
        for _ in 0..100 {
            let Some(r) = st.pop(0.0) else { break };
            st.complete(r.ticket, Outcome::Failed, 0.0);
        }
    }
    let entries = st.stats().entries;
    assert!(
        entries < 200,
        "{entries} entries kept for a 10-row viewport"
    );
    assert!(
        matches!(st.state(key(2895)), State::Failed { .. }),
        "near ones keep their wait"
    );
}

#[test]
fn a_reflow_is_not_read_as_a_scroll() {
    let mut st = store(1 << 20);
    let src = source(2000, &[]);
    let v = st.viewer();
    st.sync(v, &src, Viewport::new(30..90, S4).per_line(6));
    st.sync(v, &src, Viewport::new(42..102, S4).per_line(6));
    assert_eq!(st.direction(v), Direction::Down);
    // Narrowed to 4 a line around the same top tile: its line now starts at 40.
    st.sync(v, &src, Viewport::new(40..80, S4).per_line(4));
    assert_eq!(
        st.direction(v),
        Direction::Down,
        "a reflow keeps the direction"
    );
}

#[test]
fn a_new_margin_applies_at_the_next_sync_of_the_same_viewport() {
    let mut st = store(1 << 20);
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(40..48));
    let settings = st.settings().margin(0.0);
    st.set_settings(settings);
    st.sync(v, &src, view(40..48));
    let asked: Vec<usize> = serve(&mut st, 0.0);
    assert_eq!(asked, (40..48).collect::<Vec<_>>(), "no margin any more");
}

#[test]
fn a_step_change_alone_reaches_the_store() {
    let mut st = store(1 << 20);
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, Viewport::new(40..44, S4).per_line(2));
    serve(&mut st, 0.0);
    // Zoomed: no row moved, so no on_visible; the host passes the new step.
    st.set_step(v, &src, S8);
    let requests = drain(&mut st, 0.0);
    let order: Vec<usize> = requests.iter().map(|r| index(r.key)).collect();
    assert_eq!(
        order[..4],
        [40, 41, 42, 43],
        "the screen first, at the new step"
    );
    assert!(requests.iter().all(|r| r.step == S8));
    assert_eq!(st.direction(v), Direction::Down);
}

#[test]
fn a_4k_grid_at_the_default_budget_drains_its_queue() {
    // C3 on a 4K screen at scale 2 with 72 pt tiles: 128 px classes, 26 a line and 15 lines
    // (about 25 MiB at 64 KiB each), a 25% margin of 4 lines a side, and a budget that holds
    // the screen and only part of the margin.
    let step = Step::new(128);
    let mut st = ThumbnailStore::new(Settings::default().budget(500 * 128 * 128 * 4));
    let src = source(5000, &[]);
    let v = st.viewer();
    st.sync(v, &src, Viewport::new(1040..1430, step).per_line(26));
    let wanted = st.inner.borrow().wanted.len();
    let asked = serve(&mut st, 0.0).len();
    assert!(asked <= wanted, "{asked} requests for {wanted} keys");
    assert!((1040..1430).all(|i| ready(&st, i)), "the screen loaded");
    let stats = st.stats();
    assert!(stats.bytes <= stats.budget);
    assert!(st.pop(0.0).is_none(), "and stays drained");
}

// Review round 2.

#[test]
fn a_request_out_at_an_old_step_is_asked_again_at_the_new_one() {
    let mut st = ThumbnailStore::new(Settings::default().margin(0.0));
    let src = source(10, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(0..4));
    let out = drain(&mut st, 0.0);
    assert_eq!(out.len(), 4);
    st.set_step(v, &src, S8);
    // The host polls while the 4 px requests are still out: nothing new to ask yet.
    let mut again = drain(&mut st, 0.05);
    assert!(again.is_empty());
    for r in &out {
        st.complete(r.ticket, image(r.step), 0.1);
    }
    // The 4 px images show meanwhile; then each is asked for at 8 px.
    assert!((0..4).all(|i| st.thumb(key(i)).pixels().size() == (4, 4)));
    again.extend(drain(&mut st, 60.0));
    let asked: Vec<usize> = again.iter().map(|r| index(r.key)).collect();
    assert_eq!(asked, vec![0, 1, 2, 3]);
    assert!(again.iter().all(|r| r.step == S8));
}

#[test]
fn a_second_viewer_at_a_larger_step_gets_it_after_the_first_viewers_results_land() {
    let mut st = ThumbnailStore::new(Settings::default().margin(0.0));
    let src = source(10, &[]);
    let (a, b) = (st.viewer(), st.viewer());
    st.sync(a, &src, view(0..4));
    let out = drain(&mut st, 0.0);
    st.sync(b, &src, Viewport::new(0..4, S8));
    let mut asked: Vec<Request> = drain(&mut st, 0.0);
    for r in &out {
        st.complete(r.ticket, image(r.step), 0.1);
    }
    asked.extend(drain(&mut st, 0.1));
    let at8: Vec<usize> = asked
        .iter()
        .filter(|r| r.step == S8)
        .map(|r| index(r.key))
        .collect();
    assert_eq!(at8, vec![0, 1, 2, 3], "b's 8 px images asked for");
}

#[test]
fn after_new_settings_every_viewer_keeps_its_step() {
    let mut st = store(1 << 20);
    let src = source(200, &[]);
    let (a, b) = (st.viewer(), st.viewer());
    st.sync(a, &src, Viewport::new(0..4, S8));
    st.sync(b, &src, Viewport::new(100..104, S8));
    serve(&mut st, 0.0);
    let settings = st.settings().budget(2 << 20);
    st.set_settings(settings);
    // Another viewer scrolls before a syncs again: a's keys stay wanted at 8 px.
    st.sync(b, &src, Viewport::new(101..105, S8));
    let asked = drain(&mut st, 0.0);
    assert!(
        asked.iter().all(|r| r.step == S8),
        "{:?}",
        asked.iter().map(|r| r.step).collect::<Vec<_>>()
    );
    assert!((0..4).all(|i| matches!(st.state(key(i)), State::Ready { step: S8, .. })));
}

#[test]
fn a_resync_after_new_settings_still_cancels_what_left() {
    let mut st = store(1 << 20);
    let mut src = source(200, &[]);
    let v = st.viewer();
    st.sync(v, &src, Viewport::new(0..4, S8));
    let out = drain(&mut st, 0.0);
    let settings = st.settings().budget(2 << 20);
    st.set_settings(settings);
    src.retain(|e| e.id.get() % 2 == 0);
    st.resync(v, &src, 2);
    let cancelled = st.take_cancelled().len();
    assert!(
        cancelled > 0 && cancelled < out.len(),
        "{cancelled} of {} cancelled",
        out.len()
    );
}

/// A margin result dropped for the budget while another viewer fills it.
fn deferred_margin() -> (ThumbnailStore, ListSource, ViewerId, ViewerId) {
    let mut st = ThumbnailStore::new(Settings::default().budget(256).margin(1.0));
    let src = source(100, &[]);
    let a = st.viewer();
    st.sync(a, &src, view(10..11));
    let a_out = drain(&mut st, 0.0);
    let b = st.viewer();
    st.sync(b, &src, view(50..54));
    for r in drain(&mut st, 0.0) {
        st.complete(r.ticket, image(r.step), 0.0);
    }
    let landed: Vec<Landed> = a_out
        .iter()
        .map(|r| st.complete(r.ticket, image(r.step), 0.0))
        .collect();
    assert!(landed.contains(&Landed::Dropped(Why::Budget)), "{landed:?}");
    (st, src, a, b)
}

#[test]
fn a_deferred_margin_loads_once_the_viewer_filling_the_budget_goes() {
    let (mut st, src, a, b) = deferred_margin();
    st.drop_viewer(b);
    // a stands still: its sync from poll returns at once.
    st.sync(a, &src, view(10..11));
    serve(&mut st, 1.0);
    assert!(ready(&st, 9) && ready(&st, 11), "a's margin loaded");
}

#[test]
fn a_deferred_margin_loads_once_the_budget_grows() {
    let (mut st, _src, _a, _b) = deferred_margin();
    let settings = st.settings().budget(1 << 20);
    st.set_settings(settings);
    serve(&mut st, 1.0);
    assert!(
        ready(&st, 9) && ready(&st, 11),
        "a's margin loaded with no sync"
    );
}

// Review round 3.

const S16: Step = Step::new(16);

#[test]
fn a_still_view_loads_its_margin_after_a_zoom_out() {
    // The screen at 16 px (1 KiB each) fills the budget, so the margin waits; at 4 px it fits.
    let mut st = ThumbnailStore::new(Settings::default().budget(8 * 1024 + 32));
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, Viewport::new(40..48, S16));
    serve(&mut st, 0.0);
    st.set_step(v, &src, S4);
    // A pool of 16 takes the screen's 4 px requests (and passes the margin, still gated by the
    // 16 px images) before any lands; the view stands still.
    let mut out: Vec<Request> = Vec::new();
    for frame in 0..200 {
        let now = 1.0 + f64::from(frame) * 0.016;
        for r in std::mem::take(&mut out) {
            st.complete(r.ticket, image(r.step), now);
        }
        while out.len() < 16 {
            let Some(r) = st.pop(now) else { break };
            out.push(r);
        }
        st.sync(v, &src, Viewport::new(40..48, S4));
    }
    let missing: Vec<usize> = [38, 39, 48, 49]
        .into_iter()
        .filter(|&i| !ready(&st, i))
        .collect();
    assert!(
        missing.is_empty(),
        "margin rows {missing:?} never asked for"
    );
}

#[test]
fn a_deferred_margin_loads_once_a_wanted_image_shrinks() {
    let mut st = ThumbnailStore::new(Settings::default().budget(8 * 1024 + 32));
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, Viewport::new(40..48, S16));
    // Everything asked for before anything lands: the margin's results are dropped.
    let out = drain(&mut st, 0.0);
    let landed: Vec<Landed> = out
        .iter()
        .map(|r| st.complete(r.ticket, image(r.step), 0.0))
        .collect();
    assert!(landed.contains(&Landed::Dropped(Why::Budget)), "{landed:?}");
    st.set_step(v, &src, S4);
    // The screen's 4 px requests out before any lands; then they land, with no new viewport.
    let shrink = drain(&mut st, 1.0);
    for r in &shrink {
        st.complete(r.ticket, image(r.step), 1.0);
    }
    serve(&mut st, 1.0);
    let missing: Vec<usize> = [38, 39, 48, 49]
        .into_iter()
        .filter(|&i| !ready(&st, i))
        .collect();
    assert!(missing.is_empty(), "margin rows {missing:?} left deferred");
}

#[test]
fn a_margin_request_ending_lets_the_next_margin_key_in() {
    // Room for the screen and one margin image.
    let mut st = ThumbnailStore::new(Settings::default().budget(128).margin(1.0));
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(10..11));
    let shown = st.pop(0.0).expect("row 10");
    st.complete(shown.ticket, image(S4), 0.0);
    let first = st.pop(0.0).expect("row 11, the margin ahead");
    assert_eq!(index(first.key), 11);
    assert!(st.pop(0.0).is_none(), "row 9 waits for room");
    // Row 11's render fails: its room is free again.
    st.complete(first.ticket, Outcome::Failed, 0.0);
    assert_eq!(st.pop(0.0).map(|r| index(r.key)), Some(9));
}

#[test]
fn a_margin_key_scrolled_on_screen_gives_back_its_margin_room() {
    let mut st = ThumbnailStore::new(Settings::default().budget(191).margin(1.0));
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, view(10..11));
    let shown = st.pop(0.0).expect("row 10");
    st.complete(shown.ticket, image(S4), 0.0);
    let _margin = st.pop(0.0).expect("row 11, the margin ahead");
    assert!(st.pop(0.0).is_none(), "row 9 waits for room");
    // Row 11 scrolls on screen while its request is out: it is no longer margin, and row 12
    // (the new margin) fits beside it.
    st.sync(v, &src, view(11..12));
    assert_eq!(st.pop(0.0).map(|r| index(r.key)), Some(12));
}

#[test]
fn a_relocation_that_frees_room_lets_a_deferred_margin_in() {
    let mut st = ThumbnailStore::new(Settings::default().budget(384).margin(1.0));
    let src = source(100, &[]);
    // Viewer a at 4 px: row 10 and its margin, 9 and 11, all asked for first.
    let a = st.viewer();
    st.sync(a, &src, view(10..11));
    let a_out = drain(&mut st, 0.0);
    // Viewer b at 8 px fills the budget with rows 50 and 51.
    let b = st.viewer();
    st.sync(b, &src, Viewport::new(50..52, S8));
    for r in drain(&mut st, 0.0) {
        st.complete(r.ticket, image(r.step), 0.0);
    }
    let landed: Vec<Landed> = a_out
        .iter()
        .map(|r| st.complete(r.ticket, image(r.step), 0.0))
        .collect();
    assert_eq!(
        landed
            .iter()
            .filter(|l| **l == Landed::Dropped(Why::Budget))
            .count(),
        2
    );
    // b's row 50 is renamed onto 51: 51's 8 px image goes, room for one of a's margin images.
    assert!(st.relocate(key(50), key(51)));
    // a stands still (its sync from poll returns at once).
    st.sync(a, &src, view(10..11));
    serve(&mut st, 1.0);
    assert!(
        ready(&st, 9) || ready(&st, 11),
        "a's margin loaded once there was room"
    );
}

#[test]
fn a_wanted_margin_image_evicted_for_the_screen_is_asked_again_once_room_appears() {
    // Keys 49 to 52 have no thumbnails, so viewer b has no margin of its own.
    let mut st = ThumbnailStore::new(Settings::default().budget(2175).margin(1.0));
    let src = source(100, &[49, 51]);
    let a = st.viewer();
    // a's row 10 and its margin (9, 11) at 4 px.
    st.sync(a, &src, view(10..11));
    serve(&mut st, 0.0);
    // Zoomed in: only row 10 is asked for at 16 px.
    st.set_step(a, &src, S16);
    let big = st.pop(0.0).expect("row 10 at 16 px");
    assert_eq!(index(big.key), 10);
    st.complete(big.ticket, image(S16), 0.0);
    // Zoomed out again: row 10's 4 px request goes out; the margin is already at 4 px.
    st.set_step(a, &src, S4);
    let small = st.pop(0.0).expect("row 10 at 4 px");
    assert_eq!(index(small.key), 10);
    // Another view shows row 50 at 16 px: it lands and evicts one of a's margin images.
    let b = st.viewer();
    st.sync(b, &src, Viewport::new(50..51, S16));
    let other = st.pop(0.0).expect("row 50");
    assert_eq!(index(other.key), 50);
    st.complete(other.ticket, image(S16), 0.0);
    assert!(
        !(ready(&st, 9) && ready(&st, 11)),
        "a margin image went for the screen"
    );
    // Row 10 lands at 4 px: room for the margin image again.
    st.complete(small.ticket, image(S4), 0.0);
    serve(&mut st, 0.0);
    assert!(ready(&st, 9) && ready(&st, 11), "a's margin loaded again");
}

#[test]
fn absurd_margins_and_steps_neither_panic_nor_overflow() {
    for margin in [f32::INFINITY, f32::NAN, -1.0, 1e30] {
        let mut st = ThumbnailStore::new(Settings::default().margin(margin));
        let src = source(10, &[]);
        let v = st.viewer();
        st.sync(v, &src, Viewport::new(0..4, S4).per_line(3));
        assert!(serve(&mut st, 0.0).len() <= 10);
    }
    let mut st = ThumbnailStore::default();
    let src = source(100, &[]);
    let v = st.viewer();
    st.sync(v, &src, Viewport::new(10..14, Step::new(u32::MAX)));
    let _ = drain(&mut st, 0.0);
}

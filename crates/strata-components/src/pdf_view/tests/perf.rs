//! Perf on a 500-page PDF: 500 Letter pages, each with a 2,700-character text layer.
//! `#[ignore]`d; run with the `perf` profile, and Rune's commit budget to hear slow commits:
//!
//! ```sh
//! RUNE_COMMIT_BUDGET_MS=8 cargo test --profile perf -p strata-components --lib \
//!     pdf_view::tests::perf -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! The host renders at once in its poll (an image of the size asked, filled with one value,
//! and the page's layer), so the times are the view's, not a renderer's. It times the first
//! frame; 600 wheel frames at 120 Hz of 48 points each; 60 Ctrl+wheel notches, one a frame,
//! and the requests once the zoom rests; Ctrl+End and the 10 frames after; a drag selecting
//! across three pages; Ctrl+A then Ctrl+C with every page's layer held; and `pop`.

use std::cell::Cell as StdCell;
use std::time::Instant;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, Tx, WheelEvent, clipboard};
use strata_kit::look::Look;

use super::super::*;
use super::layer_of;
use crate::preview_drawer::HostedPreview;

thread_local! {
    static VIEWS: StdCell<usize> = const { StdCell::new(0) };
    static RENDERS: StdCell<usize> = const { StdCell::new(0) };
}

fn release_only() {
    if cfg!(debug_assertions) {
        panic!("perf tests measure release builds: run them with --profile perf");
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

const PAGES: usize = 500;

/// Page `p`'s text: 36 lines of 75 characters.
fn lines(p: usize) -> Vec<String> {
    (0..36)
        .map(|l| {
            format!(
                "page {p:03} line {l:02} {}",
                "the quick brown fox jumps ".repeat(2)
            )
        })
        .map(|s| s.chars().take(75).collect())
        .collect()
}

struct Host {
    pdf: PdfState,
    look: Look,
    texts: Vec<Vec<String>>,
}

fn view(h: &Host, cx: &mut Cx<Host>) -> El {
    VIEWS.set(VIEWS.get() + 1);
    PdfView::new(|h: &Host| &h.pdf, |h: &mut Host| &mut h.pdf).build(h, &h.look, cx)
}

/// The host's poll: every request rendered at once.
fn poll(h: &mut Host, t: f64, tx: &mut Tx) -> bool {
    let mut changed = h.pdf.tick(t) | h.pdf.flush(tx);
    h.pdf.take_cancelled();
    while let Some(r) = h.pdf.pop() {
        RENDERS.set(RENDERS.get() + 1);
        // Text-only requests render small, as a host may: only their layer is wanted.
        let w = if r.kind == RenderKind::TextOnly {
            16
        } else {
            r.box_px[0]
        };
        let height = w * 792 / 612;
        let lines: Vec<&str> = h.texts[r.page].iter().map(String::as_str).collect();
        let mut layer = layer_of(&lines);
        // The fixture's boxes are in 400-point units: scale them to the render.
        let k = w as f32 / 400.0;
        for g in &mut layer.glyphs {
            for v in g.iter_mut() {
                *v *= k;
            }
        }
        layer.width = w as f32;
        layer.height = height as f32;
        let reply = PageReply::Rendered {
            width: w,
            height,
            rgba: vec![255; (w * height * 4) as usize],
            text: Some(layer),
        };
        changed |= h.pdf.complete(r.ticket, reply).commits();
    }
    changed
}

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

fn host() -> Host {
    let mut pdf = PdfState::new("pdf");
    pdf.open(PdfDocument::new(PAGES, [612.0, 792.0]));
    Host {
        pdf,
        look: Look::fallback(false),
        texts: (0..PAGES).map(lines).collect(),
    }
}

fn runtime() -> Runtime<Host> {
    let mut rt = Runtime::new(host(), view, (700.0, 900.0), Env::new(false, 2.0));
    rt.poll = Some(poll);
    rt
}

fn frames(
    rt: &mut Runtime<Host>,
    n: usize,
    mut each: impl FnMut(&mut Runtime<Host>),
) -> (f64, f64, f64, usize) {
    let mut v = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        each(rt);
        rt.frame(1.0 / 120.0);
        v.push(ms(t));
    }
    let slow = v.iter().filter(|&&f| f > 8.0).count();
    let max = v.iter().copied().fold(0.0, f64::max);
    (percentile(&mut v, 0.5), percentile(&mut v, 0.95), max, slow)
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn five_hundred_pages() {
    release_only();
    // First frame and the one that mounts the rows, best of three.
    let mut first = f64::INFINITY;
    let mut rt = None;
    for _ in 0..3 {
        let t = Instant::now();
        let mut r = runtime();
        r.frame(0.0);
        r.frame(1.0 / 120.0);
        first = first.min(ms(t));
        rt = Some(r);
    }
    let mut rt = rt.expect("runtime");
    for _ in 0..30 {
        rt.frame(1.0 / 120.0);
    }

    VIEWS.set(0);
    RENDERS.set(0);
    let wheel = frames(&mut rt, 600, |rt| rt.wheel(350.0, 450.0, -48.0));
    let (wheel_views, wheel_renders) = (VIEWS.get(), RENDERS.get());

    // Ctrl+wheel: 60 notches, one a frame, in and out by turns, the last five in (ending near
    // 2×), then rest. Strata's caps (1400 pixels wide, 2.5 MP) already stop a 700-point pane at
    // 2× scale from rendering sharper, so this part runs with a renderer that allows more.
    rt.update(|h, _| {
        h.pdf.set_caps(RenderCaps {
            max_width: 4096,
            max_height: 8192,
            max_pixels: 32_000_000,
        })
    });
    VIEWS.set(0);
    RENDERS.set(0);
    let mut notch = 0;
    let zoom = frames(&mut rt, 60, |rt| {
        let dy = if notch >= 55 || notch % 2 == 0 {
            40.0
        } else {
            -40.0
        };
        notch += 1;
        rt.wheel_event(WheelEvent::new(0.0, dy).at(350.0, 450.0).mods(ctrl()));
    });
    let renders_while_zooming = RENDERS.get();
    for _ in 0..30 {
        rt.frame(1.0 / 120.0);
    }
    let visible = rt.state.pdf.visible().len();
    let renders_after_rest = RENDERS.get() - renders_while_zooming;
    rt.update(|h, _| h.pdf.set_caps(RenderCaps::default()));

    // A click puts focus in the pages; Ctrl+End, then 10 frames.
    rt.pointer_down(350.0, 450.0);
    rt.pointer_up(350.0, 450.0);
    rt.frame(1.0 / 120.0);
    let t = Instant::now();
    rt.key(KeyEvent {
        key: Key::End,
        mods: ctrl(),
    });
    rt.frame(1.0 / 120.0);
    let end_jump = ms(t);
    let end = frames(&mut rt, 10, |_| {});
    let at_end = rt.state.pdf.visible();
    rt.update(|h, _| {
        h.pdf.set_level(1.0);
    });
    for _ in 0..30 {
        rt.frame(1.0 / 120.0);
    }

    // A drag from mid-page held at the bottom while the wheel scrolls three pages under it.
    rt.update(|h, tx| {
        h.pdf.reset_zoom(tx);
        tx.scroll_to_instant("pdf-scroll", 0.0, 0.0);
    });
    for _ in 0..10 {
        rt.frame(1.0 / 120.0);
    }
    rt.pointer_move(350.0, 300.0);
    rt.pointer_down(350.0, 300.0);
    let mut y: f32 = 300.0;
    let mut i = 0;
    let drag = frames(&mut rt, 60, |rt| {
        i += 1;
        y = (y + 10.0).min(880.0) + (i % 2) as f32;
        if y >= 880.0 {
            rt.wheel(350.0, y, -48.0);
        }
        rt.pointer_move(350.0, y);
    });
    rt.pointer_up(350.0, y);
    rt.frame(1.0 / 120.0);
    let selected_pages = rt.state.pdf.selection.ranges.len();

    // Every page's layer held (text budgets off), then Ctrl+A and Ctrl+C.
    rt.update(|h, _| {
        h.pdf.set_budgets(usize::MAX, usize::MAX);
        h.pdf.set_searchable(true);
        h.pdf.find("fox");
    });
    for _ in 0..(PAGES / 2) {
        rt.frame(1.0 / 120.0);
    }
    let layers = (0..PAGES)
        .filter(|&p| rt.state.pdf.layer(p).is_some())
        .count();
    let mut finds = 0;
    rt.update(|h, _| {
        h.pdf.find_end();
        h.pdf.set_searchable(false);
        finds = 0;
    });
    let t = Instant::now();
    rt.update(|h, _| {
        h.pdf.set_searchable(true);
        h.pdf.find("quick brown");
        finds = h.pdf.find_status().map_or(0, |s| s.total);
    });
    let find_ms = ms(t);
    rt.update(|h, _| h.pdf.find_end());
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    rt.pointer_down(350.0, 450.0);
    rt.pointer_up(350.0, 450.0);
    rt.frame(1.0 / 120.0);
    let t = Instant::now();
    for c in ['a', 'c'] {
        rt.key(KeyEvent {
            key: Key::Command(c),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        rt.frame(1.0 / 120.0);
    }
    let copy_ms = ms(t);
    let copied = clip.text().map_or(0, |t| t.len());

    // `pop` with nothing to do, and with every visible page wanting a new width.
    let t = Instant::now();
    for _ in 0..1000 {
        let _ = rt.state.pdf.pop();
    }
    let idle_pop_us = t.elapsed().as_secs_f64() * 1e6 / 1000.0;

    let f = |(p50, p95, max, slow): (f64, f64, f64, usize)| {
        format!("p50 {p50:.2} p95 {p95:.2} max {max:.2} ms, {slow} over 8 ms")
    };
    println!(
        "pdf {PAGES} pages: first frames {first:.1} ms; wheel 600 frames {} ({wheel_views} view \
         runs, {wheel_renders} renders); Ctrl+wheel 60 notches {} ({renders_while_zooming} \
         renders while zooming, {renders_after_rest} after the rest, {visible} visible); \
         Ctrl+End {end_jump:.1} ms then {} (visible {at_end:?}); drag-select {} \
         ({selected_pages} pages selected); find over {layers} layers {find_ms:.1} ms ({finds} \
         matches); Ctrl+A Ctrl+C {copy_ms:.1} ms, {copied} bytes; idle pop {idle_pop_us:.2} us",
        f(wheel),
        f(zoom),
        f(end),
        f(drag),
    );
    assert_eq!(renders_while_zooming, 0, "no render while the zoom moves");
    assert!(
        renders_after_rest > 0 && renders_after_rest <= visible,
        "visible pages only"
    );
    assert!(at_end.contains(&(PAGES - 1)));
    assert!(selected_pages >= 2);
    assert_eq!(layers, PAGES);
    assert!(copied > PAGES * 2_000);
}

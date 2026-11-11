//! Perf on the largest document Strata renders: 1 MiB of mixed units (Strata's input limit),
//! about 10,000 of them, with code blocks coloured by the real highlighter thread and tables.
//! `#[ignore]`d; run with the `perf` profile, and Rune's commit budget to hear slow commits:
//!
//! ```sh
//! RUNE_COMMIT_BUDGET_MS=8 cargo test --profile perf -p strata-components --lib \
//!     document_view::tests::perf -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! It times the first frame; 600 frames at 120 Hz each scrolled 48 pt by the wheel (as C1e's
//! perf does), the host polling the block pool and rebuilding when colours land; find over the
//! whole document; and Ctrl+A then Ctrl+C, from the model and through Rune's own collection.

use std::cell::Cell as StdCell;
use std::sync::Arc;
use std::time::Instant;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, clipboard};
use strata_kit::look::Look;
use strata_kit::theme::syntax::syntect_theme;

use super::super::*;
use crate::code_view::highlight::{CodeBlocks, Highlighter};
use crate::preview_drawer::HostedPreview;

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

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// 1 MiB of units: headings, paragraphs, list items, a 6-line Rust block every 25 units and a
/// 4-row table every 40.
pub(crate) fn big_document() -> Document {
    let mut units = Vec::new();
    let mut bytes = 0;
    let mut i = 0;
    while bytes < 1 << 20 {
        let u = if i % 40 == 39 {
            let rows: Vec<Vec<Cell>> = (0..4)
                .map(|r| {
                    (0..3)
                        .map(|c| {
                            if r == 0 {
                                Cell::header(format!("Head {c}"))
                            } else {
                                Cell::new(format!("cell {i}.{r}.{c}"))
                            }
                        })
                        .collect()
                })
                .collect();
            let copy: String = rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| c.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\t")
                        + "\n"
                })
                .collect();
            Unit::new(
                UnitKind::Table {
                    list_depth: None,
                    rows,
                },
                "",
            )
            .copy_text(copy)
        } else if i % 25 == 24 {
            Unit::new(
                UnitKind::Code {
                    list_depth: None,
                    language: Some("rust".into()),
                },
                format!(
                    "fn block_{i}(x: u32) -> u32 {{\n    // block {i}\n    let y = x * 2;\n    \
                     let s = \"text {i}\";\n    y + s.len() as u32\n}}"
                ),
            )
        } else if i % 10 == 0 {
            Unit::new(UnitKind::Heading(2), format!("Section {i}"))
        } else if i % 7 == 0 {
            Unit::new(
                UnitKind::ListItem { depth: i % 3 },
                format!("• List item {i} with a few words in it"),
            )
        } else {
            Unit::new(
                UnitKind::Paragraph,
                format!(
                    "Paragraph {i}: the quick brown fox jumps over the lazy dog, and then some \
                     more prose so the line wraps in the preview."
                ),
            )
        };
        bytes += u.copy_text.len();
        units.push(u);
        i += 1;
    }
    Document::new(units)
}

struct Host {
    doc: DocumentState,
    look: Look,
    blocks: CodeBlocks,
}

fn view(h: &Host, cx: &mut Cx<Host>) -> El {
    VIEWS.set(VIEWS.get() + 1);
    DocumentView::new(|h: &Host| &h.doc, |h: &mut Host| &mut h.doc)
        .blocks(&h.blocks)
        .build(h, &h.look, cx)
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn a_one_mib_document() {
    release_only();
    let doc = Arc::new(big_document());
    let n = doc.units.len();
    let look = Look::fallback(false);
    let hl = Highlighter::new(Arc::new(syntect_theme(&look)));

    // First frame, best of three.
    let mut first = f64::INFINITY;
    let mut rt = None;
    for _ in 0..3 {
        let host = Host {
            doc: DocumentState::new(doc.clone()),
            look: look.clone(),
            blocks: hl.blocks(),
        };
        let t = Instant::now();
        let mut r = Runtime::new(host, view, (700.0, 900.0), Env::new(false, 2.0));
        r.frame(0.0);
        first = first.min(ms(t));
        rt = Some(r);
    }
    let mut rt = rt.expect("runtime");
    for _ in 0..30 {
        rt.update_if(|h, _| h.blocks.poll());
        rt.frame(1.0 / 120.0);
    }

    // 600 wheel frames at 120 Hz, polling the pool as a host does.
    VIEWS.set(0);
    let mut frames = Vec::new();
    let mut slow = 0;
    for _ in 0..600 {
        let t = Instant::now();
        rt.wheel(350.0, 450.0, -48.0);
        rt.update_if(|h, _| h.blocks.poll());
        rt.frame(1.0 / 120.0);
        let f = ms(t);
        if f > 8.0 {
            slow += 1;
        }
        frames.push(f);
    }
    let views = VIEWS.get();
    let (p50, p95) = (percentile(&mut frames, 0.5), percentile(&mut frames, 0.95));
    let max = frames.iter().copied().fold(0.0, f64::max);

    // Find over the whole document: each query's run and its match count.
    let mut finds = Vec::new();
    for q in ["the", "fox jumps", "block_9999", "zzz"] {
        let mut total = 0;
        let t = Instant::now();
        rt.update(|h, _| {
            h.doc.find(q);
            total = h.doc.find_status().map_or(0, |s| s.total);
        });
        finds.push(format!("{q:?} {:.1} ms ({total} matches)", ms(t)));
    }

    // Ctrl+A then Ctrl+C: from the model, then through Rune's collection.
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let command = |rt: &mut Runtime<Host>, c: char| {
        rt.key(KeyEvent {
            key: Key::Command(c),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        rt.frame(1.0 / 120.0);
    };
    // A click on a text puts focus in the document, where Ctrl+A goes.
    let text = rt
        .elements()
        .into_iter()
        .find(|e| e.path.contains("/doc-u") && e.path.ends_with("/t") && e.visible[2] > 0.0)
        .expect("a text on screen")
        .visible;
    let (x, y) = (text[0] + 2.0, text[1] + 4.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 120.0);
    let t = Instant::now();
    command(&mut rt, 'a');
    command(&mut rt, 'c');
    let model_ms = ms(t);
    let mut flagged = false;
    rt.update(|h, _| flagged = h.doc.all_selected());
    assert!(flagged, "Ctrl+A reached the document");
    let model_bytes = clip.text().map_or(0, |t| t.len());
    rt.update(|h, _| h.doc.select_all_off_for_test());
    let t = Instant::now();
    command(&mut rt, 'c');
    let rune_ms = ms(t);
    let rune_bytes = clip.text().map_or(0, |t| t.len());
    let want: usize = doc.units.iter().map(|u| u.copy_text.len()).sum();

    println!(
        "document 1 MiB, {n} units: first frame {first:.1} ms; scroll 600 frames p50 {p50:.2} ms \
         p95 {p95:.2} ms max {max:.2} ms, {slow} over 8 ms, {views} view runs; find {}; \
         Ctrl+A Ctrl+C from the model {model_ms:.1} ms, {model_bytes} of {want} bytes; through \
         Rune's collection {rune_ms:.1} ms, {rune_bytes} of {want} bytes",
        finds.join(", "),
    );
    assert_eq!(model_bytes, want, "the model copy is whole");
}

//! Perf on the work package's fixtures: the 50,000-line Rust file (`tests/fixtures/code`)
//! opened, jumped to its end and scrolled, with the real highlighter thread colouring it; and
//! a 5,000-line script edited in the editor. `#[ignore]`d; run with the `perf` profile and
//! Rune's commit budget, one at a time:
//!
//! ```sh
//! RUNE_COMMIT_BUDGET_MS=8 cargo test --profile perf -p strata-components --lib \
//!     code_view::tests::perf -- --ignored --test-threads=1 --nocapture
//! ```

use std::cell::Cell as StdCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rune::text::TextSystem;
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, clipboard};
use strata_kit::fonts;
use strata_kit::look::Look;
use strata_kit::theme::syntax::syntect_theme;

use super::super::*;
use crate::code_view::highlight::Highlighter;
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

fn summary(v: &mut [f64]) -> String {
    v.sort_by(f64::total_cmp);
    let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    format!(
        "p50 {:.2} p95 {:.2} max {:.2} ms, {} over 8 ms",
        p(0.5),
        p(0.95),
        v[v.len() - 1],
        v.iter().filter(|&&f| f > 8.0).count()
    )
}

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/../../tests/fixtures/code/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn text_system() -> TextSystem {
    TextSystem::new(fonts::FONTS, fonts::FAMILIES)
}

struct Host {
    code: CodeState,
    look: Look,
}

fn view(h: &Host, cx: &mut Cx<Host>) -> El {
    VIEWS.set(VIEWS.get() + 1);
    CodeView::new(|h: &Host| &h.code, |h: &mut Host| &mut h.code).build(h, &h.look, cx)
}

/// One frame with the host's poll, as an app runs it.
fn frame(rt: &mut Runtime<Host>, dt: f64) {
    rt.update_if(|h, tx| h.code.poll(tx));
    rt.frame(dt);
}

fn key(rt: &mut Runtime<Host>, key: Key, mods: Mods) -> f64 {
    let t = Instant::now();
    rt.key(KeyEvent { key, mods });
    frame(rt, 1.0 / 120.0);
    ms(t)
}

fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
}

fn preview(wrap: bool) {
    let text: Arc<str> = fixture("rust-50k.rs").into();
    let look = Look::fallback(false);
    let hl = Highlighter::new(Arc::new(syntect_theme(&look)));
    let mut first = f64::INFINITY;
    let mut rt = None;
    for _ in 0..3 {
        let mut code = CodeState::new(text.clone()).highlighter(&hl, Some("big.rs"), None);
        code.set_wrap(wrap);
        let host = Host {
            code,
            look: look.clone(),
        };
        let t = Instant::now();
        let mut r = Runtime::with_text(
            host,
            view,
            (700.0, 900.0),
            Env::new(false, 2.0),
            text_system(),
        );
        r.frame(0.0);
        first = first.min(ms(t));
        rt = Some(r);
    }
    let mut rt = rt.expect("a runtime");
    for _ in 0..30 {
        frame(&mut rt, 1.0 / 120.0);
    }
    // Focus the document, then Ctrl+End: the jump's frame and the ten after it.
    let t = rt
        .elements()
        .into_iter()
        .find(|e| e.path.contains("/code-l2/") && e.path.ends_with("/t"))
        .expect("row 2")
        .rect;
    rt.pointer_move(t[0] + 2.0, t[1] + 4.0);
    rt.pointer_down(t[0] + 2.0, t[1] + 4.0);
    rt.pointer_up(t[0] + 2.0, t[1] + 4.0);
    frame(&mut rt, 1.0 / 120.0);
    let mut jump = vec![key(&mut rt, Key::End, ctrl())];
    for _ in 0..10 {
        let t = Instant::now();
        frame(&mut rt, 1.0 / 120.0);
        jump.push(ms(t));
    }
    let at_end = rt.state.code.scroll_offset();
    key(&mut rt, Key::Home, ctrl());
    for _ in 0..10 {
        frame(&mut rt, 1.0 / 120.0);
    }

    // 600 wheel frames at 120 Hz, 48 pt each, colours landing as they come.
    VIEWS.set(0);
    let mut frames = Vec::new();
    for _ in 0..600 {
        let t = Instant::now();
        rt.wheel(350.0, 450.0, -48.0);
        frame(&mut rt, 1.0 / 120.0);
        frames.push(ms(t));
    }
    let views = VIEWS.get();

    // A thumb drag down the whole track over 120 frames.
    let bar = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("code-position"))
        .expect("the bar")
        .rect;
    let x = bar[0] + bar[2] / 2.0;
    let thumb = rt
        .elements()
        .into_iter()
        .find(|e| e.path.ends_with("code-position/trough/thumb"))
        .expect("a thumb")
        .rect;
    let mut y = thumb[1] + thumb[3] / 2.0;
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    frame(&mut rt, 1.0 / 120.0);
    let mut drag = Vec::new();
    for _ in 0..120 {
        y += bar[3] / 140.0;
        let t = Instant::now();
        rt.pointer_move(x, y);
        frame(&mut rt, 1.0 / 120.0);
        drag.push(ms(t));
    }
    rt.pointer_up(x, y);
    frame(&mut rt, 1.0 / 120.0);

    // Find over the whole text.
    let mut finds = Vec::new();
    for q in ["fn", "self", " ", "zzz_none"] {
        let mut total = 0;
        let t = Instant::now();
        rt.update(|h, _| {
            h.code.find(q);
            total = h.code.find_status().map_or(0, |s| s.total);
        });
        finds.push(format!("{q:?} {:.1} ms ({total})", ms(t)));
    }
    rt.update(|h, _| h.code.find_end());

    // Ctrl+A, then Ctrl+C from the model.
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let t = Instant::now();
    key(&mut rt, Key::Command('a'), ctrl());
    key(&mut rt, Key::Command('c'), ctrl());
    let copy = ms(t);
    let bytes = clip.text().map_or(0, |t| t.len());
    println!(
        "rust-50k.rs wrap {wrap}: first frame {first:.1} ms; Ctrl+End {} (offset {at_end:.0}); \
         wheel 600 frames {}, {views} view runs; thumb drag 120 frames {}; find {}; \
         Ctrl+A Ctrl+C {copy:.1} ms, {bytes} of {} bytes",
        summary(&mut jump),
        summary(&mut frames),
        summary(&mut drag),
        finds.join(", "),
        text.len(),
    );
    assert_eq!(bytes, text.len(), "the copy is whole");
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn a_50k_line_preview_wrapped() {
    release_only();
    preview(true);
}

#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn a_50k_line_preview_unwrapped() {
    release_only();
    preview(false);
}

struct Ed {
    editor: EditorState,
    look: Look,
}

fn editor_view(h: &Ed, _cx: &mut Cx<Ed>) -> El {
    CodeEditor::new(|h: &Ed| &h.editor, |h: &mut Ed| &mut h.editor).build(h, &h.look)
}

/// A keystroke's frame in the middle of the 5,000-line script, and the frame its colours
/// land in (budget: 16.7 ms at p95).
#[test]
#[ignore = "perf: run with --profile perf (see the module docs)"]
fn a_5000_line_script_edit() {
    release_only();
    let text = fixture("shell-5k.sh");
    let look = Look::fallback(false);
    let hl = Highlighter::new(Arc::new(syntect_theme(&look)));
    let mut editor = EditorState::new(text.as_str(), &hl, Some("s.sh"), None);
    editor.settle(Duration::from_secs(60));
    let mid = text.match_indices('\n').nth(2500).map_or(0, |(i, _)| i);
    editor.edit.set_caret(mid);
    editor.edit.focused = true;
    let host = Ed {
        editor,
        look: look.clone(),
    };
    let t = Instant::now();
    let mut rt = Runtime::with_text(
        host,
        editor_view,
        (900.0, 700.0),
        Env::new(false, 2.0),
        text_system(),
    );
    rt.frame(0.0);
    let first = ms(t);
    for _ in 0..10 {
        rt.update_if(|h, _| h.editor.poll());
        rt.frame(1.0 / 60.0);
    }
    let mut keys = Vec::new();
    let mut lands = Vec::new();
    for i in 0..120 {
        let ch = if i % 10 == 9 { " " } else { "x" };
        let t = Instant::now();
        rt.key(KeyEvent {
            key: Key::Text(ch.into()),
            mods: Mods::default(),
        });
        rt.update_if(|h, _| h.editor.poll());
        rt.frame(1.0 / 60.0);
        keys.push(ms(t));
        std::thread::sleep(Duration::from_millis(15));
        let t = Instant::now();
        if rt.update_if(|h, _| h.editor.poll()) {
            rt.frame(1.0 / 60.0);
            lands.push(ms(t));
        }
    }
    println!(
        "shell-5k.sh editor: first frame {first:.1} ms; keystroke {}; colours landing ({} frames) {}",
        summary(&mut keys),
        lands.len(),
        if lands.is_empty() {
            "none".into()
        } else {
            summary(&mut lands)
        },
    );
}

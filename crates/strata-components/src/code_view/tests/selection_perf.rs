// SPDX-License-Identifier: MIT

//! Regression tests for CodeView selection drag performance, O(log L) gutter-drag hit mapping,
//! lazy caret/all-selected tracking without `on_selection_change`, and deferred position-bar
//! `a11y_value` commits during active scroll.

use std::cell::Cell;
use std::rc::Rc;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, PhysicalInput, Runtime, TextStyle, clipboard};
use strata_kit::look::Look;

use super::super::caret::byte_at_x;
use super::super::render::Paint;
use super::super::*;

fn linear_byte_at_x(line: &str, x: f32, mono: TextStyle) -> usize {
    if x <= 0.0 || line.is_empty() {
        return 0;
    }
    let mut prev_end = 0;
    let mut prev_w = 0.0_f32;
    for (idx, ch) in line.char_indices() {
        let end = idx + ch.len_utf8();
        let w = rune::text::advance(&line[..end], mono);
        if x < (prev_w + w) * 0.5 {
            return prev_end;
        }
        prev_end = end;
        prev_w = w;
    }
    line.len()
}

#[test]
fn byte_at_x_binary_search_matches_linear_reference() {
    let look = Look::fallback(false);
    let mono = Paint::from_look(&look).mono;
    let samples = [
        "",
        "a",
        "the quick brown fox jumps over the lazy dog",
        "\tindented\twith\ttabs",
        "café naïve résumé — €100",
        "e\u{301} combine a\u{300}b",
        "日本語とASCIIと😀🎉混在ライン",
    ];
    for line in samples {
        let total = rune::text::advance(line, mono);
        let mut xs = vec![-5.0, 0.0, total, total + 25.0];
        let mut prev = 0.0_f32;
        for (idx, ch) in line.char_indices() {
            let w = rune::text::advance(&line[..idx + ch.len_utf8()], mono);
            let mid = (prev + w) * 0.5;
            xs.extend([prev, mid - 0.05, mid, mid + 0.05, w]);
            prev = w;
        }
        for x in xs {
            assert_eq!(
                byte_at_x(line, x, mono),
                linear_byte_at_x(line, x, mono),
                "line={line:?} x={x}"
            );
        }
    }
}

struct Host {
    code: CodeState,
    look: Look,
    views: Rc<Cell<usize>>,
}

fn view(h: &Host, cx: &mut Cx<Host>) -> El {
    h.views.set(h.views.get() + 1);
    CodeView::new(|h: &Host| &h.code, |h: &mut Host| &mut h.code).build(h, &h.look, cx)
}

fn runtime(text: String, poll: bool) -> (Runtime<Host>, Rc<Cell<usize>>) {
    let views = Rc::new(Cell::new(0));
    let mut rt = Runtime::new(
        Host {
            code: CodeState::new(text),
            look: Look::fallback(false),
            views: views.clone(),
        },
        view,
        (480.0, 240.0),
        Env::new(false, 1.0),
    );
    if poll {
        rt.poll = Some(|h, _, tx| h.code.poll(tx));
    }
    rt.frame(0.0);
    for _ in 0..16 {
        if !rt.frame(1.0 / 60.0) {
            break;
        }
    }
    (rt, views)
}

fn row_text_rect(rt: &Runtime<Host>, row: usize) -> [f32; 4] {
    let needle = format!("/code-l{row}/");
    rt.elements()
        .into_iter()
        .find(|e| e.path.contains(&needle) && e.path.ends_with("/t"))
        .unwrap_or_else(|| panic!("row {row} text not mounted"))
        .rect
}

fn send_cmd(rt: &mut Runtime<Host>, ch: char) {
    rt.key(KeyEvent {
        key: Key::Command(ch),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    });
    for _ in 0..8 {
        if !rt.frame(1.0 / 60.0) {
            break;
        }
    }
}

#[test]
fn gutter_drag_commits_zero_views_during_press_and_move() {
    let (mut rt, views) = runtime("the quick brown fox\nsecond line\n".into(), false);
    rt.update(|_, tx| tx.focus("code", false));
    rt.frame(1.0 / 60.0);
    // First drag runs with Rune's debug `live_only` verifier enabled to prove `gutter_drag`
    // mutates no view-visible state.
    rt.pointer_move(20.0, 14.0);
    rt.pointer_down(20.0, 14.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(100.0, 14.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(100.0, 14.0);
    rt.frame(1.0 / 60.0);

    // With the debug verifier off, the press focuses the document once and every subsequent
    // drag move and release commits zero views.
    rt.check_live_only(false);
    rt.pointer_move(20.0, 14.0);
    rt.pointer_down(20.0, 14.0);
    rt.frame(1.0 / 60.0);
    views.set(0);
    for step in 1..=6 {
        rt.pointer_move(20.0 + step as f32 * 20.0, 14.0);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(140.0, 14.0);
    rt.frame(1.0 / 60.0);

    assert_eq!(
        views.get(),
        0,
        "gutter drag moves and release must stay live-only with zero view commits"
    );
    let copied = rt
        .state
        .code
        .copy_keyboard()
        .expect("gutter drag selected text");
    assert!(copied.contains("quick"), "copied {copied:?}");
}

#[test]
fn offscreen_drag_commits_zero_views_and_ctrl_a_and_mouse_to_keyboard_work() {
    let text: String = (0..12_000)
        .map(|i| format!("line {i:05} alpha beta\n"))
        .collect();
    let (mut rt, views) = runtime(text.clone(), false);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);

    // 1. Start drag on row 0, wheel down so row 0 unmounts, then drag across lower visible rows.
    let r0 = row_text_rect(&rt, 0);
    rt.pointer_move(r0[0] + 0.5, r0[1] + 4.0);
    rt.pointer_down(r0[0] + 0.5, r0[1] + 4.0);
    rt.frame(1.0 / 60.0);
    for _ in 0..10 {
        rt.wheel(200.0, 120.0, -120.0);
        rt.frame(1.0 / 60.0);
    }
    views.set(0);
    for step in 0..6 {
        rt.pointer_move(r0[0] + 40.0 + step as f32 * 8.0, 120.0 + step as f32 * 8.0);
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(
        views.get(),
        0,
        "text selection drag moves must not trigger view commits"
    );
    rt.pointer_up(r0[0] + 80.0, 160.0);
    rt.frame(1.0 / 60.0);
    send_cmd(&mut rt, 'c');
    let dragged = clip.text().map(|t| t.to_string()).unwrap_or_default();
    assert!(dragged.starts_with("line 00000"), "dragged {dragged:?}");
    assert!(dragged.lines().count() > 20, "crossed unmounted rows");

    // 2. Ctrl+A -> Ctrl+C copies all 12,000 lines from the model, past Rune's 10,000-row cap.
    send_cmd(&mut rt, 'a');
    send_cmd(&mut rt, 'c');
    let all = clip.text().map(|t| t.to_string()).unwrap_or_default();
    assert_eq!(all.len(), text.len(), "Ctrl+A copies entire 12k-line text");

    // 3. Shift-click narrows the selection and clears all_selected.
    let visible_row = rt.state.code.visible.start + 1;
    let rv = row_text_rect(&rt, visible_row);
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_move(rv[0] + 30.0, rv[1] + 4.0);
    rt.pointer_down(rv[0] + 30.0, rv[1] + 4.0);
    rt.pointer_up(rv[0] + 30.0, rv[1] + 4.0);
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    rt.frame(1.0 / 60.0);
    send_cmd(&mut rt, 'c');
    let narrowed = clip.text().map(|t| t.to_string()).unwrap_or_default();
    assert!(narrowed.len() < text.len(), "Shift-click ended select-all");

    // 4. Mouse drag followed by Shift+Right extends from the mouse selection lazily.
    let r = row_text_rect(&rt, visible_row);
    rt.pointer_move(r[0] + 1.0, r[1] + 4.0);
    rt.pointer_down(r[0] + 1.0, r[1] + 4.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(r[0] + 28.0, r[1] + 4.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(r[0] + 28.0, r[1] + 4.0);
    rt.frame(1.0 / 60.0);
    let before = rt.selected_text().expect("mouse selection");
    rt.key(KeyEvent {
        key: Key::Right,
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
    });
    rt.frame(1.0 / 60.0);
    let extended = rt
        .state
        .code
        .copy_keyboard()
        .expect("Shift+Right extended mouse selection");
    assert!(
        extended.len() > before.len() && extended.starts_with(&before),
        "before={before:?} extended={extended:?}"
    );
}

#[test]
fn poll_defers_a11y_value_during_continuous_scroll_and_updates_when_settled() {
    let text: String = (0..600).map(|i| format!("line {i}\n")).collect();
    let (mut rt, views) = runtime(text, true);

    let bar_value = |rt: &Runtime<Host>| -> String {
        rt.accessibility_json()
            .to_string()
            .split("\"code-position\"")
            .nth(1)
            .and_then(|s| s.split("\"value\":\"").nth(1))
            .and_then(|s| s.split('"').next())
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(bar_value(&rt), "Line 1 of 600");

    // Continuously wheel-scroll across several percent of the file: no `said` view commits yet.
    views.set(0);
    for _ in 0..8 {
        rt.wheel(200.0, 120.0, -120.0);
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(
        views.get(),
        0,
        "continuous wheel scrolling must defer position-bar a11y_value commits"
    );

    // Once scrolling pauses past the settle window, a single commit updates `a11y_value`.
    rt.frame(0.2);
    rt.frame(1.0 / 60.0);
    assert_eq!(views.get(), 1, "exactly one settled a11y_value commit");
    let settled = bar_value(&rt);
    assert!(
        settled.starts_with("Line ") && settled != "Line 1 of 600",
        "settled value={settled:?}"
    );
}

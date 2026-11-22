// SPDX-License-Identifier: MIT

//! Regression tests for CodeView selection drag performance, whole-line gutter selection and
//! edge auto-scroll, lazy caret/all-selected tracking without `on_selection_change`, and deferred
//! position-bar `a11y_value` commits during active scroll.

use std::cell::Cell;
use std::rc::Rc;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, PhysicalInput, Runtime, clipboard};
use strata_kit::look::Look;

use super::super::*;

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

    // Explicit Ctrl+End / Ctrl+Home jumps update `a11y_value` immediately via `go_step`,
    // without waiting for the 150 ms settle window.
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    rt.update(|_, tx| tx.focus("code", true));
    rt.frame(1.0 / 60.0);
    rt.key(KeyEvent {
        key: Key::End,
        mods: ctrl,
    });
    for _ in 0..3 {
        rt.frame(1.0 / 60.0);
    }
    let at_end = bar_value(&rt);
    assert_ne!(at_end, settled, "Ctrl+End updates a11y_value immediately");
    assert_ne!(at_end, "Line 1 of 600");

    rt.key(KeyEvent {
        key: Key::Home,
        mods: ctrl,
    });
    for _ in 0..3 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(
        bar_value(&rt),
        "Line 1 of 600",
        "Ctrl+Home updates a11y_value immediately"
    );
}

#[test]
fn external_selection_clear_invalidates_stale_keyboard_and_select_all_state() {
    use rune::{col, el};

    struct OutsideHost {
        code: CodeState,
        look: Look,
    }
    fn outside_view(h: &OutsideHost, cx: &mut Cx<OutsideHost>) -> El {
        col()
            .w_full()
            .h_full()
            .child(
                el().key("outside")
                    .w_full()
                    .h(30.0)
                    .focusable(true)
                    .on_press(|_: &mut OutsideHost| {}),
            )
            .child(
                col().w_full().grow(1.0).min_h(0.0).child(
                    CodeView::new(|h: &OutsideHost| &h.code, |h: &mut OutsideHost| &mut h.code)
                        .build(h, &h.look, cx),
                ),
            )
    }

    let mut rt = Runtime::new(
        OutsideHost {
            code: CodeState::new("alpha\nbeta\ngamma\n"),
            look: Look::fallback(false),
        },
        outside_view,
        (400.0, 240.0),
        Env::new(false, 1.0),
    );
    for _ in 0..8 {
        rt.frame(1.0 / 60.0);
    }
    let click_outside = |rt: &mut Runtime<OutsideHost>| {
        rt.pointer_move(20.0, 15.0);
        rt.pointer_down(20.0, 15.0);
        rt.pointer_up(20.0, 15.0);
        for _ in 0..4 {
            rt.frame(1.0 / 60.0);
        }
    };
    let focus_code = |rt: &mut Runtime<OutsideHost>| {
        rt.update(|_, tx| tx.focus("code", true));
        rt.frame(1.0 / 60.0);
    };
    let send = |rt: &mut Runtime<OutsideHost>, key: Key, mods: Mods| {
        rt.key(KeyEvent { key, mods });
        rt.frame(1.0 / 60.0);
    };
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    let cmd = Mods {
        cmd: true,
        ..Mods::default()
    };
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };

    // 1. Shift+Down creates a keyboard selection; clicking outside clears Rune's selection.
    focus_code(&mut rt);
    send(&mut rt, Key::Home, ctrl);
    send(&mut rt, Key::Down, shift);
    assert_eq!(rt.state.code.copy_keyboard().as_deref(), Some("alpha\n"));
    click_outside(&mut rt);
    assert_eq!(rt.selected_text(), None);
    assert_eq!(rt.state.code.keyboard_selection_for(None), None);

    // Returning focus via keyboard: plain Left must not collapse a dead selection, and
    // Shift+Right must start fresh at offset 0 rather than resurrecting `alpha\n`.
    focus_code(&mut rt);
    send(&mut rt, Key::Left, Mods::default());
    assert_eq!(rt.state.code.keyboard_selection(), None);
    send(&mut rt, Key::Right, shift);
    assert_eq!(rt.state.code.copy_keyboard().as_deref(), Some("a"));

    // 2. Ctrl+A selects all; clicking outside clears Rune's selection. Returning focus and
    // pressing Shift+Left must not resurrect/shrink the cleared 0..len range.
    send(&mut rt, Key::Command('a'), cmd);
    assert!(rt.state.code.all_selected());
    click_outside(&mut rt);
    assert_eq!(rt.selected_text(), None);

    focus_code(&mut rt);
    send(&mut rt, Key::Left, shift);
    assert!(!rt.state.code.all_selected());
    assert_eq!(rt.state.code.copy_keyboard(), None);
}

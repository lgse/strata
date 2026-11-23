// SPDX-License-Identifier: MIT

use std::cell::Cell;
use std::rc::Rc;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, PhysicalInput, Runtime, clipboard, col, el};
use strata_kit::look::Look;

use super::super::*;

struct Host {
    code: CodeState,
    look: Look,
    views: Rc<Cell<usize>>,
    poll_changes: Rc<Cell<usize>>,
}

fn view(h: &Host, cx: &mut Cx<Host>) -> El {
    h.views.set(h.views.get() + 1);
    CodeView::new(|h: &Host| &h.code, |h: &mut Host| &mut h.code).build(h, &h.look, cx)
}

fn runtime(
    text: String,
    wrap: bool,
    size: (f32, f32),
    poll: bool,
) -> (Runtime<Host>, Rc<Cell<usize>>) {
    let views = Rc::new(Cell::new(0));
    let poll_changes = Rc::new(Cell::new(0));
    let mut code = CodeState::new(text);
    code.set_wrap(wrap);
    let mut rt = Runtime::new(
        Host {
            code,
            look: Look::fallback(false),
            views: views.clone(),
            poll_changes,
        },
        view,
        size,
        Env::new(false, 1.0),
    );
    if poll {
        rt.poll = Some(|h, _, tx| {
            let changed = h.code.poll(tx);
            if changed {
                h.poll_changes.set(h.poll_changes.get() + 1);
            }
            changed
        });
    }
    rt.frame(0.0);
    for _ in 0..16 {
        if !rt.frame(1.0 / 60.0) {
            break;
        }
    }
    (rt, views)
}

fn row_rect(rt: &Runtime<Host>, row: usize) -> [f32; 4] {
    let needle = format!("/code-l{row}/");
    rt.elements()
        .into_iter()
        .find(|e| e.path.contains(&needle) && e.path.ends_with("/n"))
        .unwrap_or_else(|| panic!("gutter cell for row {row} not mounted"))
        .rect
}

fn row_text_rect(rt: &Runtime<Host>, row: usize) -> [f32; 4] {
    let needle = format!("/code-l{row}/");
    rt.elements()
        .into_iter()
        .find(|e| e.path.contains(&needle) && e.path.ends_with("/t"))
        .unwrap_or_else(|| panic!("text cell for row {row} not mounted"))
        .rect
}

fn row_center_y(rt: &Runtime<Host>, row: usize) -> f32 {
    let r = row_rect(rt, row);
    r[1] + r[3] * 0.5
}

fn gutter_x(rt: &Runtime<Host>) -> f32 {
    let r = row_rect(rt, rt.state.code.visible.start);
    r[0] + r[2] * 0.4
}

fn send_copy(rt: &mut Runtime<Host>) {
    rt.key(KeyEvent {
        key: Key::Command('c'),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    });
    for _ in 0..6 {
        if !rt.frame(1.0 / 60.0) {
            break;
        }
    }
}

fn copied_text(clip: &clipboard::MemoryClipboard) -> Option<String> {
    clip.text().map(|t| t.to_string())
}

#[test]
fn gutter_click_focuses_selects_whole_line_empty_line_and_chunked_continuation() {
    let long_line = "abcd".repeat(1250);
    let text = format!("alpha\n\n{long_line}\nomega\n");
    let (mut rt, _) = runtime(text, false, (480.0, 240.0), false);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);

    let gx = gutter_x(&rt);
    let y0 = row_center_y(&rt, 0);
    rt.pointer_move(gx, y0);
    rt.pointer_down(gx, y0);
    rt.pointer_up(gx, y0);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((0, 0)));
    let (sel_path, sel_range) = rt.text_selection().expect("gutter sentinel selected");
    assert!(
        sel_path.ends_with("/code-gutter-sel") && sel_range == (0..1),
        "gutter selection must use transparent sentinel instead of double-painting a code row: {sel_path:?} {sel_range:?}"
    );
    send_copy(&mut rt);
    assert_eq!(copied_text(&clip).as_deref(), Some("alpha\n"));

    // 2. Clicking the empty line (row 1, line 1) selects "\n" and Ctrl+C copies "\n".
    let y1 = row_center_y(&rt, 1);
    rt.pointer_move(gx, y1);
    rt.pointer_down(gx, y1);
    rt.pointer_up(gx, y1);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((1, 1)));
    send_copy(&mut rt);
    assert_eq!(copied_text(&clip).as_deref(), Some("\n"));

    // 3. Clicking a continuation "↳" row (row 3) of chunked line 2 selects the full 5,000-byte line.
    assert!(rt.state.code.rows().row(3).cont);
    assert_eq!(rt.state.code.rows().row(3).line, 2);
    let y3 = row_center_y(&rt, 3);
    rt.pointer_move(gx, y3);
    rt.pointer_down(gx, y3);
    rt.pointer_up(gx, y3);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((2, 2)));
    send_copy(&mut rt);
    assert_eq!(
        copied_text(&clip).as_deref(),
        Some(format!("{long_line}\n").as_str())
    );
}

#[test]
fn gutter_vertical_drag_down_shrink_up_and_wrapped_rows_commit_zero_views() {
    let wrapped_line = "the quick brown fox jumps over the lazy dog repeatedly to wrap across several visual lines";
    let text = format!("line 0\n{wrapped_line}\nline 2\nline 3\nline 4\nline 5\n");
    for wrap in [false, true] {
        let (mut rt, views) = runtime(text.clone(), wrap, (260.0, 240.0), false);
        let clip = clipboard::memory();
        rt.use_clipboard(&clip);
        rt.update(|_, tx| tx.focus("code", false));
        rt.frame(1.0 / 60.0);
        rt.check_live_only(false);

        let gx = gutter_x(&rt);
        let y0 = row_center_y(&rt, 0);
        let y1_bottom = {
            let r = row_rect(&rt, 1);
            r[1] + r[3] - 2.0
        };
        let y2 = row_center_y(&rt, 2);
        let y3 = row_center_y(&rt, 3);
        let y4 = row_center_y(&rt, 4);

        views.set(0);
        // Press line 2, drag downward to line 4 while drifting horizontally into code pane.
        rt.pointer_move(gx, y2);
        rt.pointer_down(gx, y2);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((2, 2)));
        assert_eq!(rt.state.code.copy_keyboard().as_deref(), Some("line 2\n"));

        rt.pointer_move(180.0, y4);
        rt.frame(1.0 / 60.0);
        assert_eq!(
            rt.state.code.gutter_highlighted_lines(),
            Some((2, 4)),
            "wrap={wrap}: highlight stays strictly on lines 2..=4"
        );
        assert_eq!(
            rt.state.code.copy_keyboard().as_deref(),
            Some("line 2\nline 3\nline 4\n")
        );

        // Drag back upward toward anchor line 2 -> shrinks smoothly.
        rt.pointer_move(-10.0, y3);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((2, 3)));
        assert_eq!(
            rt.state.code.copy_keyboard().as_deref(),
            Some("line 2\nline 3\n")
        );

        rt.pointer_move(gx, y2);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((2, 2)));
        assert_eq!(rt.state.code.copy_keyboard().as_deref(), Some("line 2\n"));

        // Continue dragging upward past anchor into the bottom of wrapped line 1 and line 0.
        rt.pointer_move(gx, y1_bottom);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((1, 2)));
        assert_eq!(
            rt.state.code.copy_keyboard().as_deref(),
            Some(format!("{wrapped_line}\nline 2\n").as_str())
        );

        rt.pointer_move(gx, y0);
        rt.frame(1.0 / 60.0);
        rt.pointer_up(gx, y0);
        rt.frame(1.0 / 60.0);

        assert_eq!(
            views.get(),
            0,
            "wrap={wrap}: gutter press, vertical drags, and release must commit zero views"
        );
        assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((0, 2)));
        send_copy(&mut rt);
        assert_eq!(
            copied_text(&clip).as_deref(),
            Some(format!("line 0\n{wrapped_line}\nline 2\n").as_str())
        );
    }
}

#[test]
fn gutter_shift_click_extends_from_gutter_and_text_selections() {
    let text = "zero\none\ntwo\nthree\nfour\nfive\n".to_owned();
    let (mut rt, _) = runtime(text, false, (360.0, 240.0), false);
    let gx = gutter_x(&rt);

    // 1. Click gutter line 2, Shift+click gutter line 4 -> selects lines 2..=4.
    let y2 = row_center_y(&rt, 2);
    let y4 = row_center_y(&rt, 4);
    let y0 = row_center_y(&rt, 0);
    rt.pointer_move(gx, y2);
    rt.pointer_down(gx, y2);
    rt.pointer_up(gx, y2);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.code.copy_keyboard().as_deref(), Some("two\n"));

    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_move(gx, y4);
    rt.pointer_down(gx, y4);
    rt.pointer_up(gx, y4);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((2, 4)));
    assert_eq!(
        rt.state.code.copy_keyboard().as_deref(),
        Some("two\nthree\nfour\n")
    );

    // 2. Shift+click above anchor (line 0) -> flips selection to lines 0..=2.
    rt.pointer_move(gx, y0);
    rt.pointer_down(gx, y0);
    rt.pointer_up(gx, y0);
    rt.frame(1.0 / 60.0);
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((0, 2)));
    assert_eq!(
        rt.state.code.copy_keyboard().as_deref(),
        Some("zero\none\ntwo\n")
    );

    // 3. Regular text drag on line 1, then Shift+click gutter line 3 -> extends to lines 1..=3.
    let r1 = row_text_rect(&rt, 1);
    rt.pointer_move(r1[0] + 1.0, r1[1] + 4.0);
    rt.pointer_down(r1[0] + 1.0, r1[1] + 4.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(r1[0] + 20.0, r1[1] + 4.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(r1[0] + 20.0, r1[1] + 4.0);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.code.gutter_highlighted_lines(), None);

    let y3 = row_center_y(&rt, 3);
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_move(gx, y3);
    rt.pointer_down(gx, y3);
    rt.pointer_up(gx, y3);
    rt.frame(1.0 / 60.0);
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((1, 3)));
    assert_eq!(
        rt.state.code.copy_keyboard().as_deref(),
        Some("one\ntwo\nthree\n")
    );
}

#[test]
fn gutter_drag_edge_autoscrolls_down_and_up_and_stops_cleanly_with_zero_idle_wakes() {
    let text: String = (0..200).map(|i| format!("line {i:03}\n")).collect();
    let (mut rt, _) = runtime(text, true, (400.0, 200.0), true);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    rt.update(|_, tx| tx.focus("code", false));
    rt.frame(1.0 / 60.0);
    rt.check_live_only(false);

    let gx = gutter_x(&rt);
    let y1 = row_center_y(&rt, 1);
    let initial_visible_end = rt.state.code.visible.end;

    // 1. Start on line 1, drag below the viewport bottom edge -> auto-scrolls and extends selection.
    rt.state.poll_changes.set(0);
    rt.pointer_move(gx, y1);
    rt.pointer_down(gx, y1);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(gx, 280.0);
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(
        rt.state.poll_changes.get(),
        0,
        "edge auto-scroll poll ticks must not request view rebuilds"
    );
    let offset_after_down = rt.state.code.scroll_offset();
    assert!(
        offset_after_down > 100.0,
        "expected downward auto-scroll, offset={offset_after_down}"
    );
    let (lo, hi) = rt
        .state
        .code
        .gutter_highlighted_lines()
        .expect("gutter selection active");
    assert_eq!(lo, 1);
    assert!(
        hi > initial_visible_end + 5,
        "selection should extend to off-screen lines: hi={hi} initial_visible_end={initial_visible_end}"
    );

    // 2. Release stops auto-scroll immediately and leaves zero idle wakes once settled.
    rt.pointer_up(gx, 280.0);
    rt.frame(1.0 / 60.0);
    rt.frame(1.0 / 60.0);
    let settled_offset = rt.state.code.scroll_offset();
    rt.frame(0.2);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.code.scroll_offset(), settled_offset);
    assert_eq!(rt.wake_deadline(), None, "no idle wake after release");
    assert!(!rt.frame(1.0 / 60.0), "window rests idle after release");

    let (final_lo, final_hi) = rt
        .state
        .code
        .gutter_highlighted_lines()
        .expect("selection kept after release");
    assert_eq!(final_lo, 1);
    assert!(final_hi >= hi);
    send_copy(&mut rt);
    let copied = clip.text().expect("copied lines").to_string();
    assert!(copied.starts_with("line 001\n"), "copied={copied:?}");
    assert!(
        copied.ends_with(&format!("line {final_hi:03}\n")),
        "copied end mismatch for line {final_hi}"
    );

    // 3. Start on a lower visible line, drag above the viewport top edge -> auto-scrolls upward.
    let start_row = rt.state.code.visible.start + 2;
    let y_start = row_center_y(&rt, start_row);
    let before_up = rt.state.code.scroll_offset();
    rt.pointer_move(gx, y_start);
    rt.pointer_down(gx, y_start);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(gx, -80.0);
    for _ in 0..15 {
        rt.frame(1.0 / 60.0);
    }
    let after_up = rt.state.code.scroll_offset();
    assert!(
        after_up < before_up - 80.0 && after_up > 80.0,
        "expected mid-document upward auto-scroll: before={before_up} after={after_up}"
    );
    let (up_lo, up_hi) = rt
        .state
        .code
        .gutter_highlighted_lines()
        .expect("upward gutter selection active");
    assert_eq!(up_hi, start_row);
    assert!(
        up_lo + 5 < start_row,
        "upward selection should extend to earlier off-screen lines: {up_lo}..={up_hi}"
    );

    // Dragging back inside viewport stops auto-scrolling mid-document while keeping drag active.
    rt.pointer_move(gx, 100.0);
    rt.frame(1.0 / 60.0);
    let mid_offset = rt.state.code.scroll_offset();
    assert!(mid_offset > 50.0, "still mid-document: {mid_offset}");
    for _ in 0..10 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(
        rt.state.code.scroll_offset(),
        mid_offset,
        "moving back inside viewport halts auto-scroll"
    );

    rt.pointer_up(gx, 100.0);
    rt.frame(1.0 / 60.0);
    rt.frame(1.0 / 60.0);
    rt.frame(0.2);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.wake_deadline(), None, "no idle wake after upward drag");
    assert!(!rt.frame(1.0 / 60.0), "window rests idle after upward drag");
}

#[test]
fn external_click_clears_gutter_highlight_on_poll_without_committing() {
    struct OutsideHost {
        code: CodeState,
        look: Look,
        views: Rc<Cell<usize>>,
    }
    fn outside_view(h: &OutsideHost, cx: &mut Cx<OutsideHost>) -> El {
        h.views.set(h.views.get() + 1);
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

    let views = Rc::new(Cell::new(0));
    let mut rt = Runtime::new(
        OutsideHost {
            code: CodeState::new("alpha\n\nbeta\ngamma\n"),
            look: Look::fallback(false),
            views: views.clone(),
        },
        outside_view,
        (400.0, 240.0),
        Env::new(false, 1.0),
    );
    rt.poll = Some(|h, _, tx| h.code.poll(tx));
    for _ in 0..8 {
        rt.frame(1.0 / 60.0);
    }
    let gutter_rect = |rt: &Runtime<OutsideHost>, row: usize| {
        let needle = format!("/code-l{row}/");
        rt.elements()
            .into_iter()
            .find(|e| e.path.contains(&needle) && e.path.ends_with("/n"))
            .unwrap_or_else(|| panic!("row {row} gutter"))
            .rect
    };
    let r0 = gutter_rect(&rt, 0);
    let r1 = gutter_rect(&rt, 1);
    let r2 = gutter_rect(&rt, 2);
    let gx = r0[0] + r0[2] * 0.4;

    // 1. Multi-line gutter drag cleared by external click.
    rt.pointer_move(gx, r0[1] + r0[3] * 0.5);
    rt.pointer_down(gx, r0[1] + r0[3] * 0.5);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(gx, r2[1] + r2[3] * 0.5);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(gx, r2[1] + r2[3] * 0.5);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((0, 2)));

    rt.pointer_move(20.0, 15.0);
    rt.pointer_down(20.0, 15.0);
    rt.pointer_up(20.0, 15.0);
    for _ in 0..4 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(rt.state.code.gutter_highlighted_lines(), None);
    assert_eq!(rt.state.code.keyboard_selection(), None);

    // 2. Single empty-line ("\n") gutter selection also clears on external click via sentinel.
    rt.pointer_move(gx, r1[1] + r1[3] * 0.5);
    rt.pointer_down(gx, r1[1] + r1[3] * 0.5);
    rt.pointer_up(gx, r1[1] + r1[3] * 0.5);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.code.gutter_highlighted_lines(), Some((1, 1)));
    assert_eq!(rt.state.code.copy_keyboard().as_deref(), Some("\n"));

    rt.pointer_move(20.0, 15.0);
    rt.pointer_down(20.0, 15.0);
    rt.pointer_up(20.0, 15.0);
    for _ in 0..4 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(rt.state.code.gutter_highlighted_lines(), None);
    assert_eq!(rt.state.code.keyboard_selection(), None);
}

#[test]
fn suspend_modal_or_hiding_gutter_mid_autoscroll_stops_wakes_cleanly() {
    struct SuspendHost {
        code: CodeState,
        look: Look,
        mounted: bool,
        modal_open: bool,
    }
    fn suspend_view(h: &SuspendHost, cx: &mut Cx<SuspendHost>) -> El {
        let mut root = col().w_full().h_full();
        if h.mounted {
            root = root.child(
                CodeView::new(|s: &SuspendHost| &s.code, |s: &mut SuspendHost| &mut s.code)
                    .build(h, &h.look, cx),
            );
        }
        if h.modal_open {
            root = root.child(
                el().key("modal")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .focusable(true)
                    .modal(true, |s: &mut SuspendHost| s.modal_open = false),
            );
        }
        root
    }

    let text: String = (0..200).map(|i| format!("line {i:03}\n")).collect();
    let start_autoscroll = || {
        let mut rt = Runtime::new(
            SuspendHost {
                code: CodeState::new(text.clone()),
                look: Look::fallback(false),
                mounted: true,
                modal_open: false,
            },
            suspend_view,
            (400.0, 200.0),
            Env::new(false, 1.0),
        );
        rt.poll = Some(|h, _, tx| h.code.poll(tx));
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        let r1 = rt
            .elements()
            .into_iter()
            .find(|e| e.path.contains("/code-l1/") && e.path.ends_with("/n"))
            .expect("row 1 gutter")
            .rect;
        let gx = r1[0] + r1[2] * 0.4;
        let y1 = r1[1] + r1[3] * 0.5;
        rt.pointer_move(gx, y1);
        rt.pointer_down(gx, y1);
        rt.frame(1.0 / 60.0);
        rt.pointer_move(gx, 280.0);
        for _ in 0..8 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.state.code.scroll_offset() > 20.0);
        assert!(rt.wake_deadline().is_some());
        rt
    };

    let assert_stopped_idle = |rt: &mut Runtime<SuspendHost>, label: &str| {
        for _ in 0..4 {
            rt.frame(1.0 / 60.0);
        }
        let stopped_at = rt.state.code.scroll_offset();
        for _ in 0..5 {
            rt.frame(0.2);
        }
        rt.frame(1.0 / 60.0);
        assert_eq!(
            rt.state.code.scroll_offset(),
            stopped_at,
            "{label}: scroll offset must stay stopped"
        );
        assert_eq!(
            rt.wake_deadline(),
            None,
            "{label}: must not leave pending auto-scroll wakes"
        );
        assert!(!rt.frame(1.0 / 60.0), "{label}: must rest idle");
    };

    // 1. Unmounting/suspending CodeView mid-auto-scroll while poll keeps running stops wakes.
    let mut rt = start_autoscroll();
    rt.update(|s, _| s.mounted = false);
    assert_stopped_idle(&mut rt, "suspended view");

    // 2. Opening a modal mid-auto-scroll stops wakes cleanly.
    let mut rt = start_autoscroll();
    rt.update(|s, tx| {
        s.modal_open = true;
        tx.focus("modal", true);
    });
    assert_stopped_idle(&mut rt, "modal open");

    // 3. Hiding line numbers mid-auto-scroll stops wakes cleanly.
    let mut rt = start_autoscroll();
    rt.update(|s, _| s.code.set_line_numbers(false));
    assert_stopped_idle(&mut rt, "hiding line numbers");
}

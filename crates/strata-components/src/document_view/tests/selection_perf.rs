// SPDX-License-Identifier: MIT

//! Regression tests for DocumentView selection drag performance and lazy `all_selected`
//! invalidation without `on_selection_change`.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, clipboard};
use strata_kit::look::Look;

use super::super::*;
use crate::preview_drawer::HostedPreview;

struct Host {
    doc: DocumentState,
    look: Look,
    views: Rc<Cell<usize>>,
}

fn view(h: &Host, cx: &mut Cx<Host>) -> El {
    h.views.set(h.views.get() + 1);
    DocumentView::new(|h: &Host| &h.doc, |h: &mut Host| &mut h.doc).build(h, &h.look, cx)
}

fn settle(rt: &mut Runtime<Host>) {
    for _ in 0..32 {
        if !rt.frame(1.0 / 60.0) {
            break;
        }
    }
}

fn unit_rect(rt: &Runtime<Host>, unit: usize) -> [f32; 4] {
    let needle = format!("/doc-u{unit}/");
    rt.elements()
        .into_iter()
        .find(|e| e.path.contains(&needle) && e.path.ends_with("/t"))
        .unwrap_or_else(|| panic!("unit {unit} text not mounted"))
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
    settle(rt);
}

#[test]
fn offscreen_anchor_drag_commits_zero_views_and_find_step_clears_select_all() {
    let units: Vec<Unit> = (0..500)
        .map(|i| Unit::new(UnitKind::Paragraph, format!("paragraph {i} needle text")))
        .collect();
    let views = Rc::new(Cell::new(0));
    let mut rt = Runtime::new(
        Host {
            doc: DocumentState::new(Arc::new(Document::new(units))),
            look: Look::fallback(false),
            views: views.clone(),
        },
        view,
        (640.0, 320.0),
        Env::new(false, 1.0),
    );
    rt.poll = Some(|h, t, tx| h.doc.tick(t) | h.doc.flush(tx));
    rt.frame(0.0);
    settle(&mut rt);

    let clip = clipboard::memory();
    rt.use_clipboard(&clip);

    // Start drag on unit 0, scroll down until unit 0 unmounts, then drag across lower rows.
    let u0 = unit_rect(&rt, 0);
    rt.pointer_move(u0[0] + 2.0, u0[1] + 6.0);
    rt.pointer_down(u0[0] + 2.0, u0[1] + 6.0);
    rt.frame(1.0 / 60.0);
    for _ in 0..12 {
        rt.wheel(300.0, 160.0, -160.0);
        rt.frame(1.0 / 60.0);
    }
    views.set(0);
    for step in 0..6 {
        rt.pointer_move(
            u0[0] + 40.0 + step as f32 * 10.0,
            140.0 + step as f32 * 10.0,
        );
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(
        views.get(),
        0,
        "document selection drag moves must not trigger view commits"
    );
    rt.pointer_up(u0[0] + 90.0, 190.0);
    settle(&mut rt);
    send_cmd(&mut rt, 'c');
    let copied = clip.text().map(|t| t.to_string()).unwrap_or_default();
    assert!(copied.starts_with("paragraph 0 "), "copied {copied:?}");
    assert!(copied.lines().count() > 10, "crossed unmounted units");

    // Ctrl+A arms all_selected; stepping find clears it.
    rt.update(|h, _| h.doc.find("needle"));
    settle(&mut rt);
    send_cmd(&mut rt, 'a');
    assert!(rt.state.doc.all_selected());
    rt.update(|h, _| h.doc.find_step(true));
    settle(&mut rt);
    assert!(!rt.state.doc.all_selected());
    assert_eq!(rt.selected_text().as_deref(), Some("needle"));
}

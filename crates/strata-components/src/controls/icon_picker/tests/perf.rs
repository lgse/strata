// SPDX-License-Identifier: MIT

use std::rc::Rc;
use std::time::Instant;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col};
use strata_kit::look::Look;

use super::super::{DEFAULT_VERSION, IconPicker, IconPickerState, PickerSource};

const FRAME: f64 = 1.0 / 120.0;
const BUDGET: f64 = 1000.0 / 60.0;

struct Host {
    st: IconPickerState,
    look: Look,
}

fn picker() -> IconPicker<Host> {
    IconPicker::new(
        "p",
        |h: &Host| &h.st,
        |h: &mut Host| &mut h.st,
        |h: &Host| &h.look,
    )
}

fn view(h: &Host, _: &mut Cx<Host>) -> El {
    col().child(picker().build(h, &h.look))
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn p95(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * 0.95).round() as usize]
}

fn key(rt: &mut Runtime<Host>, key: Key, mods: Mods) -> f64 {
    let t = Instant::now();
    rt.key(KeyEvent { key, mods });
    rt.frame(FRAME);
    ms(t)
}

#[test]
#[ignore = "perf: run with --profile perf -- --ignored --nocapture"]
fn typing_switching_and_paging_stay_within_a_frame() {
    let t = Instant::now();
    let emoji = PickerSource::emoji(DEFAULT_VERSION);
    let parse = ms(t);
    let t = Instant::now();
    let source = PickerSource::catalog(&strata_kit::icons::lucide::SET).chain(emoji);
    let index = ms(t);
    let items = source.items().len();
    let host = Host {
        st: IconPickerState::new(Rc::new(source)),
        look: Look::fallback(false),
    };
    let mut rt = Runtime::new(host, view, (900.0, 700.0), Env::new(false, 2.0));
    rt.frame(0.0);
    let field = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("p-field"))
        .expect("the field");
    let (x, y) = (field.rect[0] + 20.0, field.rect[1] + field.rect[3] / 2.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(FRAME);
    rt.pointer_up(x, y);
    rt.frame(FRAME);
    let plain = Mods::default();

    let mut typing = Vec::new();
    for _ in 0..5 {
        for c in ["s", "m", "i", "l", "i", "n", "g", " ", "f", "a", "c", "e"] {
            typing.push(key(&mut rt, Key::Text(c.into()), plain));
        }
        for _ in 0..12 {
            typing.push(key(&mut rt, Key::Backspace, plain));
        }
    }
    assert_eq!(rt.state.st.query(), "");

    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    let tabs = rt.state.st.tabs().len();
    let mut switching = Vec::new();
    for _ in 0..2 * tabs {
        switching.push(key(&mut rt, Key::Text("n".into()), ctrl));
    }

    // People & Body, from the field: Tab into the grid, then a page at a time to its end.
    let people = rt
        .state
        .st
        .tabs()
        .into_iter()
        .position(|p| matches!(p, super::super::Page::Category(3)))
        .expect("People & Body");
    for _ in 0..people {
        key(&mut rt, Key::Text("n".into()), ctrl);
    }
    key(&mut rt, Key::Tab, plain);
    let mut paging = Vec::new();
    for _ in 0..30 {
        paging.push(key(&mut rt, Key::PageDown, plain));
    }
    let atlas: usize = rt.images.pages.iter().map(|p| p.data.len()).sum();

    let (type95, switch95, page95) = (p95(&mut typing), p95(&mut switching), p95(&mut paging));
    eprintln!(
        "C20d perf, {items} choices: emoji parse {parse:.2} ms, index {index:.2} ms; \
         keystroke + frame p95 {type95:.2} ms (n {}), tab switch p95 {switch95:.2} ms, \
         page p95 {page95:.2} ms; image atlas {} pages, {:.1} MiB",
        typing.len(),
        rt.images.pages.len(),
        atlas as f64 / (1024.0 * 1024.0),
    );
    assert!(type95 <= 4.0, "keystroke p95 {type95:.2} ms");
    assert!(switch95 <= BUDGET, "tab switch p95 {switch95:.2} ms");
    assert!(page95 <= BUDGET, "page p95 {page95:.2} ms");
}

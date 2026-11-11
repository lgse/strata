# 27. A scroll request past the mounted rows runs the view; the wheel doesn't

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C24-8 |
| Affects | C24 (scroll bars), rune-kit's own `ScrollBar` |
| Retires | C24's allowance for Rune's "stepping its own glide" warning on a thumb drag |
| Checked | Re-checked at `2a44156` with a new headless probe, `rv_27` (the program in the body): `scroll_to_instant` and `scroll_to_with` past the mounted rows of 1,000 virtual rows commit 4 times and run the view 3 times a frame, where a wheel moving as far commits once and runs it none; all three land on the same row. In the gallery, `mise run test a_thumb_drag_trips_only_runes_instant_scroll_warning` passed (1 test: C24's thumb drag over 10,000 rows trips Rune's instant-scroll warning on the list's viewport, and nothing else). C24's allowance is still there (`scroll/mod.rs:204–209`, `strata-gallery/tests/c24.rs:833–848`), and C10b cites it (`code_view/mod.rs:132`). Earlier: C24's review probes at the pin (the table), not re-run at `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A scroll request that moves a virtual list past its mounted rows commits several times and runs the view; a wheel as far doesn't

Context: Strata (lgse/strata), a file manager on Rune.

A thumb drag on a long virtual list asks for a scroll on every move. Each request that moves
past the mounted rows commits about four times and runs the app's view about three times that
frame, where a wheel delta moving as far commits once and runs no view. Over a long list the
commits trip Rune's own warning ("an instant scroll … on 8 frames in a row, each with a
commit"), and rune-kit's `ScrollBar` trips it the same way.

Measured at `5d421fd`, per frame:

| What moves the list | Commits | View runs |
| --- | --- | --- |
| Our scroll bar's thumb drag | +4 | 3 |
| rune-kit `ScrollBar`, thumb drag | +4 | 3 |
| Wheel jumps of up to 6,000 pt | +1 | 0 |
| `scroll_to_instant` of 400 pt or more | 2–3 | 2–3 |
| `scroll_to_with` past the mounted rows | +4 | 3 |

**Repro** (headless; ran at `2a44156`): 1,000 virtual rows; each frame, ask for a scroll 1,250 pt
further (a thumb moving 25 pt over a 400 pt track), counting `Runtime::stats.commits` and the
view's runs; then wheel the same distances.

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, Spring, col, el};
use std::cell::Cell;
#[derive(Default)]
struct S { views: Cell<u32> }
fn view(s: &S, _: &mut Cx<S>) -> El {
    s.views.set(s.views.get() + 1);
    col().w_full().h_full().child(
        col().key("vp").size(300.0, 400.0).scroll(ScrollSpec { stick_bottom: false }).child(
            col().key("list").w_full().growth(GrowthSpec::default())
                .virtual_rows(1000, 20.0, |_: &S, i| format!("r{i}").into(), |_: &S, _| el().w_full().h(20.0))))
}
fn rt() -> Runtime<S> {
    let mut rt = Runtime::new(S::default(), view, (300.0, 400.0), Env::new(true, 1.0));
    rt.frame(0.0);
    for _ in 0..10 { rt.frame(0.016); }
    rt
}
fn per_frame(name: &str, steps: usize, mut step: impl FnMut(&mut Runtime<S>, usize)) {
    let mut rt = rt();
    let mut rows = Vec::new();
    for i in 0..steps {
        let (c0, v0) = (rt.stats.commits, rt.state.views.get());
        step(&mut rt, i);
        rt.frame(0.016);
        rows.push((rt.stats.commits - c0, rt.state.views.get() - v0));
    }
    for _ in 0..60 { rt.frame(0.016); }
    let top = rt.elements().into_iter().filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with('r')) && e.rect[1] >= 0.0)
        .min_by(|a, b| a.rect[1].total_cmp(&b.rect[1])).and_then(|e| e.key);
    println!("{name}: per frame (commits, view runs) {rows:?}; top row after: {top:?}");
}
fn main() {
    let step = 1250.0;
    per_frame("scroll_to_instant", 6, |rt, i| {
        rt.update_if(|_, tx| { tx.scroll_to_instant("vp", 0.0, step * (i + 1) as f32); false });
    });
    per_frame("scroll_to_with   ", 6, |rt, i| {
        rt.update_if(|_, tx| { tx.scroll_to_with("vp", 0.0, step * (i + 1) as f32, Spring::new(0.18, 0.0)); false });
    });
    per_frame("wheel            ", 6, |rt, _| {
        rt.pointer_move(150.0, 200.0);
        rt.wheel(150.0, 200.0, -step);
    });
}
```

Output:

```
scroll_to_instant: per frame (commits, view runs) [(3, 3), (4, 3), (4, 3), (4, 3), (4, 3), (4, 3)]; top row after: Some("r375")
scroll_to_with   : per frame (commits, view runs) [(2, 2), (4, 3), (4, 3), (4, 3), (4, 3), (4, 3)]; top row after: Some("r375")
wheel            : per frame (commits, view runs) [(1, 0), (1, 0), (1, 0), (1, 0), (1, 0), (1, 0)]; top row after: Some("r375")
```

The view here doesn't change, so each of its runs is spent on remounting rows the wheel remounts
without it.

**Ask.** Let a scroll request remount virtual rows as the wheel does, without running the view.

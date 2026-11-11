# 10. Wheel routing: modified wheels, gesture latching, overlays, and sideways strips

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C3-3, U-C24-3, U-C24b-2, U-C6-5 (and U-C7-2's wheel half), U-C24-7, C20a's "a gesture keeps its first route" follow-up |
| Affects | C1e, C2, C3, C6, C7, C20a, C24b |
| Retires | C1e's `Collection::on_wheel` on the viewport, C6's and C7's vertical-to-sideways mapping (C7's `wheel.rs`, "fragile"), C24b's `OutsideWheel` panel guard |
| Checked | Re-checked at `2a44156`: the repro's five lines in `run-2a44156.txt` match the Output block. By source, the latch is still `WheelOwner` (`runtime/scroll/x.rs:46`): `Vertical` (`:53`) carries no viewport, and `Horizontal(NodeId)` (`:56`) does. `WHEEL_IDLE` is 0.1 s (`:17`), and the Shift turn is in the winit host's `wheel_delta` only (`:114`). `run_wheel` still drops a notch the holding handler declines (`runtime/input/gesture.rs:63–65`), and `scroll_at` is at `runtime/input/pointer.rs:410`. In the gallery, `mise run test a_vertical_wheel_scrolls_an_overflowing_strip_sideways` passed (1 test: C7's own vertical-to-sideways mapping). C1e's `Collection::on_wheel` (`collection/element/mod.rs:653`), C7's "`wheel.rs`, fragile" (`tabs/mod.rs:254`) and C24b's `OutsideWheel` (`scroll/mod.rs:85`) are still there. Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Wheel routing: ancestors can't claim a modified wheel, a vertical gesture jumps viewports, and wheels pass through overlays

Context: Strata (lgse/strata), a file manager on Rune with Miller columns (vertical lists in a sideways strip), an
icon grid that zooms on Ctrl+Shift+wheel, a breadcrumb strip and popovers over lists.

**What happens** (`5d421fd`, unchanged at `2a44156`), five cases from one program:
1. **An ancestor never hears a wheel over a scroll view's content.** `on_wheel` is offered from
   the element under the pointer up to the nearest scroll view. A Ctrl+Shift wheel over the
   content scrolls it; the ancestor's handler hears nothing. A wheel that began as a scroll also
   stays one until it has been still 0.1 s (`WHEEL_IDLE`), so Ctrl+Shift right after a plain
   wheel scrolls too. And once a handler holds a gesture, a notch it declines (returns `false`)
   is dropped: it neither scrolls nor reaches the next handler (`run_wheel`,
   `runtime/input/gesture.rs:63–65` at `2a44156`; by source).
2. **Only the horizontal axis latches.** A vertical gesture that starts over one column scrolls
   whichever column the pointer reaches. The README says a gesture latches "as AppKit latches a
   gesture to the view it began on".
3. **A wheel over a non-scrolling element drawn above a list scrolls the list** (`scroll_at`
   picks the innermost scroll viewport under the point, whatever covers it).
4. **A vertical mouse wheel over a sideways-only viewport scrolls nothing**; GTK and AppKit turn
   it sideways there.
5. **Headless, Shift+wheel stays vertical.** The winit host turns a Shift mouse wheel sideways
   (`wheel_delta`, `runtime/scroll/x.rs:114`), but `Runtime::wheel_event` and scripts don't, so
   it can't be tested.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, GesturePhase, Mods, Runtime, ScrollSpec, WheelEvent, col, el, row};
#[derive(Default)] struct S { outer: u32 }
fn tall(key: &str, w: f32, h: f32) -> El {
    col().key(key.to_string()).size(w, h).scroll(ScrollSpec { stick_bottom: false })
        .child(el().key(format!("{key}-content")).w_full().h(4000.0))
}
/// How far viewport `key` has scrolled, from where its content sits.
fn scrolled(rt: &Runtime<S>, key: &str) -> [f32; 2] {
    let els = rt.elements();
    let rect = |k: &str| els.iter().find(|e| e.key.as_deref() == Some(k)).map(|e| e.rect).unwrap();
    let (v, c) = (rect(key), rect(&format!("{key}-content")));
    [v[0] - c[0], v[1] - c[1]]
}
fn ancestor(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().on_wheel(|s: &mut S, _, _| { s.outer += 1; true }).child(col().size(300.0, 200.0).child(tall("vp", 300.0, 200.0)))
}
fn strip(_: &S, _: &mut Cx<S>) -> El {
    row().key("strip").size(800.0, 300.0).scroll_x().child(row().key("strip-content").w(1200.0).h_full()
        .child(tall("c0", 400.0, 300.0)).child(tall("c1", 400.0, 300.0)).child(tall("c2", 400.0, 300.0)))
}
fn sideways(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(row().key("crumbs").size(300.0, 40.0).scroll_x().child(el().key("crumbs-content").w(1500.0).h(40.0)))
}
fn overlay(_: &S, _: &mut Cx<S>) -> El {
    col().size(400.0, 300.0).child(tall("list", 400.0, 300.0))
        .child(el().key("panel").pin(Some(20.0), Some(20.0), None, None).size(220.0, 200.0).on_press(|_: &mut S| {}))
}
fn main() {
    let ctrl_shift = Mods { ctrl: true, cmd: true, shift: true, ..Mods::default() };
    let mut rt = Runtime::new(S::default(), ancestor, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    for _ in 0..2 { rt.wheel_event(WheelEvent::new(0.0, -120.0).at(150.0, 100.0).mods(ctrl_shift)); rt.frame(1.5); }
    println!("1. ancestor heard {}, viewport scrolled to {:?}", rt.state.outer, scrolled(&rt, "vp"));

    let mut rt = Runtime::new(S::default(), strip, (800.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let phases = [GesturePhase::Began, GesturePhase::Moved, GesturePhase::Moved, GesturePhase::Ended];
    for (x, phase) in [300.0, 390.0, 450.0, 500.0].into_iter().zip(phases) {
        rt.wheel_event(WheelEvent::new(0.0, -40.0).at(x, 150.0).phase(phase).precise(true));
        rt.frame(0.016);
    }
    for _ in 0..60 { rt.frame(0.016); }
    println!("2. one gesture begun over column 0: c0 {}, c1 {}", scrolled(&rt, "c0")[1], scrolled(&rt, "c1")[1]);
    rt.wheel_event(WheelEvent::new(0.0, -120.0).at(600.0, 150.0).mods(Mods { shift: true, ..Mods::default() }).precise(false));
    for _ in 0..30 { rt.frame(0.016); }
    println!("5. Shift + mouse wheel: strip x {}, c1 y {}", scrolled(&rt, "strip")[0], scrolled(&rt, "c1")[1]);

    let mut rt = Runtime::new(S::default(), sideways, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.wheel_event(WheelEvent::new(0.0, -120.0).at(150.0, 20.0).precise(false));
    for _ in 0..30 { rt.frame(0.016); }
    println!("4. vertical wheel over a sideways strip: x {}", scrolled(&rt, "crumbs")[0]);

    let mut rt = Runtime::new(S::default(), overlay, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.wheel_event(WheelEvent::new(0.0, -120.0).at(100.0, 100.0).precise(false));
    for _ in 0..30 { rt.frame(0.016); }
    println!("3. wheel over a panel above the list: list y {}", scrolled(&rt, "list")[1]);
}
```

Output:

```
1. ancestor heard 0, viewport scrolled to [0.0, 240.0]
2. one gesture begun over column 0: c0 80, c1 80
5. Shift + mouse wheel: strip x 0, c1 y 200
4. vertical wheel over a sideways strip: x 0
3. wheel over a panel above the list: list y 120
```

**Ask.**
1. A capture-phase `on_wheel` (or let an ancestor claim a modified wheel); offer a delta whose
   modifiers changed as a new gesture; and pass a declined notch on instead of dropping it.
2. Latch a vertical gesture to the viewport it began on, as a horizontal one is.
3. A wheel over an element drawn above a viewport, and not inside it, doesn't scroll it.
4. A vertical mouse wheel scrolls a viewport that only scrolls sideways.
5. Apply the Shift turn in `Runtime::wheel_event`, where scripts reach it.

**Why.** Each case is worked around in the app: a wheel handler on every viewport, a
hand-written vertical-to-sideways mapping that reads where content drew, and a guard that
swallows wheels over popovers and forwards them by hand.

Related: (link to 43) (`Tx::scroll_by`).

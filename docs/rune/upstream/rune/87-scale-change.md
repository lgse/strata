# 87. No notification on a scale-only change

| | |
| --- | --- |
| Kind | New issue |
| Drafts | C14a's "No notification on a scale-only change" draft |
| Affects | C14a, C14b (image and model viewports), C12 |
| Retires | C14a's one-off rescale child mounted to resync after a scale change |
| Checked | Re-checked at `2a44156` with a headless probe (`repros-head/src/bin/rv_87.rs`; the program below, `rv_87_body.rs`, printed the Output block as is): after `set_viewport` to the same size at scale 2, `on_layout_tx` and `on_drawn_layout` fire nothing, while the view runs again with `Cx::env().scale` 2. By source: `report_rects` compares rects in points only (`runtime/layout_read.rs:538–568`), and `change_viewport` changes the scale and relays out with no rect moving (`runtime/frame.rs:28–44`). #500 (macOS frames at the old scale) and #316 (Windows rescaling twice) are other gaps. Gallery: C14a's rescale element (`viewers/viewport/mod.rs:404`) and C12's (`pdf_view/render.rs:192`) are still there, and their tests pass (`mise run test a_scale_change_alone`, 2 tests; `scale_round_trip`, 2). Earlier: from C14a's report at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A display scale change alone fires no layout handler

Context: Strata (lgse/strata), a file manager on Rune.

When only the display scale changes (a window moved to a monitor with another scale, with the
same size in points), no `on_layout` or `on_layout_tx` fires, so an element that renders pixels
for its size in device pixels (an image or a PDF page at 1:1) doesn't hear it. The view runs
again and reads the new `Cx::env().scale`, but no handler with a `Tx` runs, so to start the
render we mount a fresh element keyed by the scale for its first `on_layout_tx`.

**Repro** (headless, at `2a44156`):

```rust
use rune::{Cx, El, Env, Runtime, col, el};
#[derive(Default)] struct S { layout: usize, drawn: usize }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("image").size(100.0, 80.0)
        .on_layout_tx(|s: &mut S, _, _| s.layout += 1)
        .on_drawn_layout(|s: &mut S, _| s.drawn += 1))
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    for _ in 0..3 { rt.frame(0.016); }
    let before = (rt.state.layout, rt.state.drawn);
    rt.set_viewport((300.0, 200.0), 2.0);
    for _ in 0..5 { rt.frame(0.016); }
    println!("after a scale-only change: on_layout_tx +{}, on_drawn_layout +{}",
        rt.state.layout - before.0, rt.state.drawn - before.1);
}
```

**Output:**

```
after a scale-only change: on_layout_tx +0, on_drawn_layout +0
```

**Ask.** Fire layout handlers on a scale change (with the scale), or an `on_scale_change` hook.

Related: #316, #500 (a window moved to another scale).

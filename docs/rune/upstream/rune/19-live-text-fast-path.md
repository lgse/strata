# 19. `live_text` turns off the virtual-window fast path

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C1f-6 |
| Affects | C1e/C1f (stuck group header), C13, C14b (route labels), C25 (gallery status lines) |
| Retires | Built text where a live label was wanted, and the view build per group crossed |
| Checked | Re-checked at `2a44156`: `doc_19`'s output in `run-2a44156.txt` matches the Output block. By source, `virtual_only` (`runtime/commit/remount.rs:41`) still requires `t.live.is_none()` on every node (`:50`). In the gallery, C1e still builds text where a live label was wanted (`collection/element/mod.rs:105`, `:1040`, U-C1f-6); the `collection::element` tests passed. Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output (`remount.rs:40` at the pin). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** One `live_text` anywhere makes every virtual-window shift run the view

Context: Strata (lgse/strata), a file manager on Rune.

A commit that only shifts a virtual window skips `view` unless some node has live text
(`virtual_only`, `runtime/commit/remount.rs:41`, the check at `:50`, at `2a44156`), so a single
live label anywhere in the window costs a view build each time scrolling crosses a row.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, GrowthSpec, LiveText, Runtime, ScrollSpec, WheelEvent, col, el, text};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
static RUNS: AtomicUsize = AtomicUsize::new(0);
static LABEL: LazyLock<LiveText> = LazyLock::new(|| LiveText::new("x"));
struct S { live: bool }
fn view(s: &S, _: &mut Cx<S>) -> El {
    RUNS.fetch_add(1, Ordering::Relaxed);
    let label = if s.live { text("x").live_text(&LABEL) } else { text("x") };
    col().w_full().h_full().child(label).child(col().size(300.0, 300.0).scroll(ScrollSpec { stick_bottom: false }).child(
        col().w_full().growth(GrowthSpec::default())
            .virtual_rows(1000, 20.0, |_: &S, i| format!("r{i}").into(), |_: &S, _| el().w_full().h(20.0))))
}
fn run(live: bool) {
    let mut rt = Runtime::new(S { live }, view, (300.0, 400.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(0.016);
    RUNS.store(0, Ordering::Relaxed);
    for _ in 0..120 { rt.wheel_event(WheelEvent::new(0.0, -7.0).at(150.0, 200.0)); rt.frame(0.016); }
    println!("live_text {live}: the view ran {} times over 120 frames of 7 pt scrolling", RUNS.load(Ordering::Relaxed));
}
fn main() { run(false); run(true); }
```

Output:

```
live_text false: the view ran 0 times over 120 frames of 7 pt scrolling
live_text true: the view ran 11 times over 120 frames of 7 pt scrolling
```

**Ask.** Let live text stay bound across a remount of the virtual window, so the fast path
holds with live labels in the tree.

Related: #422 (`live_text` panics on wrapping or styled text).

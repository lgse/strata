# 15. `element_at` for app-side drop routing: passive layers, rects, and what a hit carries

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-X2-1, U-P1-1, U-P1-2 (X2's addition to U-P1-1) |
| Affects | P1, W1e, X2, C2, C6, C24 |
| Retires | W1e's keys and scopes passed to every drop zone, P1's `drawn_rect_into` takeover, C24's zero-size `scroll::anchor` for bars over targets |
| Checked | Re-checked at `2a44156`: `doc_15`'s output in `run-2a44156.txt` matches the Output block. By source, `Pick` still has only `transparent` (`runtime/elements.rs:88–89`) and `Picked` only a path and a key (`:80–83`). In the gallery, strata-platform's dnd shim still takes over `drawn_rect_into` (`dnd/window.rs:236`, `:246`) and registers targets by key (`dnd/registry.rs:4`, U-P1-1; `dnd/mod.rs:61–66`), and C1e's bar overlay still hangs from a zero-size `scroll::anchor` (`collection/element/mod.rs:1011`); the `collection::element` tests passed. Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `element_at` stops at layers that take no input, and gives no rect, local point or way to find what the app attached

Context: Strata (lgse/strata), a file manager on Rune, routes drags from the desktop (typed, beyond
Rune's file drops) to its own drop zones: list rows, folder tiles, tabs, sidebar places. It finds
the target with `Tx::element_at` and the key along the hit path.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- `element_at` returns the topmost box, handlers or not, so a full-size passive layer (a
  scroll-bar overlay with no handlers) hides every target under it. Presses and Rune's own
  `on_drop_files` skip such elements. `Pick` can only add transparent elements.
- A `Picked` has a path and a key, but no rect and no point in the element's own coordinates
  (Rune's own drops have `drop_point`).
- An element has one handler of each kind, and nothing attached to it can be read back from a
  hit, so a crate that adds drop support to a component's element must take over its
  `on_drag` or `drawn_rect_into`, and targets are found only by keys the app passes in.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, Runtime, col, el};
#[derive(Default)] struct S { pressed: bool }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().size(300.0, 200.0)
        .child(col().key("rows").child(el().key("row-1").size(300.0, 30.0).on_press(|s: &mut S| s.pressed = true)))
        .child(col().key("layer").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).child(el().key("bar").w(6.0).h_full()))
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let p = rt.element_at(150.0, 15.0);
    println!("element_at(150, 15): path {:?}, key {:?}", p.as_ref().map(|p| p.path().to_string()), p.as_ref().and_then(|p| p.key().map(str::to_string)));
    rt.pointer_move(150.0, 15.0);
    rt.pointer_down(150.0, 15.0);
    rt.pointer_up(150.0, 15.0);
    rt.frame(0.016);
    println!("a press at the same point reaches row-1: {}", rt.state.pressed);
}
```

Output:

```
element_at(150, 15): path Some("#0/layer"), key Some("layer")
a press at the same point reaches row-1: true
```

**Ask.**
- `Pick::reaching(kind)` (or `Pick::handlers_only()`): skip elements with no handler of a kind
  (press, drop), as press and drop routing do.
- The hit element's drawn rect and the point in its own coordinates on `Picked`.
- A way to find app data on a hit (an extension slot readable from `Picked`), or composable
  handlers so a second `on_drag` doesn't replace the first.

A drop-target routing hook for non-file payloads (#357) would make most of this unneeded.

Related: #357, #265, (link to 14).

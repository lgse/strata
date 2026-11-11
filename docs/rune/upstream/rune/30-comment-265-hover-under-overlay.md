# 30. Comment on #265: hover and hover delays reach elements under an overlay too

| | |
| --- | --- |
| Kind | Comment on [#265](https://github.com/anomalyco/rune/issues/265) (pointer moves reach elements under a pinned sibling) |
| Drafts | U-C22-1 (and C1e's `row.rs` note: a row under an overlay counts as hovered) |
| Affects | C22 (folder peek), C1e (row hover) |
| Retires | C22's `set_covered` host duty and its "no peek while covered" rule |
| Checked | Re-checked at `2a44156`: the repro's output in `run-2a44156.txt` matches the block below. By source, `within_next` (`runtime/input/hover.rs:113`) still counts every hit that holds the pointer and has `on_hover_change`, and hover delays go through it (`:79`, `:89`). #265 is still open; its only comment is a fork commit that stops `on_pointer_move` at the topmost target, and nothing in the thread mentions hover or `hover_delays`, so the comment adds to it. In the gallery, `mise run test covered` passed (1 test: `folder_peek::tests::a_covered_collection_closes_its_peek_and_opens_none`, C22's `set_covered` duty, `folder_peek/mod.rs:58–61`); C1e's note is still there (`collection/element/row.rs:259`). Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output (`:82` at the pin). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10). Post it as a comment on #265, which is still open. |

Paste from here.

---

From Strata (lgse/strata), a file manager on Rune: the same holds for hover, not only
`on_pointer_move`. `on_hover_change` and `hover_delays` treat every element whose hit rect holds the
pointer as hovered (`within_next`, `runtime/input/hover.rs:113` at `2a44156`), while a press goes
only to the topmost:

```rust
use rune::{Cx, El, Env, Runtime, col, el};
#[derive(Default)] struct S { row: bool, card: bool, row_pressed: bool }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().size(300.0, 200.0)
        .child(el().key("row").size(300.0, 40.0)
            .on_press(|s: &mut S| s.row_pressed = true)
            .on_hover_change(|s: &mut S, h, _| s.row = h))
        .child(el().key("card").pin(Some(50.0), Some(0.0), None, None).size(150.0, 100.0)
            .on_press(|_: &mut S| {})
            .on_hover_change(|s: &mut S, h, _| s.card = h))
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.pointer_move(100.0, 20.0); // over the card, which covers the row here
    rt.frame(0.6);
    println!("pointer on the card over the row: card hovered {}, row hovered {}", rt.state.card, rt.state.row);
    rt.pointer_down(100.0, 20.0);
    rt.pointer_up(100.0, 20.0);
    rt.frame(0.016);
    println!("a press there reaches the row: {}", rt.state.row_pressed);
}
```

prints, at `5d421fd` and `2a44156`:

```
pointer on the card over the row: card hovered true, row hovered true
a press there reaches the row: false
```

In a file manager a row under a hover card (or under rune-kit's `scrim`, a menu's outside-press
catcher) starts its own hover-delayed peek after 0.5 s. A fix for #265 that stops hover and its
delays at the topmost element hit, as presses do, would cover both.

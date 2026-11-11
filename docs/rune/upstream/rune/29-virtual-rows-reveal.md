# 29. `virtual_rows`: `reveal_selected` and `scroll_key_to` miss rows that aren't laid out

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C17-4, U-C12-3 |
| Affects | C17 (search palette), C12 (PDF view) |
| Retires | C17's `Tx::scroll_into_view_in` on each arrow; C12's re-request of a zoom's centre row a frame later |
| Checked | Re-checked at `2a44156`: the U-C17-4 repro's output in `run-2a44156.txt` matches the Output block. By source, `measure.rs:542` still reads `reveal_selected` only in the branch for rows handed over as children. The U-C12-3 half was re-run with a new headless probe, `rv_29`: 500 rows of 20 pt, row 300 placed at the top with `scroll_key_to`; a commit that makes every row 60 pt and asks `scroll_key_to("r300", 0)` in the same handler leaves r300 unmounted, five frames later too, and asking again a frame later does nothing either. `scroll_key_to`'s doc now says so (`el/tx/scroll.rs:102–108`: "The element must be laid out: reveal a virtual row that isn't mounted with `Tx::scroll_into_view`"), but `scroll_into_view` (`:18`) scrolls just far enough to show the row, and can't place it at an offset. In the gallery, `mise run test arrows_keep_the_highlighted_row_in_view` and `an_arrow_in_a_batch_s_frame_reveals_its_row` passed (1 test each: C17's `scroll_into_view_in` on each arrow, `search_palette/view.rs:363`, `search_palette/mod.rs:287`). C12's note is still there (`pdf_view/mod.rs:206`). Earlier: the U-C17-4 repro ran at `5d421fd` and `04b38ae` with the same output; U-C12-3 was C12's probe at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `virtual_rows`: `reveal_selected` does nothing for rows built by index, and `scroll_key_to` places only a row already laid out

Context: Strata (lgse/strata), a file manager on Rune.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- `runtime/commit/measure.rs:542` checks `focus.reveal_selected` only in the branch for rows
  handed over as children (`virtualize`), not for `virtual_rows`' index-built rows, so a
  newly `focus_selected` row never scrolls into view.
- After a commit that resizes every row far down a list (a zoom), the row under the centre isn't
  mounted at the old offset, and `scroll_key_to` for it does nothing (500 rows of 20 pt made
  60 pt, row 300), in that commit or asked again a frame later. Its doc says the row must be
  laid out and points to `scroll_into_view`, which scrolls just far enough to show the row and
  can't place it at an offset.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, col, el};
struct S { selected: usize }
fn by_children(s: &S, _: &mut Cx<S>) -> El {
    col().key("vp").size(300.0, 200.0).scroll(ScrollSpec { stick_bottom: false }).child(
        col().key("list").w_full().growth(GrowthSpec::default()).virtualize(20.0).reveal_selected(true)
            .children((0..100).map(|i| el().key(format!("r{i}")).w_full().h(20.0).focus_selected(i == s.selected))))
}
fn by_index(_: &S, _: &mut Cx<S>) -> El {
    col().key("vp").size(300.0, 200.0).scroll(ScrollSpec { stick_bottom: false }).child(
        col().key("list").w_full().growth(GrowthSpec::default()).reveal_selected(true)
            .virtual_rows(100, 20.0, |_: &S, i| format!("r{i}").into(), |s: &S, i| el().w_full().h(20.0).focus_selected(i == s.selected)))
}
fn reveal(name: &str, view: fn(&S, &mut Cx<S>) -> El) {
    let mut rt = Runtime::new(S { selected: 0 }, view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.update(|s, _| s.selected = 60);
    for _ in 0..60 { rt.frame(0.016); }
    let row = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("r60")).map(|e| e.rect[1]);
    println!("{name}: row 60 selected, drawn at {row:?}");
}
fn main() {
    reveal("rows as virtualize children", by_children);
    reveal("rows built by virtual_rows ", by_index);
}
```

Output:

```
rows as virtualize children: row 60 selected, drawn at Some(180.0)
rows built by virtual_rows : row 60 selected, drawn at None
```

**Ask.** The same reveal for index-built rows (their keys are known), and `scroll_key_to` that
places a virtual row by key from the commit's layout, as `scroll_into_view` reaches an unmounted
row.

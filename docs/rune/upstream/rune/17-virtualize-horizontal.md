# 17. Virtualization is vertical only: no grid, no columns, and sideways content is cut

| | |
| --- | --- |
| Kind | New issue (related #203) |
| Drafts | U15 (S5), U-C11b-3, R7 (C11) |
| Affects | C3 (icon grid), C11b (sheet grid), C10b and C11 (wrap-off documents) |
| Retires | C3's tiles packed into rows and re-keyed on every reflow, C11b's own column windowing from `on_layout_tx`, C10b's min width from an estimate of the widest rows |
| Checked | Re-checked at `2a44156`: `doc_17`'s output in `run-2a44156.txt` matches the Output block, with the same debug warning. Rune's README still says "Virtual rows are vertical only" (`crates/rune/README.md:410`). In the gallery, C3 still packs tiles into virtual rows (`icons_grid/mod.rs:5`, U15; the `c3-default` accessibility golden reads rows of tiles), C11b still windows its own columns (`sheet_grid/mod.rs:226`, U-C11b-3) and C10b still notes R7 (`document_view/mod.rs:91`, `:129`); `c3::` and `c11b::` passed. Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output (README line 384 at the pin). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Virtual rows are vertical only: a wrapped grid mounts every tile, `.virtualize` in a horizontal scroll stacks diagonally, and wide rows are cut

Context: Strata (lgse/strata), a file manager on Rune: its icon view shows tens of thousands of
thumbnails in a wrapped grid, a spreadsheet preview has hundreds of columns, and a code view with
wrap off scrolls sideways.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- A wrapped grid has no virtualization: every tile mounts. Packing N tiles per virtual row
  works, but the app reflows rows on every width change.
- `.virtualize` on a row inside `scroll_x` picks the window as if the row were vertical, so
  each tile also moves down one estimate.
- A wide virtual grid builds every column of every mounted row (256 columns ran p95 31 ms a
  frame sideways in our grid before it windowed its own columns).
- Virtual rows inside a sideways viewport (`scroll_x` around the vertical one, or `scroll_xy`)
  lay out at the viewport's width: a 2,000-character unwrapped row is cut at the edge, and the
  sideways offset stays at 0 because the list reports no wider content.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, col, el, row};
const TILES: usize = 20_000;
const PER_ROW: usize = 8;
fn tile(i: usize) -> El { el().key(format!("t{i}")).size(96.0, 96.0) }
fn wrapped(_: &(), _: &mut Cx<()>) -> El {
    col().key("scroll").size(800.0, 600.0).scroll(ScrollSpec { stick_bottom: false })
        .child(row().key("grid").w(800.0).flex_wrap().children((0..TILES).map(tile)))
}
fn packed(_: &(), _: &mut Cx<()>) -> El {
    col().key("scroll").size(800.0, 600.0).scroll(ScrollSpec { stick_bottom: false })
        .child(col().key("rows").w_full().growth(GrowthSpec::default())
            .virtual_rows(TILES / PER_ROW, 96.0, |_: &(), r| format!("r{r}").into(),
                |_: &(), r| row().h(96.0).children((0..PER_ROW).map(|c| tile(r * PER_ROW + c)))))
}
fn horizontal(_: &(), _: &mut Cx<()>) -> El {
    row().key("scroll").size(800.0, 600.0).scroll_x()
        .child(row().key("cols").h_full().growth(GrowthSpec::default()).virtualize(96.0)
            .children((0..TILES).map(tile)))
}
fn count(name: &str, view: fn(&(), &mut Cx<()>) -> El) {
    let mut rt = Runtime::new((), view, (800.0, 600.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    let tiles: Vec<_> = rt.elements().into_iter().filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with('t'))).collect();
    println!("{name}: {} of {TILES} tiles have elements", tiles.len());
    for e in tiles.iter().take(3) { println!("  {} at {:?}", e.key.as_deref().unwrap_or(""), &e.rect[..2]); }
}
fn main() {
    count("wrapped grid", wrapped);
    count("8 tiles per virtual row", packed);
    count("virtualize in scroll_x", horizontal);
}
```

Output (the debug build also warns `virtualized list "scroll/cols" … built 20000 rows to mount
19`):

```
wrapped grid: 20000 of 20000 tiles have elements
  t0 at [0.0, 0.0]
  t1 at [96.0, 0.0]
  t2 at [192.0, 0.0]
8 tiles per virtual row: 120 of 20000 tiles have elements
  t0 at [0.0, 0.0]
  t1 at [96.0, 0.0]
  t2 at [192.0, 0.0]
virtualize in scroll_x: 19 of 20000 tiles have elements
  t0 at [0.0, 0.0]
  t1 at [96.0, 96.0]
  t2 at [192.0, 192.0]
```

**Ask.**
- Grid virtualization (a fixed tile size and a column count from the width would do).
- Horizontal virtualization (virtual columns), or a panic for `.virtualize` in a horizontal
  viewport, as there is for one without `.growth`.
- Virtual rows in a sideways viewport laid out at their content's width, so the sideways
  viewport scrolls to the widest mounted row.

Related: #203 (a count change re-reads every key).

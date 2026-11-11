# 18. Exact or per-index heights for `virtual_rows`

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C1f-1, U-C12-2 |
| Affects | C1e/C1f (group headers forced to the row height), C12 (PDF pages of mixed sizes) |
| Retires | C1f's "header lines are the row height" deviation; C12's zoom that lands up to 0.4 pages off for a frame |
| Checked | Re-checked at `2a44156`, by a new headless probe (`rv_18`, the repro below): rows scrolled through land at the sum of their heights, but after one 20,000 pt jump the first mounted row (r737) sits at content y 19,226 where its rows above sum to 20,634, 4 pt high for each unvisited 30 pt row. By source, one estimate per list (`virtual_rows(count, estimated_row_height, ..)`, `el/virtual_rows.rs:83`, stored by `virtualize` at `:35`). In the gallery, C1f's "header lines are the row height" deviation (`collection/element/mod.rs:114`, `:137`, `lines.rs:19`, U-C1f-1) and C12's U-C12-2 (`pdf_view/mod.rs:202`) are still there; the `collection::element` and `c12::` tests passed. Earlier: by source at `5d421fd` (`:80`) and `04b38ae`; the numbers were the components' own probes. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `virtual_rows` has one height estimate, so rows of known, mixed heights land where the estimate puts them

Context: Strata (lgse/strata), a file manager on Rune: its grouped list has section headers of one
height and rows of another; its PDF view has pages of mixed sizes that are known before they are
drawn.

**What's missing** (`5d421fd`, unchanged at `2a44156`). A virtual list has one height
estimate (`virtual_rows(count, estimated_row_height, ..)`), and rows it hasn't laid out take it.
Rows of two heights then sit where the app's own index geometry doesn't put them: a press, a
marquee or a reveal computed from the real heights misses. The scroll extent drifts until rows
are visited, and a zoom far down a document can't land in one frame.

**Repro** (headless; ran at `2a44156`): 1,000 rows alternating 26 and 30 pt with an estimate of
26, scrolled about 20,000 pt in small wheel steps, then in one step.

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, WheelEvent, col, el};
fn h(i: usize) -> f32 { if i % 2 == 0 { 26.0 } else { 30.0 } }
fn view(_: &(), _: &mut Cx<()>) -> El {
    col().key("vp").size(300.0, 400.0).scroll(ScrollSpec { stick_bottom: false }).child(
        col().key("list").w_full().growth(GrowthSpec::default())
            .virtual_rows(1000, 26.0, |_: &(), i| format!("r{i}").into(), |_: &(), i| el().w_full().h(h(i))))
}
fn scroll(step: f32) {
    let mut rt = Runtime::new((), view, (300.0, 400.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(0.016);
    for _ in 0..(20_000.0 / step) as usize { rt.wheel_event(WheelEvent::new(0.0, -step).at(150.0, 200.0)); rt.frame(0.016); }
    for _ in 0..30 { rt.frame(0.016); }
    let els = rt.elements();
    let vp = els.iter().find(|e| e.key.as_deref() == Some("vp")).unwrap().rect[1];
    let offset = els.iter().find(|e| e.key.as_deref() == Some("list")).unwrap().scrolled[1];
    let (i, y) = els.iter().filter_map(|e| Some((e.key.as_deref()?.strip_prefix('r')?.parse::<usize>().ok()?, e.rect[1])))
        .min_by_key(|r| r.0).unwrap();
    let exact: f32 = (0..i).map(h).sum();
    println!("wheel steps of {step} pt: first mounted row r{i} at content y {}, its rows above sum to {exact}", y - vp + offset);
}
fn main() { scroll(300.0); scroll(20_000.0); }
```

Output:

```
wheel steps of 300 pt: first mounted row r678 at content y 18984, its rows above sum to 18984
wheel steps of 20000 pt: first mounted row r737 at content y 19226, its rows above sum to 20634
```

After the jump the rows sit 1,408 pt above where their known heights put them: 4 pt for each
30 pt row the list never laid out. In our PDF view, 500 pages of mixed sizes, the scroll extent
changes as pages are visited.

**Ask.** A height per index that layout takes instead of the estimate, either exact
("rows are exactly this tall": `virtual_rows_heights(|s, i| h)`) or as a hint used until a row
is measured.

Related: #364 (a running glide and the anchor), #203.

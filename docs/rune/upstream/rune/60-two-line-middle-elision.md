# 60. Two-line text with a middle cut

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C3-1 |
| Affects | C3 (icon captions) |
| Retires | C3's own two-line middle cut on grapheme clusters (and its `unicode-segmentation` dependency) |
| Checked | At `2a44156`, repro re-run (`doc_60`): same output. Gallery: C3's own cut and its `unicode-segmentation` dependency are still there (`icons_grid/caption.rs:6`, `icons_grid/mod.rs:243`, `strata-components/Cargo.toml:23`); the `caption` tests pass. Earlier: repro run at `5d421fd` and `04b38ae`, same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `max_lines(2)` and `truncate_middle()` don't combine: a long file name over two lines can't keep its end

Context: Strata (lgse/strata), a file manager on Rune.

`max_lines(n)` ends its last line with "…"; `truncate_middle` makes the text one line; whichever
comes last wins. A file name over two lines ("Quarterly report … two of the budget.xlsx")
can't keep its extension.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, Runtime, col, text};
const NAME: &str = "Quarterly report final revised version two of the budget.xlsx";
fn view(_: &(), _: &mut Cx<()>) -> El {
    col().w(120.0)
        .child(text(NAME).key("a").max_lines(2))
        .child(text(NAME).key("b").max_lines(2).truncate_middle())
        .child(text(NAME).key("c").truncate_middle().max_lines(2))
}
fn main() {
    let mut rt = Runtime::new((), view, (300.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    for e in rt.elements().into_iter().filter(|e| matches!(e.key.as_deref(), Some("a" | "b" | "c"))) {
        println!("{}: height {} pt", e.key.unwrap(), e.rect[3]);
    }
}
```

Output: `a: height 32.5 pt`, `b: height 16.25 pt` (one line), `c: height 32.5 pt` (two lines,
cut at the end).

**Ask.** An elision mode that combines with `max_lines` (`.max_lines(2).elide_middle()`),
cutting between graphemes, so the last line keeps the text's end.

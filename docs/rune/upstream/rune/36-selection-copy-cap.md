# 36. Copying a selection across unbuilt virtual rows stops at a cap, silently

| | |
| --- | --- |
| Kind | New issue |
| Drafts | R4 (C11) |
| Affects | C11 (document view), C10b (code view) |
| Retires | C11's and C10b's Ctrl+A copy from their own model (and the frame in which Ctrl+C copies Rune's cut report) |
| Checked | Re-checked at `2a44156`. By source, `MAX_VIRTUAL_COPY_ROWS = 10_000` and `MAX_VIRTUAL_COPY_BYTES = 4 MiB` are still at `runtime/virtual/selection.rs:21, 25`, the collection stops on them (`runtime/virtual/selection_collect.rs:231`), and `TextSelection` has no mark of a cut (`runtime/selection/text_selection.rs`). Re-measured with a new headless probe, `rv_36` (the program in the body): `Tx::select_all_text` over 12,000 virtual rows of selectable text reports 10,020 texts and 99,109 of 120,889 bytes. In the gallery, `mise run test ctrl_a_then_ctrl_c_copies_the_whole_document`, `past_runes_cap` and `select_all_copies_the_whole_large_text` passed (1 test each: C11's and C10b's Ctrl+A copy from their own model), and their notes are still there (`document_view/copy.rs:131`, `document_view/mod.rs:42`, `code_view/copy.rs:54`). Earlier: by source at `5d421fd` and `04b38ae`; C11's probe P3 at the pin (10,057 of 12,000 texts). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A selection across unbuilt virtual rows is cut at 10,000 rows or 4 MiB, and nothing says so

Context: Strata (lgse/strata), a file manager on Rune.

**What happens** (`5d421fd`, unchanged at `2a44156`). Copying or reading a selection that spans
virtual rows not built stops at 10,000 rows or 4 MiB (`MAX_VIRTUAL_COPY_ROWS`,
`MAX_VIRTUAL_COPY_BYTES`, `runtime/virtual/selection.rs:21, 25`): `selection_copy` sees a cut
list, and nothing says it was cut.

**Repro** (headless; ran at `2a44156`):

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, col, text};
#[derive(Default)]
struct S;
const ROWS: usize = 12_000;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(col().key("vp").size(300.0, 200.0).scroll(ScrollSpec { stick_bottom: false }).child(
        col().key("list").w_full().growth(GrowthSpec::default()).selection_group(true)
            .virtual_rows(ROWS, 20.0, |_: &S, i| format!("r{i}").into(), |_: &S, i| text(format!("line {i}")).selectable(true).h(20.0))))
}
fn main() {
    let mut rt = Runtime::new(S, view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.update(|_, tx| tx.select_all_text("list"));
    rt.frame(0.016);
    let sel = rt.selection().expect("a selection");
    let last = sel.texts().last().and_then(|t| t.key().map(str::to_string));
    println!("Tx::select_all_text(\"list\") over {ROWS} texts: the report holds {} texts, the last {last:?}, {} bytes of text",
        sel.texts().len(), sel.text().len());
    let whole: usize = (0..ROWS).map(|i| format!("line {i}").len() + 1).sum::<usize>() - 1;
    println!("the whole selection would be {whole} bytes");
}
```

Output:

```
Tx::select_all_text("list") over 12000 texts: the report holds 10020 texts, the last Some("r10019"), 99109 bytes of text
the whole selection would be 120889 bytes
```

**Ask.** Mark a cut report (`TextSelection::truncated()`), or let `selection_copy` ask the app
for the rows it didn't build.

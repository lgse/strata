# 78. Inside a draggable parent, a child's cursor never shows

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C12-4 |
| Affects | C12 (PDF text over draggable pages) |
| Retires | C12's own I-beam: the view hit-tests its text layer on every pointer move (`PdfView`'s `hover`, `pdf_view/state.rs:776`, from `render.rs:157`) and sets one cursor on the whole view (`render.rs:120-124`), committing when it flips, in place of a text band with `Cursor::Text` inside the draggable pages |
| Checked | Re-checked at `2a44156` (2026-11-10): `doc_78` printed the Output block (`reverify/run-2a44156.txt`). By source: `Runtime::cursor` (`runtime/input/pointer.rs:381-405`) takes the held hover, else `selectable_at`, else `primary_target` (`runtime/input/hit.rs:162`), then walks up to the nearest cursor; `top_target` skips an element whose only handler is a hover change, so the page wins. Selectable text is the one exception (`selectable_at` first). Duplicates: none in the 222 issues (grep `primary_target`, cursor with parent, child, inherit). Gallery: `mise run test cursor` runs C12's three cursor tests (`the_cursor_follows_text_scrolled_under_a_still_pointer`, `the_cursor_is_worked_out_again_on_release`, `the_cursor_shows_which_a_drag_does_and_a_margin_drag_pans`), which pass on the workaround above. Earlier: repro run at `5d421fd` and `04b38ae`, same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** The cursor is the press target's, so a hoverable child with its own cursor inside a draggable parent shows the parent's

Context: Strata (lgse/strata), a file manager on Rune.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cursor, Cx, Drag, El, Env, Runtime, col, el};
fn view(_: &(), _: &mut Cx<()>) -> El {
    col().key("page").size(300.0, 200.0).cursor(Cursor::Grab).on_drag(|_: &mut (), _: &mut Drag, _| {})
        .child(el().key("text-band").size(300.0, 40.0).cursor(Cursor::Text).on_hover_change(|_: &mut (), _, _| {}))
}
fn main() {
    let mut rt = Runtime::new((), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.pointer_move(100.0, 20.0);
    rt.frame(0.016);
    println!("pointer over the text band (Cursor::Text) inside the page (Cursor::Grab): {:?}", rt.cursor());
}
```

Output: `pointer over the text band (Cursor::Text) inside the page (Cursor::Grab): Grab`.

`Runtime::cursor` (`runtime/input/pointer.rs:381`) takes the press target (`primary_target`,
`runtime/input/hit.rs:162`) and walks up from it; only selectable text is asked first.

**Ask.** The cursor of the innermost element under the pointer that sets one.

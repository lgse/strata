# 46. `on_press` also fires after the element's own drag

| | |
| --- | --- |
| Kind | New issue |
| Drafts | R-C4-1 |
| Affects | C4 (header sort click after a resize or reorder drag), C2 |
| Retires | C4's held-back sort click cleared in `ListTable::build` |
| Checked | Re-checked at `2a44156`: the repro's output in `run-2a44156.txt` matches the Output block. By source, `pointer_up` still queues `Press` whenever the release is inside the captured element, moved or not (`runtime/input/pointer.rs:347–348`; `moved` only clears the click run, `:336`, and gates the double press, `:350`). In the gallery, `mise run test list_table::tests::runtime` passed (15 tests, among them the held-back sort click's cases, such as `an_assistive_press_after_a_sortable_heading_released_on_itself_sorts`), and C4's held-back click is still there (`list_table/header.rs:239`, `list_table/state.rs:53`). Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output (`e96c55ee`'s press-run rule, past the pin, doesn't change it). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** An element with `on_drag` and `on_press` hears a press after its own drag past the threshold

Context: Strata (lgse/strata), a file manager on Rune.

`pointer_up` queues `Press` whenever the release is inside the captured element
(`runtime/input/pointer.rs:348` at `2a44156`, `:324` at `5d421fd`), so a header that both sorts on a click and
resizes or reorders on a drag also sorts at the end of every drag.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, Drag, El, Env, Runtime, col, el};
#[derive(Default)] struct S { log: Vec<String> }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("header").size(300.0, 40.0)
        .on_drag(|s: &mut S, d: &mut Drag, _| s.log.push(format!("drag {:?}", d.phase)))
        .on_press(|s: &mut S| s.log.push("press".into())))
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.pointer_move(50.0, 20.0);
    rt.pointer_down(50.0, 20.0);
    rt.pointer_move(75.0, 20.0);
    rt.pointer_move(100.0, 20.0);
    rt.pointer_up(100.0, 20.0);
    rt.frame(0.016);
    println!("{:?}", rt.state.log);
}
```

Output: `["drag Down", "drag Move", "drag Move", "drag Up", "press"]`.

**Ask.** No press after a drag that moved past the threshold, or a flag on `Drag` (at `Up`)
saying a press will follow.

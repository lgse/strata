# 14. File-drop hooks: acceptance with app state, the whole drag, enter and leave, the action

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U3 (W1; S5 index), U-C2-1 |
| Affects | W1's `Dnd::rune` binder, C1e, C2, C6, C7, C8, X2 |
| Retires | C1e's forwarded drag-over hook (`Collection::on_drag_over`), C8's folder check that waits for the drop, C7's tabs that open only after the drop |
| Checked | Re-checked at `2a44156`: `doc_14`'s output in `run-2a44156.txt` matches the Output block. By source, `accepts_files(Fn(&Path) -> bool)` (`el/drop.rs:70`), `on_drop_hover(Fn(&mut S, bool, &mut Tx))` (`:85`) and `on_drop_files(&[PathBuf])` with no action (`:63`) are unchanged, `Tx` has no `drag_paths`, and only the innermost zone is the target (`drop_target`, `runtime/input/drop.rs:173`). In the gallery, the `collection::element` tests (115, incl. the `on_drag_over` x2 cases) and `c2::`, `c7::`, `c8::` passed; C1e's `Collection::on_drag_over` (`collection/element/mod.rs:608`, U-C2-1) and `columns/mod.rs:342`'s U-C2-1 note are still there. Earlier: both repros ran at `5d421fd` and `04b38ae` with the same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** File drops: acceptance can't see app state or the whole drag, an ancestor can't follow it, and nothing says which action it gets

Context: Strata (lgse/strata), a file manager on Rune, decides while a drag hovers whether a folder
takes it (not a read-only folder, not a folder onto itself), copy or move by the source's volume,
and a column strip follows a drag across the columns inside it.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- `El::accepts_files(Fn(&Path) -> bool)` (`el/drop.rs:70`) is asked one path at a time, with no
  `&S`, so it can't read app state or judge the drag as a whole. It is also asked again each
  time Rune aims the drag.
- The paths reach state only with the drop: `on_drop_hover` gets `(S, bool, Tx)` and `Tx` has
  no `drag_paths()`.
- No drag identity and no window-level enter or leave.
- No action: the drop hands over paths with no copy or move, and the target can't pick one.
- Only the innermost zone under the drag hears it (`drop_target`, `runtime/input/drop.rs:173`);
  an ancestor with its own `on_drop_*` hears nothing until the drag reaches its own space.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, Runtime, col, el, row};
use std::path::PathBuf;
#[derive(Default)] struct S { log: Vec<String>, read_only: bool }
fn folder(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("zone").size(200.0, 150.0)
        // Asked per path, with no state: it can't see `read_only`, or the other paths.
        .accepts_files(|path| { println!("accepts_files({})", path.display()); true })
        .on_drop_hover(|s: &mut S, over, _| s.log.push(format!("on_drop_hover({over})")))
        .on_drop_move(|s: &mut S, p, _| s.log.push(format!("on_drop_move({}, {})", p.x, p.y)))
        .on_drop_files(|s: &mut S, files, _| s.log.push(format!("on_drop_files({files:?}), read_only {}", s.read_only))))
}
fn zone(key: &'static str) -> El {
    el().key(key).size(150.0, 150.0)
        .on_drop_hover(move |s: &mut S, over, _| s.log.push(format!("{key} hover {over}")))
        .on_drop_files(move |s: &mut S, _, _| s.log.push(format!("{key} drop")))
}
fn strip(_: &S, _: &mut Cx<S>) -> El {
    row().key("strip").size(400.0, 150.0)
        .on_drop_hover(|s: &mut S, over, _| s.log.push(format!("strip hover {over}")))
        .on_drop_move(|s: &mut S, p, _| s.log.push(format!("strip move {}", p.x)))
        .on_drop_files(|s: &mut S, _, _| s.log.push("strip drop".into()))
        .child(zone("column-a")).child(zone("column-b"))
}
fn main() {
    let mut rt = Runtime::new(S { read_only: true, ..S::default() }, folder, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let p = vec![PathBuf::from("/home/me/a.txt"), PathBuf::from("/home/me/b.png")];
    rt.drag_files(p.clone(), Some((10.0, 10.0)));
    rt.frame(0.02);
    rt.drag_files_to(Some((60.0, 70.0)));
    rt.frame(0.02);
    rt.drop_files(p, Some((60.0, 70.0)));
    rt.frame(0.02);
    for l in &rt.state.log { println!("{l}"); }

    let mut rt = Runtime::new(S::default(), strip, (400.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let p = vec![PathBuf::from("/tmp/a.txt")];
    rt.drag_files(p.clone(), Some((50.0, 50.0)));
    rt.frame(0.016);
    for x in [200.0, 350.0] { rt.drag_files_to(Some((x, 50.0))); rt.frame(0.016); }
    rt.drop_files(p, Some((350.0, 50.0)));
    rt.frame(0.016);
    println!("{:?}", rt.state.log);
}
```

Output: `accepts_files` five times per path, then

```
on_drop_hover(true)
on_drop_move(10, 10)
on_drop_move(60, 70)
on_drop_hover(false)
on_drop_files(["/home/me/a.txt", "/home/me/b.png"]), read_only true
["column-a hover true", "column-a hover false", "column-b hover true", "column-b hover false", "strip hover true", "strip move 350", "strip hover false", "strip drop"]
```

The read-only folder took the drop, and the strip heard nothing while the drag crossed its
columns.

**Ask.**
- An acceptance hook with `&S` and every dragged path at once
  (`accepts_drag(|s: &S, paths: &[PathBuf]| -> bool)`), or `Tx::drag_paths()` in
  `on_drop_hover`.
- A drag id, with window-level drag enter and leave.
- The drop's action: the source's allowed actions, and a way for the target to pick one while
  hovering.
- A passive drop-motion observer on an ancestor, heard alongside the target's (as GTK's
  `DropControllerMotion` on a scroller).

Related: (link to 06) (typed drops), #357 (in-app drop targets), #442 (headless drops lost
below 60 fps).

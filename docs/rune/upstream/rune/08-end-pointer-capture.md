# 08. No public way to end a pointer capture

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U10 (S5, from S2 R5) |
| Affects | P1 |
| Retires | P1's synthetic Escape through `Tx::send_key`, its proof checks, and the seven capture paths in `dnd/mod.rs` |
| Checked | Re-checked at `2a44156`: the repro's output in `run-2a44156.txt` matches the Output block. By source, `cancel_pointer` is still `pub(crate)` (`runtime/input/pointer.rs:106`), Rune still ends the capture itself only in `start_drag_out` (`:140`), and no `Tx` or `WindowHandle` method releases the pointer. `mise run test dnd::tests::` passed (15 tests: P1's Escape guard, which relies on Rune ending a held drag on Escape and on `Tx::send_key`). Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output (`pointer.rs:94` at the pin). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** No public way to end a pointer capture when a system drag takes the pointer

Context: Strata (lgse/strata), a file manager on Rune.

When an app starts a system drag from a press, the compositor takes the pointer: winit gets
`wl_pointer.leave` and never the release. Rune stays pressing, and `on_drag` keeps getting
moves with no button held. Rune ends the capture itself for macOS `drag_out`
(`start_drag_out`), but an app can't.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, Drag, El, Env, Phase, Runtime, col, el};
#[derive(Default)] struct S { phases: Vec<(Phase, bool)> }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("tile").size(100.0, 60.0)
        .on_drag(|s: &mut S, d: &mut Drag, _| s.phases.push((d.phase, d.cancelled()))))
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let (x, y) = rt.find("tile").unwrap();
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_move(x + 12.0, y); // the app starts a system drag here
    rt.pointer_leave(); // the compositor takes the pointer; no release follows
    rt.pointer_move(x + 40.0, y + 20.0); // back, no button held
    rt.frame(0.0);
    println!("pressing after the drag: {}", rt.pressing());
    println!("on_drag phases: {:?}", rt.state.phases);
    rt.window_focus(false); // the only public path that ends it
    rt.frame(0.0);
    println!("after window_focus(false): pressing {}, phases {:?}", rt.pressing(), rt.state.phases);
}
```

Output:

```
pressing after the drag: true
on_drag phases: [(Down, false), (Move, false), (Move, false)]
after window_focus(false): pressing false, phases [(Down, false), (Move, false), (Move, false), (Up, true)]
```

`Runtime::cancel_pointer` is `pub(crate)`, and a windowed app has no `Runtime`. Only a focus
loss ends the capture; sway sends one at the drop, other compositors may not.

**Ask.** `Tx::release_pointer()` (or `WindowHandle::cancel_pointer()`) that delivers the
cancelled `Up` and clears press and hover.

**Why.** Today a downstream crate sends Escape through `Tx::send_key`, which Rune takes as
"cancel the drag". An Escape that finds no capture reaches the app's keys, so it may only be sent
once the compositor proves it took the pointer, with the source still where it was pressed,
and several paths still leave the capture held until the next press.

Related: (link to 02), (link to 05).

# 49. A secondary press outside an open menu reaches nothing, its scrim included

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C21-6 (and C1e's `secondary_press_at` follow-up) |
| Affects | C21 (context menus), C1e, A6 (`dialogs_and_menus/secondary-click-retargets-menu/list`) |
| Retires | C21's "a right-click outside an open menu does nothing" (bullet 184 not met) |
| Checked | At `2a44156`, probe runs (`repros-head/src/bin/rv_49.rs` and `rv_49b.rs`, headless): with a kit `context_menu` open on row `a`, a right-click on row `b` outside it leaves the menu open on `a`, and `on_close` doesn't run; a left-click there closes it. `rv_49b` (2026-11-14) also gives the menu's scrim an `on_secondary_press` of its own, and it never runs. Source: `secondary_target` (`runtime/input/secondary.rs:9`) walks up from the topmost hit and stops at the first node that isn't `allowed()` (line 15), and `allowed()` (`runtime/input/focus.rs:100`) is false for anything outside the open modal's members (`modal_member`, line 69). rune-kit's `dismissal` (`rune-kit/src/menu.rs:267`) pushes the scrim into the layer before the menu's modal surface, so the scrim is outside the modal, as is everything under it. A primary press outside reaches the modal's own `outside` dismissal instead (`runtime/input/pointer.rs:197`); `secondary_down` has no such branch. `647cbbe2` (#499) adds `El::press_focuses_below` (`el.rs:1120`), which only hands a primary press's focus to the scroll region below (`runtime/input/pointer.rs:250`). Gallery: `a_right_click_outside_an_open_menu_leaves_it_open_on_its_target` passes (C21's bullet 184 is still not met). Earlier: by source at `5d421fd` and `04b38ae`. |
| Workaround | None in the app. A6 tried two on 2026-11-14: a secondary-press handler on the scrim (C21's `on_secondary_outside`, shelved, because the scrim is outside the modal and never hears the press), and a full-window modal wrapper holding the scrim (deferred: it moves C21's modality and its anchor placement, for one run). `secondary-click-retargets-menu/list` stays `pending` on this entry. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-14; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A secondary press outside an open menu neither dismisses it nor reaches anything, the menu's own scrim included

Context: Strata (lgse/strata), a file manager on Rune.

A right-click outside an open menu does nothing: the menu stays open on its old target, and the
item under the pointer can't open its own menu in the same click (as every desktop menu does).
A left-click there closes the menu, through the modal's outside dismissal
(`runtime/input/pointer.rs:197` at `2a44156`).

The secondary press has no such path. `secondary_target` (`runtime/input/secondary.rs:9`)
walks up from the topmost hit and stops at the first node that isn't `allowed()` (line 15),
and `allowed()` is false for every node outside the open modal (`modal_member`,
`runtime/input/focus.rs:69`). rune-kit's `dismissal` (`rune-kit/src/menu.rs:267`) pushes the
outside-press scrim into the layer before the menu's modal surface, so the scrim is outside the
modal. An app can't work around it by giving the scrim an `on_secondary_press`: the press
never reaches it.

**Repro** (headless; ran at `2a44156`):

```rust
use rune::{Cx, El, Env, PointerPress, Runtime, col, el};
use rune_kit::{Kit, MenuItem, context_menu};
#[derive(Default)] struct S { point: Option<PointerPress>, on: Option<&'static str>, closes: usize, scrim: usize }
fn row(key: &'static str, y: f32) -> El {
    el().key(key).absolute(0.0, y).size(400.0, 40.0)
        .on_secondary_press(move |s: &mut S, p, _| { s.point = Some(p); s.on = Some(key); })
}
fn view(s: &S, _: &mut Cx<S>) -> El {
    let k = Kit::neutral(true);
    let mut layer = Vec::new();
    let body = col().w_full().h_full().child(
        context_menu("menu", el().w_full().h_full().child(row("a", 0.0)).child(row("b", 200.0)), s.point)
            .item(MenuItem::new("Open"))
            .on_close(|s: &mut S| { s.point = None; s.closes += 1; })
            .build(&k, &mut layer));
    // The menu's outside-press scrim is the first element it pushed: give it a secondary
    // press handler, as an app working around the missing one would.
    if let Some(slot) = layer.first_mut() {
        let scrim = std::mem::replace(slot, el());
        *slot = scrim.on_secondary_press(|s: &mut S, _, _| s.scrim += 1);
    }
    rune_kit::root(body, layer)
}
fn settle(rt: &mut Runtime<S>) { for _ in 0..30 { rt.frame(0.016); } }
fn main() {
    let mut rt = Runtime::new(S::default(), view, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.secondary_down(20.0, 20.0); rt.secondary_up(20.0, 20.0);
    settle(&mut rt);
    rt.secondary_down(20.0, 220.0); rt.secondary_up(20.0, 220.0);
    settle(&mut rt);
    println!("right-click a, then b: menu open {}, on {:?}, closed {} times, scrim heard {}",
        rt.state.point.is_some(), rt.state.on, rt.state.closes, rt.state.scrim);
    rt.pointer_down(20.0, 220.0); rt.pointer_up(20.0, 220.0);
    settle(&mut rt);
    println!("then left-click b: menu open {}, closed {} times", rt.state.point.is_some(), rt.state.closes);
}
```

Output:

```text
right-click a, then b: menu open true, on Some("a"), closed 0 times, scrim heard 0
then left-click b: menu open false, closed 1 times
```

**Ask.** A secondary press outside an open modal that dismisses on an outside press dismisses
it, as a primary press does, and then reaches what is under it, so the element there can open
its own menu in the same click. Or let the scrim, or another element the app names, take the
secondary press while the modal is open.

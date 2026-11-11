# 49. A secondary press on a menu's outside scrim reaches nothing

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C21-6 (and C1e's `secondary_press_at` follow-up) |
| Affects | C21 (context menus), C1e |
| Retires | C21's "a right-click outside an open menu does nothing" (bullet 184 not met) |
| Checked | At `2a44156`, probe run (`repros-head/src/bin/rv_49.rs`, headless): with a kit `context_menu` open on row `a`, a right-click on row `b` outside it leaves the menu open on `a`, and `on_close` doesn't run; a left-click there closes it. Source: `secondary_target` still asks only the topmost hit and its ancestors (`runtime/input/secondary.rs:9`). `647cbbe2` (#499) adds `El::press_focuses_below` (`el.rs:1120`), which only hands a primary press's focus to the scroll region below (`runtime/input/pointer.rs:250`). rune-kit's `scrim` (`rune-kit/src/overlay.rs:131`) still has only `on_press` and doesn't use it. Gallery: `a_right_click_outside_an_open_menu_leaves_it_open_on_its_target` passes (C21's bullet 184 is still not met). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A secondary press on an open menu's outside scrim neither dismisses the menu nor reaches what's under it

Context: Strata (lgse/strata), a file manager on Rune.

A menu's outside-press catcher (rune-kit's `scrim`) has no `on_secondary_press`, and
`secondary_target` (`runtime/input/secondary.rs:9` at `2a44156`) asks only the topmost hit and
its ancestors. So a right-click outside an open menu does nothing: the menu stays open on its
old target, and the item under the scrim can't open its own menu in the same click (as every
desktop menu does). `El::press_focuses_below` (#499) hands a primary press's focus to what's
under a scrim; nothing does the same for a secondary press.

**Repro** (headless; ran at `2a44156`):

```rust
use rune::{Cx, El, Env, PointerPress, Runtime, col, el};
use rune_kit::{Kit, MenuItem, context_menu};
#[derive(Default)] struct S { point: Option<PointerPress>, on: Option<&'static str>, closes: usize }
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
    rune_kit::root(body, layer)
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.secondary_down(20.0, 20.0); rt.secondary_up(20.0, 20.0);
    for _ in 0..30 { rt.frame(0.016); }
    rt.secondary_down(20.0, 220.0); rt.secondary_up(20.0, 220.0);
    for _ in 0..30 { rt.frame(0.016); }
    println!("right-click a, then b: menu open {}, on {:?}, closed {} times", rt.state.point.is_some(), rt.state.on, rt.state.closes);
}
```

Output: `right-click a, then b: menu open true, on Some("a"), closed 0 times`. A left-click on `b`
closes the menu.

**Ask.** Let the scrim pass a secondary press through after dismissing (or dismiss and replay
it), as a primary press outside does.

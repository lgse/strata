# 28. A scoped reveal can't be kept to its own viewport's axis

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C2-4 |
| Affects | C2 (Miller columns), C1e (`set_own_viewport_reveals`) |
| Retires | C1e's own-viewport reveals by an offset computed from geometry, which give up Rune's following of a row layout moves |
| Checked | Re-checked at `2a44156` with a new headless probe, `rv_28` (the program in the body, C2's probe `p7` rebuilt): `scroll_into_view_in("c3", "row-3-2")` puts column 3 at x 900, then 600 from the next frame on, where the strip's own `scroll_to` glides 900, 888, 861, 827. By source, `Tx::scroll_into_view_in` (`el/tx/scroll.rs:24`) is still a reveal by key with a scope and no axis. In the gallery, `mise run test own_viewport_reveals_leave_a_sideways_strip` passed (1 test: C1e's own-viewport reveals, `collection/element/state.rs:412`, `collection/element/mod.rs:94`), and C2's notes are still there (`columns/mod.rs:120`, `:358`, `columns/state.rs:413`). Earlier: C2's probe `p7` at the pin; the signature was unchanged at `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `scroll_into_view_in(scope, key)` also scrolls a sideways viewport outside its scope, at once

Context: Strata's (lgse/strata, a file manager on Rune) Miller columns: vertical lists side by side
in a horizontally scrolled strip that glides to the column being worked in.

**What happens** (`5d421fd`, unchanged at `2a44156`). `Tx::scroll_into_view_in(scope, row)`
scrolls the nearest vertical viewport around the row and the nearest horizontal one too, even
when that one is outside `scope`, and at once in an animated commit. A list inside the strip can't reveal its row
without jumping the strip, and the strip can't glide there itself in the same frame.

**Repro** (headless; ran at `2a44156`): four 300 pt columns in a 900 pt `scroll_x` strip, each a
vertical viewport of virtual rows; reveal a row of column 3 already in view vertically, against
the strip's own `scroll_to` to the same place.

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, col, el, row};
#[derive(Default)]
struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(
        row().key("strip").size(900.0, 400.0).scroll_x().children((0..4).map(|c| {
            col().key(format!("c{c}")).size(300.0, 400.0).scroll(ScrollSpec { stick_bottom: false }).child(
                col().key(format!("list{c}")).w_full().growth(GrowthSpec::default())
                    .virtual_rows(100, 20.0, move |_: &S, i| format!("row-{c}-{i}").into(), |_: &S, _| el().w_full().h(20.0)))
        })))
}
fn x_of(rt: &Runtime<S>, key: &str) -> Option<f32> {
    rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).map(|e| e.rect[0])
}
fn run(name: &str, ask: impl FnOnce(&mut rune::Tx)) {
    let mut rt = Runtime::new(S, view, (900.0, 400.0), Env::new(true, 1.0));
    rt.frame(0.0);
    for _ in 0..5 { rt.frame(0.016); }
    let before = x_of(&rt, "c3");
    rt.update_if(|_, tx| { ask(tx); false });
    let mut xs = vec![before];
    for _ in 0..4 { rt.frame(0.016); xs.push(x_of(&rt, "c3")); }
    println!("{name}: column 3's drawn x {xs:?}");
}
fn main() {
    run("scroll_into_view_in(\"c3\", \"row-3-2\")", |tx| tx.scroll_into_view_in("c3", "row-3-2"));
    run("strip's own scroll_to(300)             ", |tx| tx.scroll_to("strip", 300.0, 0.0));
}
```

Output:

```
scroll_into_view_in("c3", "row-3-2"): column 3's drawn x [Some(900.0), Some(600.0), Some(600.0), Some(600.0), Some(600.0)]
strip's own scroll_to(300)             : column 3's drawn x [Some(900.0), Some(887.9531), Some(860.6094), Some(827.0781), Some(792.6719)]
```

**Ask.** A reveal limited to an axis, or to the viewports inside its scope.

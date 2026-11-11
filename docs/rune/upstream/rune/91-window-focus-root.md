# 91. `on_window_focus` is heard only on the root

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C6-6 |
| Affects | C6 (location bar), X1 (header's active state), gallery pages |
| Retires | Hosts forwarding window activation to each component |
| Checked | Re-checked at `2a44156` with a headless probe (`repros-head/src/bin/rv_91.rs`; the program below, `rv_91_body.rs`, printed the Output block as is): a child's `on_window_focus` hears nothing while the root's hears every change. By source: `Action::WindowFocus` runs only the app root's handler (`runtime/input/action.rs:306–311`), and the method's docs still say "On the root" (`el.rs:729–731`). Gallery: C6's host forwarding (`location_bar/mod.rs:26`, `:227–231`, `LocationBarState::set_window_focused` at `state.rs:310`) is still there, and its tests pass (`mise run test deactivation`: 2 tests in c6). Earlier: C6's probe P2b at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `on_window_focus` runs only on the app's root element

Context: Strata (lgse/strata), a file manager on Rune.

A component can't hear its window activate or deactivate: `on_window_focus` is heard only on the
root, so each host forwards it. (Scripts can deactivate the window: `{"t": .., "focus": false}`
reaches the root's handler.)

**Repro** (headless, at `2a44156`):

```rust
use rune::{Cx, El, Env, Runtime, col, el};
#[derive(Default)] struct S { heard: Vec<String> }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full()
        .on_window_focus(|s: &mut S, f, _| s.heard.push(format!("root {f}")))
        .child(el().key("location-bar").size(200.0, 30.0)
            .on_window_focus(|s: &mut S, f, _| s.heard.push(format!("location-bar {f}"))))
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.016);
    for focused in [true, false, true] {
        rt.window_focus(focused);
        rt.frame(0.016);
    }
    println!("on_window_focus heard: {:?}", rt.state.heard);
}
```

**Output:**

```
on_window_focus heard: ["root true", "root false", "root true"]
```

**Ask.** Call `on_window_focus` on every element that has one, or add `Cx::window_focused()`
that re-runs the view on change.

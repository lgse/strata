# 90. Arrow keys between a dialog's controls by position

| | |
| --- | --- |
| Kind | New issue (low priority) |
| Drafts | C15's "Up and Down between body controls" gap |
| Affects | C15 (dialogs) |
| Retires | C15's partial bullet "With focus outside a text field, an unmodified arrow key…" |
| Checked | Re-checked at `2a44156` with a headless probe (`repros-head/src/bin/rv_90.rs`; the program below, `rv_90_body.rs`, printed the Output block as is): Down and Up leave focus on a body checkbox, and in the footer's `focus_group` Up moves to the other button by order, wrapping. By source: arrows move focus only inside a `focus_group`, by order (`runtime/input/focus.rs:460–475`), and a group's Tab visits only its current item (`El::focus_group`, `el.rs:1026–1031`); nothing moves by position. Gallery: C15's partial bullet is still there (`dialog/mod.rs:93–95`, coverage row `:128`); `mise run test c15::` passes (13 tests, scripts `arrows` and `arrows-select` among them). Earlier: from C15's notes at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Up and Down don't move focus between a dialog's controls by position

Context: Strata (lgse/strata), a file manager on Rune.

In a GTK dialog an unmodified arrow key, with focus outside a text field, moves focus to the
nearest control in that direction. In Rune, arrows move along a `focus_group` (a dialog's
footer) by order, and Down opens a focused select, but Up and Down don't move between the body's
controls. Making the body a `focus_group` would take its controls out of Tab's order.

**Repro** (headless, at `2a44156`):

```rust
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col, el, row};
struct S;
fn control(key: &str, role: &'static str) -> El {
    el().key(key).size(160.0, 24.0).role(role).label(key).focusable(true).on_press(|_: &mut S| {})
}
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(
        col().key("dialog").role("dialog").label("Preferences").size(300.0, 200.0).modal(true, |_: &mut S| {})
            .child(control("show-hidden", "checkbox").autofocus())
            .child(control("folders-first", "checkbox"))
            .child(row().focus_group().child(control("cancel", "button")).child(control("ok", "button"))),
    )
}
fn main() {
    let mut rt = Runtime::new(S, view, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.016);
    rt.frame(0.016);
    let focused = |rt: &Runtime<S>| rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key).unwrap_or_default();
    let mut log = vec![format!("start {}", focused(&rt))];
    for (name, key) in [("Down", Key::Down), ("Tab", Key::Tab), ("Up", Key::Up), ("Tab", Key::Tab), ("Up", Key::Up)] {
        rt.key(KeyEvent { key, mods: Mods::default() });
        rt.frame(0.016);
        log.push(format!("{name} -> {}", focused(&rt)));
    }
    println!("{}", log.join(", "));
}
```

**Output:**

```
start show-hidden, Down -> show-hidden, Tab -> folders-first, Up -> folders-first, Tab -> cancel, Up -> ok
```

**Ask.** An opt-in spatial arrow navigation for a subtree (`El::arrow_focus(Spatial)`), or a
`focus_group` mode that moves by position in two dimensions.

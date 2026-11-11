# 88. Closing a modal returns focus to an opener that became disabled

| | |
| --- | --- |
| Kind | New issue |
| Drafts | C15's "modal restore to a disabled opener" gap |
| Affects | C15 (dialogs) |
| Retires | C15's "Rune gap … or Strata policy with `return_focus`" bullet |
| Checked | Re-checked at `2a44156` with a headless probe (`repros-head/src/bin/rv_88.rs`; the program below, `rv_88_body.rs`, printed the Output block as is): after Reset disables Reset all and closes the dialog, focus is on Reset all, which isn't focusable. By source: the modal focus machine (`22aa8cde`, `dece2aff`) returns focus with `ModalEffect::Return`, which drops only an opener that has gone (`runtime/input/modal_focus.rs:139–141`), and `set_focus` doesn't check `can_focus` (`runtime/input/focus.rs:232`). `2a44156b` (#510) changes only `Tx::return_focus` yielding to a `Tx::focus` in the same change (`el/tx.rs:372`), which a modal's own restore doesn't use. Gallery: C15's known gap and `StrataDialog::return_focus` are still there (`dialog/mod.rs:96–99`); `mise run test c15::` passes (13 tests). Earlier: by source at `5d421fd` (`focus.rs:440–443`). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A modal whose action disables its opener returns focus to that disabled opener

Context: Strata (lgse/strata), a file manager on Rune.

A settings page's "Reset all" opens a confirmation; its Reset disables the "Reset all" button.
When the dialog closes, Rune returns focus to that now-disabled button: the modal restore drops
only an opener that has gone (`ModalEffect::Return`, `runtime/input/modal_focus.rs:140` at
`2a44156`), and `set_focus` takes it without checking that it can take focus.

**Repro** (headless, at `2a44156`):

```rust
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col, el};
#[derive(Default)] struct S { open: bool, reset_done: bool }
fn button(key: &str) -> El { el().key(key).size(120.0, 30.0).role("button").label(key).focusable(true) }
fn view(s: &S, _: &mut Cx<S>) -> El {
    let mut root = col().w_full().h_full()
        .child(button("before").on_press(|_: &mut S| {}))
        .child(button("reset-all").disabled(s.reset_done).on_press(|s: &mut S| if !s.reset_done { s.open = true }));
    if s.open {
        root = root.child(col().key("confirm").size(200.0, 100.0).role("dialog").label("Reset everything?")
            .modal(true, |s: &mut S| s.open = false)
            .child(button("reset").on_press(|s: &mut S| { s.reset_done = true; s.open = false; })));
    }
    root
}
fn focused(rt: &Runtime<S>) -> String {
    rt.elements().into_iter().find(|e| e.focused)
        .map_or("nothing".into(), |e| format!("{} (focusable: {})", e.key.unwrap_or_default(), e.focusable))
}
fn press(rt: &mut Runtime<S>, key: Key) {
    rt.key(KeyEvent { key, mods: Mods::default() });
    for _ in 0..30 { rt.frame(0.016); }
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.016);
    press(&mut rt, Key::Tab);
    press(&mut rt, Key::Tab);
    press(&mut rt, Key::Text(" ".into())); // Reset all: the dialog opens on Reset
    println!("dialog open: focus on {}", focused(&rt));
    press(&mut rt, Key::Text(" ".into())); // Reset: disables Reset all, closes the dialog
    println!("dialog closed: focus on {}", focused(&rt));
}
```

**Output:**

```
dialog open: focus on reset (focusable: true)
dialog closed: focus on reset-all (focusable: false)
```

**Ask.** Treat a disabled (or inert) opener as gone, and fall back as for a removed one.

# 09. Comment on #106: focus change, focus within, and a field's blur

| | |
| --- | --- |
| Kind | Comment on [#106](https://github.com/anomalyco/rune/issues/106) (`El::on_focus_change`) |
| Drafts | U-C6-3, U-C16-4, U-C1e-4 (its notification half), C9's styled focus-within draft |
| Affects | C1e, C5, C6, C9, C16, C18, C20a, C20b |
| Retires | Each host's per-frame poll comparing `TextEdit::focused` (C6, C18, C20a, C20b, a frame late), C16's countdown that focus can't pause, C9's whole-pane ring |
| Checked | Re-checked at `2a44156`: the repro's output in `run-2a44156.txt` matches the line the comment quotes. By source, there is still no `on_focus_change` or focus-within callback, and `2a44156b` (#510, `Tx::return_focus`) adds no notification. `focus_within_ring` and `focus_within_flat` (`el.rs:1143`, `:1153`, both at the pin too) style a ring and a raised edge only. #106 is still open with no comments (checked live 2026-11-10), so the comment adds to an empty thread. In the gallery, `mise run test typed_text_waits_while_the_field_menu_is_open_then_commits_on_the_blur` passed (1 test: C20a's blur noticed by its per-frame poll). The polls are still there (`filter_bar/field.rs:146`, `filter_bar/filter/state.rs:325`). Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output, and `Tx::focused` has been in every handler since `cd2bd55a`, which doesn't notify. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10). Post it as a comment on #106, which is still open. |

Paste from here.

---

Strata (lgse/strata), a file manager on Rune, needs this in several places, and two cases go beyond
a single element:

1. **A text field's blur.** A path bar, a rename field and a number field commit or cancel when
   the field loses focus. Rune clears `TextEdit::focused` without telling anyone, so each host
   compares it frame to frame and acts a frame late.
2. **Focus within a subtree.** A notification card pauses its countdown while the keyboard is
   inside it; a preview pane draws an accent edge while focus is inside it. Nothing hears focus
   enter or leave a subtree, and `on_hover_change` doesn't fire for `hover_group_focus`, though
   the hover visual turns on:

```rust
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col, el, row};
#[derive(Default)] struct S { log: Vec<String> }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full()
        .child(el().key("before").size(80.0, 30.0).focusable(true).on_press(|_: &mut S| {}))
        .child(row().key("card").size(200.0, 40.0).hover_group_focus()
            .on_hover_change(|s: &mut S, h, _| s.log.push(format!("card hover {h}")))
            .child(el().key("cancel").size(80.0, 30.0).role("button").label("Cancel").focusable(true).on_press(|_: &mut S| {})))
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    for _ in 0..2 { rt.key(KeyEvent { key: Key::Tab, mods: Mods::default() }); rt.frame(0.016); }
    let focused: Vec<_> = rt.elements().into_iter().filter(|e| e.focused).filter_map(|e| e.key).collect();
    println!("after two Tabs focus is on {focused:?}; the card's handlers heard {:?}", rt.state.log);
}
```

At `5d421fd` and `2a44156` this prints `after two Tabs focus is on ["cancel"]; the card's
handlers heard []`.

Could `on_focus_change` come with a `within` form (`El::on_focus_within(|s, bool, tx|)`), and
fire for a text field's blur as for any element? A focus-within style (a border colour, not
only the ring) would cover the pane case without a handler.

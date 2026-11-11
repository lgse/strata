# 03. rune-kit text fields drop handlers meant for their inner field

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C21-2, U-C5-1, R-C14b-3 |
| Affects | C5, C6, C14b, C17, C18, C19, C20a, C20b, C21, C26 |
| Retires | Each component's own frame around core `text_input`, in place of rune-kit `TextInput` |
| Checked | Re-checked at `2a44156`: the repro's output in `run-2a44156.txt` matches the Output block. By source, `TextInput::build` (`rune-kit/src/text_input.rs:300`) still passes `None` for `name_field`'s description (`:117`), and nothing in rune-kit's text fields forwards `on_secondary_press`. `TextField` has `error` (`:607`) but still no `on_submit`. `secondary_target` is at `runtime/input/secondary.rs:9`. In the gallery, `mise run test c5s_field_menu_holds_the_edit_open` passed (1 test: C21's menu on a core `text_input`), and C21's docs still list rune-kit fields as having no menu (`context_menu/mod.rs:56`, `:195`). Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit `TextInput`, `TextArea` and `TextField` drop `on_secondary_press` and `description` meant for their field

Context: Strata (lgse/strata), a file manager on Rune draws its own text-field context menu (Cut, Copy, Paste, Undo)
on Linux, and shows validation errors as a field's description.

**What's missing** (`5d421fd`, unchanged at `2a44156`). `TextInput::build`
(`rune-kit/src/text_input.rs:300`) returns the box around the field. Its builder forwards
`on_edit`, `on_submit`, `on_key` and `on_change` to the inner `text_input`, but not
`on_secondary_press` or a description (`name_field`, `text_input.rs:117`, takes one but the
builder passes none). A field's own menu wins over every ancestor's `on_secondary_press`
(`secondary_target`, `runtime/input/secondary.rs:9`), so a handler on the returned box never
runs, and a description lands on the box, not the field.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, PointerPress, Runtime, TextEdit, Tx, col, text_input};
use rune_kit::{Kit, TextInput};
struct S { kit_edit: TextEdit, core_edit: TextEdit, heard: Vec<&'static str> }
fn view(s: &S, _: &mut Cx<S>) -> El {
    let k = Kit::neutral(true);
    col().w_full().h_full().gap(20.0)
        .child(TextInput::new(&s.kit_edit).on_edit(|s: &mut S| &mut s.kit_edit).build(&k).key("kit")
            .description("Name is taken")
            .on_secondary_press(|s: &mut S, _: PointerPress, _: &mut Tx| s.heard.push("kit TextInput")))
        .child(text_input(&s.core_edit, k.ty.ui).key("core").w(200.0).on_edit(|s: &mut S| &mut s.core_edit)
            .description("Name is taken")
            .on_secondary_press(|s: &mut S, _: PointerPress, _: &mut Tx| s.heard.push("core text_input")))
}
fn main() {
    let edits = (TextEdit::new("a.txt"), TextEdit::new("b.txt"));
    let mut rt = Runtime::new(S { kit_edit: edits.0, core_edit: edits.1, heard: Vec::new() }, view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    for key in ["kit", "core"] {
        let (x, y) = rt.find(key).unwrap();
        rt.pointer_move(x, y);
        rt.secondary_down(x, y);
        rt.secondary_up(x, y);
        rt.frame(0.016);
    }
    println!("secondary press heard by: {:?}", rt.state.heard);
    let mut stack = vec![rt.accessibility_json()];
    while let Some(mut n) = stack.pop() {
        if let Some(kids) = n.get_mut("children").map(serde_json::Value::take) {
            stack.extend(kids.as_array().into_iter().flatten().rev().cloned());
        }
        if n.get("description").is_some() { println!("description on {} ({})", n["key"], n["role"]); }
    }
}
```

Output:

```
secondary press heard by: ["core text_input"]
description on "kit" ("group")
description on "core" ("multiline-text-input")
```

**Ask.** Forward `on_secondary_press` and a `description` (or `error`) to the inner field on
`TextInput`, `TextArea` and `TextField`, or expose the inner field's `El`. `TextField` has
`error` but no `on_submit`.

**Why.** On Linux a right-click in a rune-kit field shows nothing (see (link to 04)), and the
app can't attach its own menu, so every field that needs one is rebuilt on core `text_input`
inside the app's own frame, losing rune-kit's look and behaviour.

Related: #286 (`TextArea` has no `plain` mode).

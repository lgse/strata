# 21. A text input can't take another role: combobox, spin button

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C6-1, U-C20a-1; the rune-kit NumberField proposal (C20a) folded in as a follow-up |
| Affects | C6 (location bar), C17 (search palette), C18 (filter bar), C20a (number field) |
| Retires | The `combobox` group wrapped around C6's, C17's and C18's inputs; C20a's spin-button wrapper with `focusable(false)` and its own Page keys |
| Checked | Re-checked at `2a44156`: `doc_21`'s output in `run-2a44156.txt` matches the Output block. By source, `if cx.input { … }` still returns before the role is read (`a11y/roles.rs:24–28`), and a focused element's `focus.step` takes Left, Right, Up, Down, Page Up and Page Down (`runtime/input/focus.rs:446–452`); the body's `focus.rs:25` was `navigates`, corrected. Through the gallery, the accessibility goldens show the wrappers: `c17-idle` reads a `combo-box` holding a `text-input`, and `c20a-controls` a `spin-button` holding a `text-input` (`number_field/view.rs:213`, U-C20a-1; `filter_bar/prompt/view.rs:193`, U-C6-1); `c6::`, `c17::`, `c18::` and `c20a::` passed. Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A text input ignores `role(..)`: an editable combobox or a spin button can't be expressed, and `on_step` takes Left and Right from the caret

Context: Strata (lgse/strata), a file manager on Rune: its path bar is an editable combobox (ARIA
1.2: the input has the role, `expanded` and `controls`); its number fields are spin buttons.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- `a11y/roles.rs:24` maps any element with an input to a text input (or search or password
  input) before it reads the role, so `role("combobox")` and `role("spinbutton")` are dropped.
- `on_step` on a field runs on Left and Right too (the step keys,
  `runtime/input/focus.rs:446–452`, run before the editor), so the caret can't move.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, TextEdit, col, text_input};
struct S { size: TextEdit, place: TextEdit, steps: Vec<i32> }
fn view(s: &S, _: &mut Cx<S>) -> El {
    let k = rune_kit::Kit::neutral(true);
    col().w_full().h_full()
        .child(text_input(&s.size, k.ty.ui).key("size").on_edit(|s: &mut S| &mut s.size)
            .role("spinbutton").label("Size").range_value(4.0, 1.0, 16.0)
            .on_step(|s: &mut S, d, _| s.steps.push(d)))
        .child(text_input(&s.place, k.ty.ui).key("place").on_edit(|s: &mut S| &mut s.place)
            .role("combobox").label("Location").expanded(true))
}
fn main() {
    let mut size = TextEdit::new("4");
    size.focused = true;
    let mut rt = Runtime::new(S { size, place: TextEdit::new("/home"), steps: Vec::new() }, view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(0.016);
    let mut stack = vec![rt.accessibility_json()];
    while let Some(mut n) = stack.pop() {
        if let Some(kids) = n.get_mut("children").map(serde_json::Value::take) {
            stack.extend(kids.as_array().into_iter().flatten().cloned());
        }
        if matches!(n["key"].as_str(), Some("size" | "place")) { println!("{} role {}", n["key"], n["role"]); }
    }
    let caret = rt.state.size.focus;
    rt.key(KeyEvent { key: Key::Left, mods: Mods::default() });
    rt.frame(0.016);
    println!("Left in the field: on_step heard {:?}; caret {} -> {}", rt.state.steps, caret, rt.state.size.focus);
}
```

Output:

```
"place" role "multiline-text-input"
"size" role "multiline-text-input"
Left in the field: on_step heard [-1]; caret 1 -> 1
```

**Ask.**
- A field whose role is `combobox` maps to `Role::ComboBox`, and `spinbutton` to
  `Role::SpinButton`, keeping its text value beside its range, `expanded` and `controls`.
- `on_step` on a field steps on Up, Down, Page Up and Page Down only.

With that, a rune-kit `NumberField` (min, max, step, page, decimals, a unit, − and + in the box)
would be small.

Related: #149 (rune-kit Slider).

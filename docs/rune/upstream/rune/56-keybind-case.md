# 56. rune-kit `Keybind` upper-cases single characters

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C18-1 |
| Affects | C18 (chord panel), C8 (sidebar keycaps), C19 |
| Retires | C18's own keycap chip |
| Checked | At `2a44156`, repro re-run (`doc_56`): same output. Source: `keycap` still upper-cases any single character that maps back (`rune-kit/src/keybind.rs:54`). Gallery: C18's own keycap is still there (`filter_bar/keycap.rs:5`); the `filter_bar` tests pass. Earlier: repro run at `5d421fd` and `04b38ae`, same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit `Keybind` draws "g" as "G", with no way to keep the case

Context: Strata (lgse/strata), a file manager on Rune.

Vim-style key maps tell `g` from `G` (Shift+g). `keycap` (`rune-kit/src/keybind.rs:54` at `2a44156`)
upper-cases any single character, so a keycap for `g` shows the wrong key.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, Runtime, col};
use rune_kit::{Keybind, Kit};
fn view(_: &(), _: &mut Cx<()>) -> El {
    let k = Kit::neutral(true);
    col().w_full().h_full().child(Keybind::new(&["g"]).build(&k)).child(Keybind::new(&["Shift", "g"]).build(&k))
}
fn main() {
    let mut rt = Runtime::new((), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let texts: Vec<_> = rt.elements().into_iter().filter_map(|e| e.text).collect();
    println!("Keybind::new(&[\"g\"]) and Keybind::new(&[\"Shift\", \"g\"]) draw {texts:?}");
}
```

Output: `Keybind::new(&["g"]) and Keybind::new(&["Shift", "g"]) draw ["G", "Shift", "G"]`.

**Ask.** `Keybind::keep_case()`, or upper-case only keys named as letter keys rather than typed
characters.

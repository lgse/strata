# 53. `on_step`'s Page Up and Page Down are always 10 steps

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C13-2 |
| Affects | C13 (media timeline: 5 s steps, 30 s pages), C20a (slider: Strata pages 4 steps) |
| Retires | C13's and C20a's Page keys taken in `on_key` before `on_step` |
| Checked | At `2a44156`, repro re-run (`doc_53`): same output. Source: `PageDown => Some(-10), PageUp => Some(10)` (`runtime/input/focus.rs:449`); no `range_page` or other page count. Gallery: C20a's and C13's Page keys still go through `on_key` (`controls/slider/mod.rs:13`, `media_player/keys.rs:173`); `a_page_is_whole_steps` and the `page` tests pass. Earlier: repro run at `5d421fd` (`focus.rs:376`) and `04b38ae`, same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `on_step` pages are always 10 steps; add `El::range_page`

Context: Strata (lgse/strata), a file manager on Rune.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col, el};
#[derive(Default)] struct S { steps: Vec<i32> }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("timeline").size(200.0, 20.0).role("slider").label("Position")
        .range_value(60.0, 0.0, 600.0).range_step(5.0).focusable(true)
        .on_step(|s: &mut S, d, _| s.steps.push(d)))
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let (x, y) = rt.find("timeline").unwrap();
    rt.pointer_move(x, y); rt.pointer_down(x, y); rt.pointer_up(x, y); rt.frame(0.016);
    for key in [Key::Up, Key::PageUp, Key::PageDown] { rt.key(KeyEvent { key, mods: Mods::default() }); rt.frame(0.016); }
    println!("Up, Page Up, Page Down -> on_step heard {:?}", rt.state.steps);
}
```

Output: `Up, Page Up, Page Down -> on_step heard [1, 10, -10]`. A media timeline that steps 5 s
and pages 30 s would move 50 s a page.

**Ask.** `El::range_page(steps)` (default 10) for the Page keys' step count, also reported to
assistive technology where it has a page increment.

Related: #149 ((link to 54)).

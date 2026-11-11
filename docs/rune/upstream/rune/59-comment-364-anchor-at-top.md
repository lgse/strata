# 59. Comment on #364: at offset 0, rows inserted above scroll out of view

| | |
| --- | --- |
| Kind | Comment on [#364](https://github.com/anomalyco/rune/issues/364) (virtual list: a glide's target and the anchor, and anchoring on a row the app marks) |
| Drafts | U-C17-3 |
| Affects | C17 (search palette streaming better results first) |
| Retires | C17's rows keyed by place so the anchored key keeps its place |
| Checked | At `2a44156`, repro re-run (`doc_59`): same output. #364 open, no comments (live, 2026-11-10). `El::scroll_anchor`'s docs say the same (`el/scroll.rs:134`–`141`). Gallery: C17's rows keyed by place are still there (`search_palette/mod.rs:68`); the `search_palette` tests pass. Earlier: repro run at `5d421fd` and `04b38ae`, same output (docs at `el/scroll.rs:126`–`134` at the pin). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

A related case from Strata (lgse/strata), a file manager on Rune: a search list that streams better
results in at the top. At offset 0, `virtual_rows` still holds the first visible row in place when
rows are inserted above it, so the new rows scroll out above the viewport though the user never
scrolled. `El::scroll_anchor`'s docs (`el/scroll.rs:134–141` at `2a44156`) say anchoring holds "only
while the viewport is … scrolled off its top … at the very top new content above shows", and that
`virtual_rows` lists anchor "this way".

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, col, el};
struct S { front: usize }
fn key(s: &S, i: usize) -> String { if i < s.front { format!("a{i}") } else { format!("b{:02}", i - s.front) } }
fn view(s: &S, _: &mut Cx<S>) -> El {
    col().key("vp").size(300.0, 200.0).scroll(ScrollSpec { stick_bottom: false }).child(
        col().w_full().growth(GrowthSpec::default())
            .virtual_rows(s.front + 30, 20.0, |s: &S, i| key(s, i).into(), |_: &S, _| el().w_full().h(20.0)))
}
fn main() {
    let mut rt = Runtime::new(S { front: 0 }, view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(0.016);
    rt.update(|s, _| s.front = 2);
    for _ in 0..30 { rt.frame(0.016); }
    let y = |k: &str| rt.elements().into_iter().find(|e| e.key.as_deref() == Some(k)).map(|e| e.rect[1]);
    println!("two rows inserted above at offset 0: a0 at {:?}, b00 at {:?}", y("a0"), y("b00"));
}
```

prints `two rows inserted above at offset 0: a0 at Some(-40.0), b00 at Some(0.0)` at `5d421fd`
and `2a44156`. Either skipping the anchor at offset 0, as the docs say, or the opt-out this issue
asks for (anchor on a row the app marks, or none) would cover it.

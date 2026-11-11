# 51. rune-kit scroll bars over viewports keyed alike share state and drag the first

| | |
| --- | --- |
| Kind | New issue |
| Drafts | R-C24-1 |
| Affects | C24 (its own overlay bar, a deviation from the plan) |
| Retires | One reason C24 keeps its own scroll bar instead of rune-kit's |
| Checked | At `2a44156`, probe run (`repros-head/src/bin/rv_51.rs`, headless, the body's repro): dragging `b`'s thumb scrolls `a` to 11,381 and leaves `b` at 2,000. Source: the area is still `"{key}-scroll-area"` (`El::wrap_scroll_area`, `rune/src/el/layout.rs:733`), kept by that name (`kept(area)`, `rune-kit/src/scroll_bar.rs:782`), and the drag's target is `"{area}/{key}"` (`:784`). `96def2a2` makes a press one grip (`scroll_bar/press.rs`) but keeps the keys. Gallery: C24's own bar and its R-C24-1 note are still there (`scroll/mod.rs:107`, `:155`). Earlier: by source at `5d421fd` (`:799`) and `04b38ae` (`:757`), and C24's probe at the pin (`a` went to 7,956 in that probe's layout). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit `ScrollBar`: two viewports with the same key share a bar's state, and dragging one scrolls the first

Context: Strata (lgse/strata), a file manager on Rune.

`ScrollBar::scroll_bar_styled` keys its kept state by `"{key}-scroll-area"` (the area
`El::wrap_scroll_area` names, `rune/src/el/layout.rs:733`; `kept(area)`,
`rune-kit/src/scroll_bar.rs:782` at `2a44156`) and drags the target `"{area}/{key}"` (`:784`),
which resolves to the first match in tree order.

**Repro** (headless; ran at `2a44156`): two panes keyed `a` and `b`, each holding a 1,000-row
virtual viewport keyed `viewport`.

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, WheelEvent, col, el, row};
use rune_kit::{Kit, ScrollBar, ScrollBarStyle};
fn pane(key: &str, k: &Kit) -> El {
    col().key(key).size(200.0, 300.0).child(
        col().key("viewport").w_full().h_full().scroll(ScrollSpec { stick_bottom: false })
            .child(col().w_full().growth(GrowthSpec::default())
                .virtual_rows(1000, 20.0, |_: &(), i| format!("r{i}").into(), |_: &(), _| el().w_full().h(20.0)))
            .scroll_bar_styled(ScrollBarStyle::scroll_view(k)))
}
fn view(_: &(), _: &mut Cx<()>) -> El {
    let k = Kit::neutral(true);
    row().w_full().h_full().child(pane("a", &k)).child(pane("b", &k))
}
fn offsets(rt: &Runtime<()>) -> Vec<(String, f32)> {
    let es = rt.elements();
    es.iter().enumerate().filter(|(_, e)| e.key.as_deref() == Some("viewport"))
        .map(|(i, e)| (e.path.clone(), es.iter().find(|c| c.parent == Some(i)).map_or(f32::NAN, |c| c.scrolled[1]))).collect()
}
fn main() {
    let mut rt = Runtime::new((), view, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    for _ in 0..16 { rt.wheel_event(WheelEvent::new(0.0, -125.0).at(300.0, 150.0)); rt.frame(0.016); }
    for _ in 0..60 { rt.frame(0.016); }
    println!("after wheeling b: {:?}", offsets(&rt));
    rt.pointer_move(395.0, 150.0);
    for _ in 0..30 { rt.frame(0.016); }
    let thumb = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("thumb") && e.path.starts_with("#0/b/")).unwrap();
    let (x, y) = (thumb.rect[0] + thumb.rect[2] / 2.0, thumb.rect[1] + thumb.rect[3] / 2.0);
    rt.pointer_move(x, y); rt.frame(0.016);
    rt.pointer_down(x, y); rt.frame(0.016);
    for i in 1..=12 { rt.pointer_move(x, y + 10.0 * i as f32); rt.frame(0.016); }
    rt.pointer_up(x, y + 120.0);
    for _ in 0..30 { rt.frame(0.016); }
    println!("after dragging {} down 120 pt: {:?}", thumb.path, offsets(&rt));
}
```

Output (`RUNE_PERF_WARN=0`):

```text
after wheeling b: [("#0/a/viewport-scroll-area/viewport", 0.0), ("#0/b/viewport-scroll-area/viewport", 2000.0)]
after dragging #0/b/viewport-scroll-area/scroll-bar/thumb-slot/thumb down 120 pt: [("#0/a/viewport-scroll-area/viewport", 11380.953), ("#0/b/viewport-scroll-area/viewport", 2000.0)]
```

Dragging `b`'s thumb scrolls `a`, and `b` stays where the wheel left it.

**Ask.** Key the state and the target by the viewport's path, or take an explicit key.

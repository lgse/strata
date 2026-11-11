# 50. A second `on_scroll_offset` replaces the first; chaining is hidden

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C24b-1 |
| Affects | C4 (sideways scroll followers), C22 (folder peek), C24b |
| Retires | C4's single handler that calls the host's followers (`ListTable::follow_sideways`) |
| Checked | At `2a44156`, repro re-run (`doc_50`): same output. Source: `on_scroll_offset` still sets the one handler (`el/layout_watch.rs:148`) and `chain_scroll_offset` is still `#[doc(hidden)]` (`:153`–`154`). `9edc78a1` only changes how the runtime tells a new handler from the one it last told (a weak reference instead of an address, `runtime/scroll/watch.rs`), not what a second call does. Gallery: C4's single handler is still there (`list_table/mod.rs:598`), and `a_sideways_scroll_reaches_both_bars_and_every_follower_…` and `a_peek_whose_anchor_moved_sideways_closes` pass. Earlier: repro run at `5d421fd` and `04b38ae`, same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `El::on_scroll_offset` replaces an earlier one, and the chaining rune-kit uses is `#[doc(hidden)]`

Context: Strata (lgse/strata), a file manager on Rune.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, Runtime, ScrollPos, WheelEvent, col, el};
use std::sync::atomic::{AtomicUsize, Ordering};
static A: AtomicUsize = AtomicUsize::new(0);
static B: AtomicUsize = AtomicUsize::new(0);
fn view(_: &(), _: &mut Cx<()>) -> El {
    col().w_full().h_full().child(col().size(300.0, 200.0).scroll_x()
        .on_scroll_offset(|_: ScrollPos| { A.fetch_add(1, Ordering::Relaxed); })
        .on_scroll_offset(|_: ScrollPos| { B.fetch_add(1, Ordering::Relaxed); })
        .child(el().w(3000.0).h(200.0)))
}
fn main() {
    let mut rt = Runtime::new((), view, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.wheel_event(WheelEvent::new(-120.0, 0.0).at(150.0, 100.0));
    for _ in 0..30 { rt.frame(0.016); }
    println!("first handler ran {} times, second {} times", A.load(Ordering::Relaxed), B.load(Ordering::Relaxed));
}
```

Output: `first handler ran 0 times, second 2 times`.

**Ask.** Make `on_scroll_offset` additive, or make `chain_scroll_offset` public, as rune-kit
relies on it. Components that follow a scroll (a sticky overlay, a peek card) and their hosts
then don't have to agree on one owner.

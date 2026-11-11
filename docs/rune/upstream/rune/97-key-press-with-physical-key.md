# 97. A host that owns a `Runtime` cannot send a key press with its physical key

| | |
| --- | --- |
| Kind | New issue |
| Drafts | V8 of the vertical slice (`docs/rune/slice/V8-read-path-parity.md`), from V7's finding |
| Affects | `strata-app`'s QA backend (`crates/strata-app/src/qa/`), which drives an offscreen `Runtime` as a window would |
| Retires | The parity ledger's `rune` blocker on `arrow-keys-move-focus/columns-hjkl-tenxer-ru`: a plain-letter binding under a non-Latin layout |
| Checked | At `2a44156` (2026-11-11): the repro below was built against the pin and run headless, and its output is the Output block. By source, `Runtime::physical_key_input` is `pub(crate)` (`runtime/input/physical.rs:54`) and only the script player calls it (`tooling.rs:372`); `Runtime::physical_input` is public and queues the physical event "without a logical fallback" (`runtime/input/physical.rs:30–36`); `Runtime::key` queues the logical key with no physical key and no base. |
| Status | Filed on 2026-11-11 as [anomalyco/rune#657](https://github.com/anomalyco/rune/issues/657), by the owner's request, from the text below without its last line, the placeholder link to entry 44. Entry [44](44-headless-tooling.md) asks for the same entry point among five other things, for scripts: when it is filed, link #657 from its fourth item |

Paste from here.

---

**Title:** `Runtime`: no public way to send a key press with its physical key and base character

Context: Strata (lgse/strata), a file manager on Rune, runs its end-to-end scenarios against an
offscreen `Runtime` that the test host owns and steps itself: it injects input at the runtime
and reads `accessibility_json`. It has no window and plays no Rune script.

**Repro** (headless; ran at `2a44156`):

```rust
use rune::{Cx, El, Env, Key, KeyEvent, Keybind, Mods, Runtime, Tx, col};
#[derive(Default)] struct S { seen: u32, matched: u32 }
fn view(_: &S, _: &mut Cx<S>) -> El { col().w_full().h_full() }
fn on_key(s: &mut S, ev: &KeyEvent, tx: &mut Tx) -> bool {
    s.seen += 1;
    if Keybind::new("j").matches_tx(ev, tx) { s.matched += 1; }
    println!("key {:?}: physical_key {:?}, key_base {:?}", ev.key, tx.physical_key(), tx.key_base());
    true
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.on_key = Some(on_key);
    rt.frame(0.0);
    // The J key under a Russian layout types "о". A window sends the press with its physical
    // key (`window/keys.rs`), and "j" matches by it. `Runtime::key` is all a host that owns
    // the `Runtime` has.
    rt.key(KeyEvent { key: Key::Text("о".into()), mods: Mods::default() });
    rt.frame(0.016);
    println!("keys seen {}, matched \"j\" {}", rt.state.seen, rt.state.matched);
}
```

Output:

```
key Text("о"): physical_key None, key_base None
keys seen 1, matched "j" 0
```

In a window the same press reaches the handler with `Tx::physical_key()` set, and
`Chord::matches_tx` falls back on it for a layout whose characters are not ASCII, so a plain
`j` binding matches under a Russian layout. A host that owns the `Runtime` can only call
`Runtime::key`, which carries neither the physical key nor the base, or
`Runtime::physical_input`, which queues the physical event without the logical key. The entry
that pairs them, `Runtime::physical_key_input(event, key, base)`, is `pub(crate)`; a script's
`{"type": "key", "code": …}` reaches it, an embedder cannot.

**Ask.** Make `Runtime::physical_key_input` public, or add a public
`Runtime::key_with(event: KeyEvent, physical: PhysicalKeyCode, base: Option<char>)`, so a
headless host can deliver a press exactly as `window/keys.rs` does.

Related: (link to 44).

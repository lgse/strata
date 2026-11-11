# 98. The Escape that breaks off a dead-key sequence reaches a text field as a key press

| | |
| --- | --- |
| Kind | New issue |
| Drafts | V9 of the vertical slice (`docs/rune/slice/V9-rename-parity.md`) |
| Affects | C5's inline rename editor as `strata-app` hosts it (`crates/strata-app/src/rename.rs`), and every field that closes on Escape: C6's location entry, C18's filter and prompts |
| Retires | The parity ledger's `rune` blocker on `tone-mark-escape-before-rename/columns` |
| Checked | By source only, at `2a44156` with winit 0.30.13 (2026-11-11). winit feeds every press to xkbcommon's compose state (`KeyEventResults::new`, `platform_impl/linux/common/xkb/mod.rs`), and `KeyEventResults::key` returns a named key such as Escape at once, whatever the compose state made of it. Rune maps it to `Key::Escape` (`window/keys.rs`, `map_key`) and sends it to the focused field (`window/events.rs`, `WindowEvent::KeyboardInput`). A dead key itself arrives as `Key::Dead`, which `map_key` maps to nothing. No input-method event is involved, so `TextEdit::is_composing` is false throughout. Not run in a window. The slice's offscreen host models the same order with xkbcommon's compose state (`crates/strata-app/src/qa/compose.rs`), and there the scenario stops where this predicts |
| Status | Drafted, not filed. It has no headless repro and cannot have one: `Runtime::key` takes what winit has composed already, and a `Runtime` has no dead-key input. Run the window repro below before filing, and say so in the issue |

Paste from here.

---

**Title:** A text field hears the Escape that only breaks off a dead-key sequence

Context: Strata (lgse/strata), a file manager on Rune, renames a file in a `text_input`
whose `on_input_key` closes the editor on Escape and keeps the old name. Its GTK
application runs the same scenario under a layout with dead keys, where GTK's input method
takes that Escape: it ends the pending dead key and nothing else.

**Repro** (in a window on Linux; a layout with dead keys, such as `us(intl)` or
`cn(altgr-pinyin)`):

```rust
use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx, col, text_input, tok};
struct S { edit: TextEdit, escapes: u32 }
fn view(s: &S, _: &mut Cx<S>) -> El {
    col().child(
        text_input(&s.edit, tok::type_::BODY).single_line().key("name").w(300.0)
            .on_edit(|s: &mut S| &mut s.edit)
            .on_input_key(|s: &mut S, ev: &KeyEvent, _: &mut Tx| {
                if ev.key == Key::Escape { s.escapes += 1; println!("Escape reached the field"); }
                false
            }),
    )
}
```

1. Focus the field and press a dead key (`'` under `us(intl)`; AltGr+3 under
   `cn(altgr-pinyin)`). Nothing is typed, and no handler runs: winit reports `Key::Dead`.
2. Press Escape.

Expected: the pending dead key is dropped and the field hears no key, as in GTK, Qt and a
browser, where Escape with a dead key pending ends the composition only. A field that
closes on Escape stays open.

By the source, what happens: the Escape that cancelled the sequence is delivered as
`Key::Escape`, so `on_input_key` runs and "Escape reached the field" prints. An inline
editor closes and drops what was typed. `TextEdit::is_composing()` is false at that point,
since a dead key is no input-method composition, so a handler cannot tell this Escape from
any other.

**Ask.** One of:

- Rune's window host swallows the named key press that broke off a dead-key sequence. It
  sees the sequence start (`Key::Dead` from winit) and can track that one is pending until
  the next press.
- Or a pending dead key counts as a composition: `TextEdit::is_composing()` is true while
  one is pending, and Rune gives it Enter and Escape as it gives them to an input method's
  preedit.

Related: (link to 97, the same host's other gap in what a key press carries).

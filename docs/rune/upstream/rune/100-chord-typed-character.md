# 100. A chord's handler gets the US key in place of the character typed, and the character is gone

| | |
| --- | --- |
| Kind | New issue |
| Drafts | The review of B3's registry, lgse/strata#2210 (`docs/rune/parity/owner.md`, B3, the eighth key case) |
| Affects | `strata-app`'s press builder (`crates/strata-app/src/commands/press.rs`), which reads every key Strata's shortcuts match |
| Retires | The builder's guess from `Tx::key_base()` at whether Rune replaced the character (`as_laid_out`), and the deviation where it cannot tell |
| Checked | By source only, at `2a44156` (2026-11-12): `map_key_event` in `window/keys.rs` replaces `Key::Command(c)` by `us_char(code)` whenever `c` is not ASCII, off macOS, and nothing keeps `c`. `Tx::key_base()` is the key's unmodified character, not the one typed. |
| Status | Drafted, not filed. It has no headless repro: `Runtime::key` takes the key already mapped. Run it in a window before filing, under a French (AZERTY) or Polish layout, with a handler that prints the key and `Tx::key_base()`. Workaround in force: where the base is not ASCII the builder takes the base for the character typed; where it is ASCII it reads the US key |

Paste from here.

---

**Title:** A chord's key event loses the character typed when Rune puts the US key in its place

Context: Strata (lgse/strata), a file manager on Rune, matches a shortcut by the character
the key types, then by the key's position, with its own rules for the Shift a symbol
consumes and for the levels AltGr selects.

**What happens.** Off macOS, when a key pressed with Ctrl types a character that is not
ASCII, `map_key_event` delivers `Key::Command(us)`, the US key at that position, and the
character itself is dropped. That is right for a layout that types no ASCII on the key
(Ctrl+М on a Russian layout is Ctrl+V). It also happens where only the level typed is not
ASCII:

- French (AZERTY), Ctrl+Shift and the `*` key, which types `µ` with Shift: the handler gets
  `Key::Command('\\')` with `shift`, and `Tx::key_base()` is `*`. Nothing says `µ`, and a
  handler cannot tell this press from Ctrl+Shift+`\` on a US keyboard.
- Polish, Ctrl+AltGr+A, which types `ą`: the handler gets `Key::Command('a')`. Nothing says
  that AltGr was held or that the key typed `ą`, so the press is Ctrl+A.

**Expected.** A handler can learn the character the key typed with the held modifiers, as
it can for a press without Ctrl (`Key::Text`).

**Ask.** `Tx::key_typed() -> Option<char>` in a key handler: the character the key typed
with the modifiers held, before any substitution; `None` for a key that types none and for
a key with no physical origin. `Key::Command` can then stay as it is for apps that want the
US key.

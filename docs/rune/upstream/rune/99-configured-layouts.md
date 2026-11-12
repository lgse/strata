# 99. A key handler cannot learn what a key types in the other configured layouts

| | |
| --- | --- |
| Kind | New issue |
| Drafts | B3 of the parity plan (`docs/rune/parity/packages/B3.md`, "The press", case 1) |
| Affects | `strata-app`'s press builder (`crates/strata-app/src/commands/press.rs`), which reads every key Strata's shortcuts match |
| Retires | The deviation recorded in `docs/rune/parity/owner.md` under B3: a non-Latin layout with a non-US Latin layout also configured |
| Checked | By source only, at `2a44156` (2026-11-12). `window/keys.rs` gives a handler the active layout's unmodified character (`key_base`, from winit's `key_without_modifiers`) and the physical key, and `map_key_event` replaces a non-ASCII character typed with Ctrl by the US key at that position (`shortcuts::us_char`). `Chord::matches_tx` falls back the same way (`input/chord.rs:210`). Nothing reads another XKB group. |
| Status | Drafted, not filed. It has no headless repro: the configured layouts are the window host's, and `Runtime::key` carries no layout. Workaround in force: the US position, in `press.rs` |

Paste from here.

---

**Title:** A key handler cannot tell what a key types in the user's other configured layouts

Context: Strata (lgse/strata), a file manager on Rune, matches its shortcuts by the letter
printed on the key. Under a layout that types no ASCII (Russian, Greek, Hebrew), its GTK
application reads the key through the user's other configured layouts first, and falls back
to the US key at that position only when none of them types a Latin character there. GDK
answers that with `gdk_display_map_keycode`, which lists what a keycode types in every
configured group and level.

**What happens.** A user has two layouts configured, Russian and German (QWERTZ), and the
Russian one is active. They press Ctrl and the key that is Z on their German layout (Y on a
US keyboard).

- In GTK, Qt and the GTK build of Strata, the shortcut is Ctrl+Z: the other configured
  layout types `z` on that key.
- In a Rune window the handler gets `Key::Command('y')`: off macOS `map_key_event` has put
  the US key in the non-ASCII character's place. `Tx::physical_key()` says `KeyY` and
  `Tx::key_base()` says `н`. Nothing says `z`.

So Undo and Redo trade places for that user, and every shortcut on a key that differs
between US QWERTY and their Latin layout moves with them. `Chord::matches_tx` has the same
fallback, so an app that matches chords gets the same result.

**Expected.** When the active layout types no ASCII on a key, prefer the Latin character
another configured layout types there, at the same Shift level, before the US position: for
the substitution in `map_key_event`, and for `Chord::matches_tx`.

**Ask.** Either of:

1. `Tx::key_in_layouts() -> &[(u32, char)]` (or the like): in a key handler, what the key
   types unmodified and with Shift in each configured layout group, where the platform can
   say (xkbcommon's `xkb_keymap_key_get_syms_by_level` over the keymap's layouts on Linux;
   `TISCopyInputSourceForLanguage` and `UCKeyTranslate` on macOS; `MapVirtualKeyEx` over
   `GetKeyboardLayoutList` on Windows). An app then applies its own rule.
2. Or do it in Rune: make the non-ASCII fallback in `map_key_event` and `Chord::matches_tx`
   look at the other configured layouts before `us_char`.

The first is enough for Strata, and keeps the rule the app's.

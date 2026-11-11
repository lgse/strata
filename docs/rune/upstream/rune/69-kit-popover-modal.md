# 69. rune-kit `Popover` has no Escape and no initial focus

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C20b-1 |
| Affects | C20b (colour button) |
| Retires | C20b's `El::modal` on the popover's body |
| Checked | Re-checked at `2a44156` by source. `Popover::build` still makes a non-modal `role("dialog")` with `focus_return` to its trigger, and `on_close` still adds a scrim for outside presses (`rune-kit/src/overlay.rs:216–233`), unchanged past the pin. rune-kit's README still says "popover/select opening autofocus is still separate" (`crates/rune-kit/README.md:441`). `El::autofocus` (`el.rs:1127`), new since the pin, picks the first focus of an `El::modal` only. Gallery: C20b still puts `El::modal` on the popover's body (`color_picker/view/button.rs:60`); `c20b::` (a11y and scripts, including `popover-escape` and `popover-outside`) and `color_picker::` pass. Earlier: by source at `5d421fd` and `04b38ae`. The repro wasn't run headless. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit `Popover`: Escape does nothing and nothing inside takes focus

Context: Strata (lgse/strata), a file manager on Rune.

`Popover`'s content is a non-modal `dialog` with a scrim for presses outside: Escape does
nothing, nothing in it takes focus, and Tab reaches the page behind it (rune-kit's README:
"popover/select opening autofocus is still separate").

**Repro** (headless): a `Popover` with a focusable child, opened by a button press; Escape
leaves it open, Tab reaches the page behind.

**Ask.** `Popover::modal(true)`: Escape closes, Tab stays in, the first stop is focused, and
focus returns to the trigger on close.

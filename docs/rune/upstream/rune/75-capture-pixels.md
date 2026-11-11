# 75. Capture an element's drawn pixels

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C23-3 |
| Affects | C23 (dissolve), P1 (drag icons rendered in a second `Runtime`) |
| Retires | C23's dissolve from a host-built ghost instead of the real pixels |
| Checked | Re-checked at `2a44156` (2026-11-10) by source: no subtree capture on `Tx`, `Cx` or `Runtime` (grep for `capture`, `snapshot`, `render_to`); `drag_out::snapshot` is still `pub(crate)` (`drag_out/mod.rs:320`); `El::effect_backdrop` (`el/effect.rs:60`) is the only copy of drawn pixels. Duplicates: #148 (read a `Pixels` back) and #68 (closed, window screenshots for QA) aren't this. Gallery: C23's dissolve still draws from the host's ghost (`motion/dissolve.rs:4-7`, deviation 5 in `motion/mod.rs`); `mise run test -- c23::` passes. Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Render a subtree into `Pixels` (or a texture) from the live frame

Context: Strata (lgse/strata), a file manager on Rune.

A dissolve effect (an item breaking into fragments as it's deleted) and a drag icon need an
element's drawn pixels. Only an `effect_backdrop` slot copies the target, the whole of it up to
a point, and only inside a compositor; macOS `drag_out` has a live-frame snapshot, but it is
`pub(crate)`.

**Ask.** `Tx::capture(key) -> Pixels` (or a capture request answered next frame) for a subtree
from the live frame.

Related: (link to 05).

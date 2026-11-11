# 67. Anchored overlays are bounded by the window only

| | |
| --- | --- |
| Kind | New issue (related #362) |
| Drafts | U-C22-3 |
| Affects | C22 (folder peek beside a drawer) |
| Retires | C22's `FolderPeek::bounds` clip computed by the host |
| Checked | Re-checked at `2a44156` by source. `anchor_position(a, t, size, vp)` still flips and clamps against the viewport only (`runtime/anchor.rs:21`); past the pin the file gained only right-to-left mirroring (`cded95f0`, #339). #362 is still open (read-only `gh issue view`, 2026-11-10). Gallery: C22's host-computed `FolderPeek::bounds` clip stands (`folder_peek/mod.rs:151`); `folder_peek::` and `c22::` pass, including `a_trailing_edge_card_stays_inside_a_sideways_scrolled_pane`. Earlier: by source at `5d421fd` and `04b38ae`; C22's probe at the pin, not re-run. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Anchored overlays flip and clamp against the window only; add a boundary element

Context: Strata (lgse/strata), a file manager on Rune.

`anchor_position` (`runtime/anchor.rs:21` at `2a44156`) flips and clamps against the window, so
a card beside a list next to a drawer opens over the drawer.

**Repro** (headless): a 400 pt list at x 100 and a 300 pt drawer at x 500 in an 800 pt window; a
256 pt card anchored `RightStart` to a row lands at x 508, over the drawer.

**Ask.** A boundary element for flip and clamp, as Floating UI's `boundary`.

Related: #362 (report the side a placement flipped to).

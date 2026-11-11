# 39. rune-kit `TabStrip`: labels, look hooks, gaps, keys and edge margin

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C7-1, U-C7-3, U-C7-4, U-C7-5, U-C7-7 |
| Affects | C7 (browser tabs), C11b (sheet tabs) |
| Retires | C7's ids prefixed with the strip key, the underline and tint set over each tab, its one-jump reveal clear of the edge fades (`probe.rs`) |
| Checked | Re-checked at `2a44156` by source: the close button is still `.label("Close tab")` with no tooltip (`rune-kit/src/shell/tab_strip/tab.rs:307`), tabs are still keyed `t-{id}` whatever the strip's key (`tab.rs:124`, and the reveal at `:331`), only the vertical strip has `scroll_margin(fade)` (`shell/tab_strip.rs:600`; the horizontal viewport, `:604`, has none), and `TabDragState::set_incoming` still takes only a point (`tab_strip/drag.rs:193`). `07c369b1` (#336, `set_incoming` and `incoming_slot`) is before the pin, so item 3 already counts it; since the pin, `af7971ae` (`TearOutSpec::lone`), `1ea05bc7` (the drag as a state machine), `22aa8cde` and `cded95f0` add no label, look hook, slot, key scope or sideways margin. `TabItem::decorate` (`tab_strip.rs:233`, at the pin too) wraps a tab's element, which is how C7 draws its underline and tint over each tab. In the gallery, `mise run test tabs::` passed (12 tests) and `mise run test c7` (25 gallery tests and 3 more, including `a_revealed_tab_is_clear_of_the_edge_fades`, `probe.rs`'s one-jump reveal), and C7's docs still list U-C7-1, 3, 4, 5 and 7 (`tabs/mod.rs:247–267`). Earlier: by source at `5d421fd`, and the label and key at `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit `TabStrip`: fixed English close label, no active indicator, gaps only at a point, tab keys that collide, and reveals under the edge fade

Context: Strata (lgse/strata), a file manager on Rune.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
1. The close button is labelled "Close tab" in English (`tab.rs:307`), with no tooltip or
   shortcut hint, and nothing sets them.
2. The look has no hooks past its fill, close glyph and sizes: no active indicator (an
   underline), no title colour, no drop-hover state.
3. `TabDragState::set_incoming` takes only a point, so a strip can't open its gap at a slot
   (after the last tab, from the keyboard), the gap is always a tab wide, and there's no thin
   insertion mark.
4. Each tab is keyed `t-<id>` whatever the strip's key (`tab.rs:124`), so two strips' tabs (or
   any other `t-` key) collide.
5. A horizontal strip's viewport has no scroll margin (the vertical one has
   `scroll_margin(fade)`), so a tab revealed at an end lands half under the edge fade.

**Ask.** `TabStrip::close_label` and `close_help`; an active-indicator option and a title or
drop-hover style; `set_incoming_at(slot)` and an insertion-mark mode; tab keys scoped by the
strip's key; the same scroll margin sideways.

Related: #396 (SidebarToggle's fixed English labels).

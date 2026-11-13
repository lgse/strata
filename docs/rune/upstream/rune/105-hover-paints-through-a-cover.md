# 105. Hover paints through a covering element

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A8 of the parity plan (`docs/rune/parity/packages/A8.md`); finding [lgse/strata#2392](https://github.com/lgse/strata/issues/2392) |
| Affects | Strata's plain popovers (`crates/strata-app/src/popover.rs`): the Appearance popover, the Icons header's "Thumbnail size", and "Customize columns…" opened from the first. Any overlay that is not modal |
| Retires | Nothing yet: the owner left the difference in place until Rune can block hover (2026-11-14) |
| Checked | By reading at `2a44156` (2026-11-14): `hover_flip` in `runtime/input/hover.rs` (lines 286-308), `allowed` in `runtime/input/focus.rs` (line 100) and `scroll_at_axis` in `runtime/input/pointer.rs` (lines 415-422); and by a hosted probe in `strata-app` (the List view under the open Appearance popover) |
| Status | Drafted, not filed: to file at the owner's filing pass. It is not [104](104-outside-wheel-closes-a-popover.md), which asks for a modal that lets an outside wheel through; either would let Strata's popover stop the hover beneath it |

Paste from here.

---

**Title:** Let a covering element stop hover paint beneath it

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. There, a popover is
a surface of its own: while the pointer is on it, nothing under it shows hover.

Rune paints hover on every element whose rect holds the pointer: `hover_flip` turns a track on
when `node_contains` and `allowed` hold, and no element above it in the tree can stop that.
`hover_alone` shadows only the element's own ancestors. The two ways to make what lies beneath
not `allowed`, `.inert()` and `El::modal`, also take its scroll viewports out of `scroll_at_axis`,
so a wheel outside the cover can no longer scroll the view under it (104).

**Repro** (headless, `2a44156`): a vertical list of rows, each with `.hover(color)`; over it, as
a later sibling, a full-window element with `on_press` and `on_wheel` (a scrim), and on that a
small surface with buttons. Move the pointer over the scrim, and over the surface where a row lies
beneath: the row's `ElementInfo::hover` is 1.0 in both places, beside the surface's own hover.

**Ask.** A way for an element to stop hover below it: hover going only to the topmost element
under the pointer that takes input (and its ancestors), or an opt-in such as
`El::blocks_hover()` for a scrim or a surface, as an AppKit view that covers others takes their
tracking areas' mouse-moved events. Or, as 104 asks, a modal background that still routes a wheel
to its viewport, which an app could then use in place of the scrim.

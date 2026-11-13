# 104. A modal popover can't also close on a wheel outside it and let that wheel through

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A5 of the parity plan (`docs/rune/parity/packages/A5.md`); finding [lgse/strata#2341](https://github.com/lgse/strata/issues/2341) |
| Affects | A popover made with `El::modal` over a scrolling view: Strata's popovers that GTK closes on an outside scroll (`dismiss_on_outside_scroll` in Strata's `src/ui/scrolling/popover.rs`) |
| Retires | Nothing: Strata's plain popover is not modal (A5's next pull request) |
| Checked | By reading at `2a44156` (2026-11-14): `El::modal` (an inert background), `scroll_at_axis` in `runtime/input/pointer.rs` (only nodes that are not inert), and a reviewer's probe of the workaround |
| Status | Drafted, not filed, by the coordinator's ruling. Worked around: a popover that is not modal, over a full-window scrim whose `on_wheel` closes it and returns false |

Paste from here.

---

**Title:** Let a modal close on an outside wheel and pass the wheel on

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. There, a popover
opened over a scrolling grid closes when the user scrolls the grid, and the grid scrolls by
that same wheel.

A popover that is not modal can do this on Rune: a full-window scrim under it whose `on_wheel`
closes it and returns false, and Rune's fallback scrolls the viewport under the point, which
it finds whatever covers the point. A modal (`El::modal`) cannot: it makes everything outside
it inert, so the wheel finds no handler and no viewport to scroll, and the modal stays.

**Repro** (headless, `2a44156`): a scroll view with tall content, and over it an anchored
surface made with `.modal(true, close)`. Send a wheel at a point of the scroll view outside the
surface: the scroll offset stays at 0 and the surface stays.

**Ask.** A modal option that dismisses on an outside wheel and lets the wheel route as if the
modal were gone, as AppKit's transient popovers do; or document the scrim pattern as the way.

# 104. A popover can't close on a wheel outside it and let that wheel scroll what is under it

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A5 of the parity plan (`docs/rune/parity/packages/A5.md`); finding [lgse/strata#2341](https://github.com/lgse/strata/issues/2341) |
| Affects | Strata's popovers that GTK closes on an outside scroll: the Icons header's "Thumbnail size", the sort popover and the pane header's others (`dismiss_on_outside_scroll` in Strata's `src/ui/scrolling/popover.rs`) |
| Retires | Nothing yet: the Rune popover stays open under an outside wheel, which goes nowhere |
| Checked | By reading at `2a44156` (2026-11-14): `El::modal` (an inert background), rune-kit's `overlay.rs` scrim, and the wheel's routing in `runtime/input/gesture.rs` (`wheel_event`, `offer_wheel`, `run_wheel`) |
| Status | Drafted, not filed. No workaround: the difference is recorded in the finding |

Paste from here.

---

**Title:** Let a popover close on an outside wheel and pass the wheel to what is under it

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. There, a popover
opened over a scrolling grid closes when the user scrolls the grid, and the grid scrolls by
that same wheel.

On Rune a popover can be modal or not. A modal (`El::modal`) makes everything outside it inert,
so a wheel outside goes nowhere. A popover that is not modal closes on an outside press
through a full-window scrim, as rune-kit's `Popover::on_close` builds one. The scrim covers
the grid, and a wheel reaches only the `on_wheel` handlers at the point, up to the nearest
scroll view, or that scroll view. A handler on the scrim can pass the wheel only to the
scrim's own ancestors, never to the element under the scrim.

**Repro** (headless, `2a44156`): a scroll view with tall content, and over it an anchored
surface with a full-window scrim that sets `open = false` on press. Send a wheel at a point of
the scroll view outside the surface. The scroll offset stays at 0 and `open` stays true.

**Ask.** Either of:
- an outside hook on an overlay, such as `El::on_outside_wheel(f)`, that runs `f` and then lets
  the wheel route as if the overlay weren't there;
- a modal option that dismisses on an outside wheel and forwards it, as AppKit's transient
  popovers do.

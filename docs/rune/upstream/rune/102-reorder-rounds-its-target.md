# 102. rune-kit's `Reorder` rounds where a dragged row lands, and keeps its drag offset private

| | |
| --- | --- |
| Kind | New issue; a comment on U21 (strata-components' C8, `crates/strata-components/src/sidebar/mod.rs`, "Rune gaps") would also do |
| Drafts | A3 of the parity plan (`docs/rune/parity/packages/A3.md`), its reordering pull request |
| Affects | strata-components' places sidebar (C8): the built-in places and the pins, dragged to a new order |
| Retires | C8's own row drag, `RowDrag` and `landing` in `crates/strata-components/src/sidebar/drag.rs`, which replaced `SidebarRow::reorder` |
| Checked | At `2a44156` (2026-11-13), by source (`crates/rune-kit/src/shell/reorder.rs`, `Reorder::follow`) and by the parity runs `builtin-places-reorder-survive-restart/columns-gap-after` and `pin-insertion-keeps-one-destination`, which failed on `SidebarRow::reorder` and pass on C8's drag |
| Status | Drafted, not filed. A headless repro is a few lines (below). Workaround in force: C8's drag |

Paste from here.

---

**Title:** `Reorder` rounds the dragged row's target, so a drop past half a row lands a row further on

Context: Strata (lgse/strata), a file manager on Rune, reorders its sidebar's places and
pinned folders by dragging them, as its GTK sidebar does.

**What happens.** `Reorder::follow` puts the dragged row at `from + round(dy / pitch)`
(`shell/reorder.rs`). A drag that ends with the pointer one and a half rows on, in the gap
below the next row but one, lands it two rows on. A file manager's sidebar, and most lists,
drop a row before or after the row under the pointer by which half of it the pointer is in,
and next to the rows on either side of a gap: that is `from + trunc(dy / pitch)`, measured
from the row's middle, so the same drag lands one row on.

**Repro.** Three `SidebarRow`s with `.reorder(..)`, 24 points tall with a 2 point gap. Press
row 0 at its middle and drag 39 points down, into the gap below row 1, then release: the drop
reports `(0, 2)`. The row under the pointer's half is row 1, and a list that drops by it
reports `(0, 1)`.

**Why an app cannot correct it.** The drop callback gets only `(from, to)`, and the drag's
offset (`Lift::Dragging { dy, .. }`) and `Reorder::follow` are private. So an app that wants
another rule has to drop `SidebarRow::reorder` and write the whole drag again, the lift and the
rows stepping aside included. The callback also runs without a `Tx` (U21's note).

**What would help.** A landing rule the app can choose (rounding, or the half of the row under
the pointer), or the drag's offset, public on `Reorder` while a drag is held, and a `Tx` in the
drop callback.

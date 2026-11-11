# 62. `on_visible_rows` a frame late after a rekeying commit

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C1f-5 |
| Affects | C1f, C3 (grid reflow), C25 (thumbnail requests) |
| Retires | A frame of thumbnail requests for the old range after every reflow |
| Checked | Re-run at `2a44156` as probe `rv_62`; the same program at `5d421fd` prints the same. It builds 4,000 cells in 30 pt lines keyed by their first cell, 5 a line, scrolled to line 76, then narrows them to 2 a line in one commit. On frame 1 the cells drawn are 378..406 while the last report is still 380..465; frame 2 reports 380..414. Reports come from `report_visible_rows` (`runtime/virtual/window.rs:15`). Gallery: C25's queue still protects the previous sync's rows for it (`thumbnails/mod.rs:61`, `thumbnails/queue.rs:193`); `thumbnails::` and `icons_grid::tests::runtime` pass. Earlier: C1f's probe at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** After a commit that changes the rows, `on_visible_rows` hears the new range a frame late

Context: Strata (lgse/strata), a file manager on Rune.

After a commit that rekeys a virtual list's rows (a grid reflowed to a new width), the handler
hears the new range only on the next frame, so for one frame the app's visible range is the
old one.

**Repro** (headless, `2a44156`; the same at `5d421fd`): a grid of 4,000 cells in lines keyed by
their first cell, scrolled to line 76 at 5 a line; narrow it to 2 a line: on the first frame the
cells drawn are 378..406 and the last report still 380..465; it updates on the second
(380..414).

**Ask.** Report the new range from the commit that changed the rows.

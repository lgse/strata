# 106. A move laid out while layout feedback is quiet never reaches `on_layout`

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A4 of the parity plan (`docs/rune/parity/packages/A4.md`); finding [lgse/strata#2477](https://github.com/lgse/strata/issues/2477) |
| Affects | Any element whose `on_layout_tx` keeps its rect as bookkeeping: strata-components' collections (C1e) take their viewport from it, and drop a press outside it |
| Retires | Nothing to retire: `strata-app` removed the one trigger it knows of (a new tab drawn for a frame without its sidebar). Any other move in a quiet frame can still lose its report |
| Checked | By reading at `2a44156` (2026-11-14): `report_rects` and `layout_feedback_open` in `runtime/layout_read.rs` (lines 431-433 and 540-568), `ECHOES` (line 41) and the module docs (lines 18-23). By probes in Strata's QA runs: a Columns column laid out at x=0 then x=201, whose `on_layout` heard only x=0, under CPU load |
| Status | Drafted, not filed: to file at the owner's filing pass |

Paste from here.

---

**Title:** `on_layout` never hears a rect that moved while layout feedback was quiet

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. Its list views keep
their viewport's window rect from `on_layout_tx`, and test a press against it.

`layout_read.rs` stops layout feedback after `ECHOES` commits in a row that only layout reads or
`on_layout` caused. The module docs say reads then "get the current rects at the next commit".
`on_layout` does not. While `layout_feedback_open()` is false, `report_rects(quiet = true, …)`
records each element's new rect in `layout_fired` without queuing its handler. A later frame
compares against that recorded rect, finds no move, and queues nothing. A rect that moved in a
quiet frame and then stays where it is is never reported.

**Repro sketch** (headless, `2a44156`):
1. A row with two children:
   - A sidebar whose width is a state field `w`.
   - A box with `on_layout_tx(|s, r, tx| { s.seen = r; tx.live_only(); })`.
2. Make layout feedback go quiet. For example, give a third element an `on_layout` that writes
   a value the view reads into its own size, so `ECHOES` feedback-only commits happen in a row.
3. While it is quiet, change `w` from 0 to 200 in a `Runtime::update` that the counter does not
   reset (or in the same frame as the last echo), so the box moves from x=0 to x=200.
4. Let the runtime settle.

The box is drawn at x=200, its `ElementInfo` says so, and `s.seen` still holds x=0.

**Ask.** Either:
- keep the last *reported* rect while quiet (do not overwrite `layout_fired`), so the move is
  reported once feedback opens again, the way reads get the current rect at the next commit; or
- queue the latest rect of every element that moved while quiet when anything else commits.

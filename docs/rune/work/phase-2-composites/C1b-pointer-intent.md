# C1b Pointer intent and click modes

| | |
| --- | --- |
| Phase / lane | 2 · Collection |
| Plan | §4 C1 (Pointer intent, Click modes) |
| Depends on | C1a. It plugs into C1e's seam, which can land before or after. |
| Unblocks | C2, C3, C4 |
| Owns | `crates/strata-components/src/collection/pointer/` |
| Nodes | `browser/selection/pointer-intent`, `browser/selection/click-modes` |
| Branch | `feat/<issue>-c1b-pointer-intent` |

## Goal

A state machine that turns presses, moves and releases on a row into one of: select,
activate, start a drag, start a marquee, or rename.

## Scope

- Content and gutter hit regions per row, as `src/ui/pointer.rs` defines them in Strata. The
  regions come from the view through a small trait.
- The threshold decision between click, drag and marquee. Rune's `DRAG_THRESHOLD` is 8 pt on
  Linux. A press that crossed the threshold never activates. A modified press (Ctrl, Shift)
  claims only on release.
- Click modes: single or double click per view and per entry kind. The double-click interval
  comes in as data (from P5 in the app, with a default).
- Slow second click to rename: a second click on an already selected item, after the
  double-click interval and before a rename timeout.
- Outputs: C1a operations, plus intents (activate, begin drag with a W1 `DragPayload`, begin
  marquee, begin rename).

## Tests

- Unit tests on a virtual clock, one per Behavior bullet in the two nodes, each naming its
  bullet.
- When C1e has landed: scripts for each click mode, drag beyond the threshold, and the slow
  second click.

## Done when

All pointer-intent and click-mode bullets are covered by unit tests or scripts.

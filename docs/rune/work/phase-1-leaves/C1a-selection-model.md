# C1a Selection model

| | |
| --- | --- |
| Phase / lane | 1 · Collection (critical path) |
| Plan | §4 C1: Selection, Position restore |
| Depends on | W1 |
| Unblocks | C1b, C1c, C1d, C1e, C18 |
| Owns | `crates/strata-components/src/collection/selection/` |
| Nodes | `browser/selection`, `browser/view-modes/position-restore`, `integration/10xer-mode/file-verbs` |
| Branch | `feat/<issue>-c1a-selection-model` |

## Goal

The selection state for every view, as plain Rust with no Rune elements, so that its
behaviour is tested by unit tests and the controllers can be built against it at once.

## Scope

- `Selection`, keyed by `EntryId`: the selected set, the anchor and the cursor, with the
  cursor kept apart from the selection.
- Operations: set, toggle (Ctrl), extend a range from the anchor (Shift), Ctrl+Shift extend,
  select all, clear (Escape), and an empty-space click.
- Ranges resolve through `EntrySource` index order. They are stored so that a re-sort or a
  live update keeps the same ids selected. Removed ids drop out, and the cursor moves to a
  neighbour by the rule in Strata's sweeps.
- 10xer mode: a "filled" selection versus the cursor item. Verbs act on the filled set if
  there is one, else on the cursor.
- `RestoreMemory`: cursor, selection and scroll anchor per folder key, with a size bound.
  Restoring it after a refresh comes back in C1e.
- Change notifications (what changed), so C1e announces only real changes to a11y.

## Out of scope

Pointer and keyboard event handling (C1b, C1c), drawing (C1e) and the marquee (C1d).

## Tests

- One unit test per `browser/selection` Behavior bullet that this layer decides. Each test
  names its bullet id in a comment.
- A property test: random operations, re-sorts and removals never leave the cursor or anchor
  on a missing id.
- 100k entries: select all, then a Shift range across the list, each under 1 ms in
  `--release`. Mark it as a perf test.

## Done when

`featuremap.py qa --node browser/selection` bullets that belong to the model are each covered
or listed in the PR as belonging to C1b–C1e.

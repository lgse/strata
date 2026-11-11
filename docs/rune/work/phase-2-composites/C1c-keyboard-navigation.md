# C1c Keyboard navigation

| | |
| --- | --- |
| Phase / lane | 2 · Collection |
| Plan | §4 C1 (Keyboard) |
| Depends on | C1a, W1 geometry. It plugs into C1e's seam. |
| Unblocks | C2, C3, C4 |
| Owns | `crates/strata-components/src/collection/keyboard/` |
| Nodes | `browser/selection/keyboard-navigation` |
| Branch | `feat/<issue>-c1c-keyboard-navigation` |

## Goal

The key map for moving the cursor and selection in any layout, through
`CollectionGeometry::neighbor`, so the list, the grid and each column behave the same.

## Scope

- Arrows and `h`, `j`, `k` and `l` move between regions. The arrow keys never change
  directory: crossing into another column is C2's call, not this layer's.
- Page Up and Page Down by `page_step`, Home and End, and Ctrl+Up and Ctrl+Down.
- Shift extends from the anchor. Ctrl moves the cursor without selecting, and Ctrl+Space
  toggles.
- Type-ahead: find by a typed prefix, with a timeout.
- Keys are matched with Rune's layout-aware chords (`physical_key` and `key_base`), so `hjkl`
  follow Strata's layout rules.
- Output: C1a operations, plus a "scroll cursor into view" request that C1e applies.

## Tests

Unit tests per Behavior bullet over the uniform geometry and a fake grid geometry. Scripts
when C1e has landed.

## Done when

All keyboard-navigation bullets are covered.

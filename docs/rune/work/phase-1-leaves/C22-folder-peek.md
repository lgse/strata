# C22 Folder peek

| | |
| --- | --- |
| Phase / lane | 1 · Chrome (small) |
| Plan | §4 C22, `preview/quick-preview/folder-peek` |
| Depends on | W1, W2 |
| Unblocks | M3 |
| Owns | `crates/strata-components/src/folder_peek/`, `scenes/c22.rs` |
| Branch | `feat/<issue>-c22-folder-peek` |

## Goal

Adopt rune-kit `HoverCard` with `El::hover_delays` to show a folder's first rows.

## Scope

Up to 8 rows (name and icon) from data, a "and N more" line, and the empty and loading
states. Off by default, behind a flag the host passes in. The open and close delays follow
Strata.

## Tests

A script that waits out the hover delay on the virtual clock, then checks the card's rows in
the tree. Leaving closes it. A11y golden.

## Done when

The folder-peek bullets pass headless.

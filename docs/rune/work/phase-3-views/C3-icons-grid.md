# C3 IconsGrid

| | |
| --- | --- |
| Phase / lane | 3 · Collection |
| Plan | §4 C3, `browser/view-modes/icons`, `.../thumbnail-size`, `.../folder-views` |
| Depends on | C1e, C1b, C1c, C1d, C25 |
| Unblocks | C20d, X2, M2 |
| Owns | `crates/strata-components/src/icons_grid/`, `scenes/c3.rs`, `strata-kit/src/slots/icons.rs` |
| Branch | `feat/<issue>-c3-icons-grid` |

## Goal

The thumbnail grid. Rune virtualizes vertical rows only, so the grid packs N tiles per
virtual row.

## Scope

- A grid `CollectionGeometry`: N comes from the viewport width and the tile size. It
  implements `rect_of`, `index_at`, `range_in` and `neighbor` (up and down move by N), so C1c
  and C1d work unchanged.
- Reflow when the width or tile size changes, keeping the cursor tile in view
  (`keep_on_screen`).
- Tile sizes step along W2's ladder with Ctrl+Shift+wheel.
- Thumbnails from C25, with the placeholder icon until one is ready.
- A two-line caption with its own hit region, for C1b's content regions.
- Note the Rune issue for grid virtualization (S5), and keep this packing until it ships.

## Tests

- Unit: the geometry at row edges, with a last partial row.
- Scripts: resizing reflows and keeps the cursor, the size ladder, keyboard moves by rows, a
  marquee across rows, and caption hits.
- A11y golden: a grid with row and column positions.
- Perf: 100k entries at 60 Hz.

## Done when

The icons and thumbnail-size bullets pass headless, and the perf bar is met.

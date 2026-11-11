# C1d Marquee

| | |
| --- | --- |
| Phase / lane | 2 · Collection |
| Plan | §4 C1 (Marquee) |
| Depends on | C1a, W1 geometry. It plugs into C1e's seam. C1b emits "begin marquee". |
| Unblocks | C2, C3, C4 |
| Owns | `crates/strata-components/src/collection/marquee/` |
| Nodes | `browser/selection/marquee` |
| Branch | `feat/<issue>-c1d-marquee` |

## Goal

Band selection over virtual rows, including rows that are not mounted.

## Scope

- The band is drawn over the viewport in content coordinates, so it follows a scroll.
- Hit-testing goes through `CollectionGeometry::range_in` by index, never through mounted
  elements, so a band dragged across 10,000 rows selects them all.
- Plain replaces the selection, Ctrl toggles, and Shift adds. The selection from before the
  band is kept until release, so moving the band back restores it.
- Auto-scroll near the edges, on a Rune glide (`scroll_to_with`), not stepped scrolls. The
  speed follows the distance past the edge.

## Tests

Unit tests: a range over unmounted rows, the modifiers, and moving back restores. Scripts when
C1e has landed: a drag selects, edge auto-scroll with `RUNE_PERF_WARN` silent, and Escape
cancels.

## Done when

All marquee bullets are covered.

# C24 Scroll extras

| | |
| --- | --- |
| Phase / lane | 1 · Collection |
| Plan | §4 C24, `browser/scrolling` |
| Depends on | W2 |
| Unblocks | C1e, C2 (both use it) |
| Owns | `crates/strata-components/src/scroll/`, `scenes/c24.rs` |
| Branch | `feat/<issue>-c24-scroll-extras` |

## Goal

The scroll behaviours Strata adds on top of a Rune scroll viewport, usable by any view.

## Scope

- Middle-click autoscroll: `pointer_buttons([Middle])` starts it. An origin marker is drawn.
  Speed follows the pointer's distance from the origin, and the scroll runs on a
  `scroll_to_with` glide, not stepped instant scrolls (Rune warns about those). A click or
  Escape stops it.
- Overlay scrollbars from rune-kit `ScrollBar`, with Strata's look tokens.
- Check that wheel routing follows Rune's latching rules in nested scrollers (a column inside
  the Columns strip). Fix ours if not. If Rune is wrong, file an issue.

## Tests

- Scripts: middle-press then move scrolls, and Escape stops. The perf warnings about stepped
  scrolls stay silent (`RUNE_PERF_WARN`).
- Nested scrollers: a vertical wheel over an inner list scrolls the list, and a horizontal
  wheel reaches the strip.

## Done when

The scripts pass, and the scene shows a 10k-row list with the overlay scrollbar.

# P6 Freedesktop icon theme lookup

| | |
| --- | --- |
| Phase / lane | 1 · Desktop (moved earlier from plan phase 6: it has no dependencies) |
| Plan | §4 P6, `integration/open-with` |
| Depends on | W0 |
| Unblocks | Strata's Open With chooser (C15-based) |
| Owns | `crates/strata-platform/src/icon_theme/` |
| Feature | `icon-theme` |
| Branch | `feat/<issue>-p6-icon-theme` |

## Goal

Find an application or MIME icon by name, at a size and scale, the way GTK's `IconTheme`
did, and hand it over as `rune::Pixels`.

## Scope

- The Icon Theme Specification: `index.theme` parsing, `Inherits` chains, the `hicolor`
  fallback, directory size matching (Fixed, Scalable, Threshold), scale directories,
  `/usr/share/pixmaps`, and absolute paths from `.desktop` `Icon=`.
- The current theme name comes from a parameter. The app gets it from P5 or its settings.
- Rasterize SVG with `resvg`, and decode PNG. Run lookups and rasterizing off the UI thread,
  with an LRU cache keyed by name, size and scale.
- Return `rune::Pixels`, or "not found".

## Tests

Unit tests over a fixture icon tree in `tests/fixtures/icons/`: inheritance, the closest
size, the scalable fallback, the scale-2 directories and a missing icon. No display needed.

## Done when

The fixture tests pass, and an example shows Firefox's and Nautilus's icons from the system
theme.

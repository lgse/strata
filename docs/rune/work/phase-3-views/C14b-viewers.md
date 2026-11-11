# C14b Viewers and archive preview

| | |
| --- | --- |
| Phase / lane | 3 · Preview |
| Plan | §4 C14, `preview/preview-panel/model-views`, `.../routes`, `.../gcode` |
| Depends on | C14a, C1e (archive preview), C9 |
| Unblocks | M5 |
| Owns | `crates/strata-components/src/viewers/` (except `viewport/`), `scenes/c14b.rs` |
| Branch | `feat/<issue>-c14b-viewers` |

## Goal

The thin preview parts on top of C14a. Each one is small, and they share a single WP.

## Scope

- **Image view:** C14a with the image name and its dimensions.
- **3D model view:** a software-rendered frame (from Strata, as `Pixels`), four camera presets,
  an orbit indicator, and badges. Orbit by drag sends a camera intent.
- **Route view:** an image, a marker animated along the route, and the elevation profile
  drawn with `canvas` shapes.
- **G-code thumbnails:** an image with its badges.
- **Archive preview:** C1e in read-only mode over the member tree. No rename, no drop, no
  drag-out.

The image view and the model, route and G-code views can be built before C1e lands. Only the
archive preview waits for it.

## Tests

Scripts per viewer: camera presets, the route marker on the virtual clock, and archive
navigation by keyboard. A11y golden per viewer.

## Done when

The model, route and G-code bullets, and the archive preview bullets, pass headless.

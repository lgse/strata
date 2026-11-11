# K2 Shape, density and type

| | |
| --- | --- |
| Phase / lane | 1 · Look |
| Plan | §4 K2, `settings/themes/corner-roundness`, `browser/view-modes/density`, `.../text-size`, `settings/themes/interface-font` |
| Depends on | W2 |
| Unblocks | X1, K4. C20a's sliders drive these values in Strata. |
| Owns | `crates/strata-kit/src/shape.rs`, `density.rs`, `typography.rs` |
| Branch | `feat/<issue>-k2-shape-density-type` |

## Goal

Fill W2's extension tokens with Strata's real values, as functions of the user's settings.

## Scope

- Corner roundness from 0 to 200 % scales `Kit.radius` and every radius token.
- Frame and Content density, with the same steps as Strata, become the spacing and row-height
  tokens.
- Text size scales the type ramp. The desktop text scale (P5) multiplies it, and arrives as a
  number.
- The interface font switches through `Tx::set_font_families`. List installed fonts for the
  picker with Rune's `system_fonts` if it can, or with P5's addition (S5 decides).
- A `LookParams { roundness, frame_density, content_density, text_size, text_scale, font }`
  that, with a theme, builds the `Look`.

## Tests

- Golden scenes at the density and text-size extremes, light and dark. Inspect them for
  clipping. Don't assert pixel sizes.
- Roundness at 0 % and 200 % (PNG).

## Done when

The gallery has a "look" scene with controls for every parameter, and the C1e owner signs off
on the row-height tokens.

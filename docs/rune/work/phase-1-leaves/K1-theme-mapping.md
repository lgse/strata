# K1 Theme mapping

| | |
| --- | --- |
| Phase / lane | 1 · Look |
| Plan | §4 K1, `settings/themes`, `.../omarchy` |
| Depends on | W2 |
| Unblocks | X1. C10a's real syntax colours, and K4. |
| Owns | `crates/strata-kit/src/theme/`, `crates/strata-kit/themes/` (theme data) |
| Branch | `feat/<issue>-k1-theme-mapping` |

## Goal

Turn Strata's themes into a `Look`: the 95 bundled themes, custom theme files, and the Omarchy
Quattro mapping.

## Scope

- Read Strata's theme format, as it is at the plan's watermark commit, into a typed theme.
- Build the base with `Kit::from_palette`, then override the semantic tokens per theme.
- Syntax palettes:
  - map onto rune-kit's `SyntaxTheme`;
  - build one syntect `Theme` per palette, from scope selectors. C10a takes it as input,
    which is why K1 needs no C10a code. Only the type comes from syntect.
- Glow maps onto `rune-fx` glow parameters in the W2 extension tokens.
- Omarchy: read the current Omarchy theme and map it as Strata does.
- Theme switching: a helper that moves from one `Look` to the next on `Tx::color_motion`.
- A gallery scene that shows any bundled theme by name, for review. It stays out of the
  goldens.

## Tests

- Every bundled theme parses and maps, with no missing token.
- A custom theme with a bad value reports its line.
- PNG goldens for three representative themes, light and dark.

## Done when

All 95 themes map. A theme switch in the gallery animates. The syntect themes colour the
S4 fixtures.

# Bundled font

`JetBrainsMono[wght].ttf` is JetBrains Mono, the face Strata draws its interface and its
previews in. strata-kit compiles it in (`src/fonts.rs`) and exports it as `fonts::FONTS`, for
`AppSpec::fonts`, with the families `fonts::FAMILIES`, for `AppSpec::families`.

## Source

| | |
| --- | --- |
| File | `data/fonts/JetBrainsMono[wght].ttf` in [lgse/strata](https://github.com/lgse/strata) |
| Revision | `35ee677ab0a633d5d49055464f2946fcf4995ae7` |
| Upstream | [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) 2.304, the variable face with a `wght` axis from 100 to 800 |
| Family name | `JetBrains Mono` |
| Changes | None. Byte for byte, under the upstream file name. |
| SHA-256 | `662a196d58f1183bf2d77428b6d5283fe3f45161ab021bea4036bc98e5cac016` (303,144 bytes) |

The licence file is Strata's `data/licenses/JetBrainsMono-OFL-1.1.txt` at the same revision,
byte for byte (SHA-256 `30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad`).

## Licence and credits

- **JetBrains Mono**: SIL Open Font License 1.1, Copyright 2020 The JetBrains Mono Project
  Authors (JetBrains s.r.o.). The font is distributed unmodified. The licence must ship with
  any copy of the font, including a binary that embeds strata-kit. Full text:
  [`LICENSE-JetBrains-Mono-OFL-1.1.txt`](LICENSE-JetBrains-Mono-OFL-1.1.txt).

The crate's own licence stays MIT; the font keeps its own.

## Refreshing

Copy the font and its licence from a newer Strata revision, byte for byte, and update the
revision, version and both hashes above. Then run `mise run test fonts::` and
`mise run test k6::`, and re-bless the PNG goldens (`mise run bless`): every page draws text.

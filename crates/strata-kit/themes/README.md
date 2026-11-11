# Theme data

`catalog.toml` is Strata's bundled theme catalogue: 95 themes, each with nine interface
colours and five syntax colours. strata-kit compiles it in (`src/theme/catalog.rs`) and maps
each theme onto a `Look` (`src/theme/map.rs`).

## Source

| | |
| --- | --- |
| File | `data/themes/catalog.toml` in [lgse/strata](https://github.com/lgse/strata) |
| Revision | `35ee677ab0a633d5d49055464f2946fcf4995ae7` |
| Changes | A four-line header naming the source. The rest is byte for byte. |

## Licences and credits

Both are MIT, and both notices must ship with any copy of this data, including a binary
that embeds it:

- **Strata**: Copyright (c) 2026 LGSE Ltd. Its catalogue, its original themes (Azure Glow,
  Omarchy Light) and its readability adjustments. Full text: [`LICENSE-Strata.txt`](LICENSE-Strata.txt).
- **Tinted Theming schemes** ([tinted-theming/schemes](https://github.com/tinted-theming/schemes)
  at `fdca32a0d14ec80ad83a78a9ccb85592ca6cb9e1`): Copyright (c) 2022 Tinted Theming and the
  scheme authors named in the upstream files. The Base16 palettes derive from them. Full
  text: [`LICENSE-Tinted-Theming.txt`](LICENSE-Tinted-Theming.txt).

## Refreshing

Copy the file from a newer Strata revision and keep the header, with the new revision in it
and in the table above. Then run `mise run test theme::` and `mise run test k1::`. The
catalogue tests check that every theme parses and meets Strata's contrast floors, and the
k1 goldens show three of them.

The repository's `typos.toml` accepts the one theme name the spell checker would correct.

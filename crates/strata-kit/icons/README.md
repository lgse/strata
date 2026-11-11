# Icon sources

strata-kit's `build.rs` turns these SVGs into the icon catalogs in `src/icons/`
(`lucide::SET`, `catppuccin::MOCHA`, `catppuccin::LATTE`). Each file's stem is its icon's
`Icon::name`, so Strata's own icon names (`strata-folder`, `strata-ctp-rust`) find the same
icons here.

| Folder | What | From |
| --- | --- | --- |
| `lucide/` | 110 Lucide interface and generic file-type icons, Strata's `strata-*.svg` | `data/icons/scalable/actions/` in [lgse/strata](https://github.com/lgse/strata) at `35ee677ab0a633d5d49055464f2946fcf4995ae7`, byte for byte. Strata took them from [Lucide](https://github.com/lucide-icons/lucide) 1.35.0. |
| `catppuccin/` | 354 Catppuccin file-type icons for code, language and config files, `strata-ctp-*.svg` | `data/icons/catppuccin/` in lgse/strata at the same revision, byte for byte. Strata copied them from [catppuccin/vscode-icons](https://github.com/catppuccin/vscode-icons) `b6915da9f6889b683a110aa747de96c2820a537d` (v1.26.0), `icons/css-variables/`. |
| `lucide-extra/` | 11 Lucide icons Strata doesn't ship, renamed `strata-kit-<lucide name>.svg` | `https://raw.githubusercontent.com/lucide-icons/lucide/1.35.0/icons/<name>.svg` (tag `1.35.0`), byte for byte apart from the name. SHA-256 below. |
| `own/` | `strata-kit-chevron-left` and `-up`: Strata's Lucide chevron-right and -down turned half a turn | This repository |

### `lucide-extra/` files

Fetched read-only from the tag above, checked to be plain SVGs (`path`, `line`, `circle`,
`rect` only; no scripts, styles or references), then renamed:

| File | Lucide name | SHA-256 |
| --- | --- | --- |
| `strata-kit-circle-question-mark.svg` | `circle-question-mark` (formerly `circle-help`) | `975db087da041530dbf59ba1e61a7b99c3519fad43dae6375e42ccc74ba63924` |
| `strata-kit-clipboard.svg` | `clipboard` | `d9b433e6a571d96a035ddb83d2ff52c4f85925ffcba9e584cf371d0dfa90875d` |
| `strata-kit-ellipsis.svg` | `ellipsis` | `4f495cc72013ffdfec677f03b33a150f7b4dd741979283fd6853a09024bca112` |
| `strata-kit-grip-vertical.svg` | `grip-vertical` | `ddf3a20c45869614f7b49dd14c5cbaba026d4155382e30b0ae8418f31c1ce2c5` |
| `strata-kit-link.svg` | `link` | `4f3237183dcaabfac92f4e94c776a9a9e45c3c5e77a921061a6b34b61eb5413f` |
| `strata-kit-menu.svg` | `menu` | `af6491988ac1ba30dc5414f485bc48713e957e1edb0fbef4a0dd8691c3c21598` |
| `strata-kit-repeat.svg` | `repeat` | `6b67b8707e6a3107a1ccad48716bd93fc5e95e8d91f2bff5eec64f62cc7dfc4b` |
| `strata-kit-rotate-cw.svg` | `rotate-cw` | `ddcfe6d87240475946935e77411cd4d15a06f3d28a9b921bafed224ebe953668` |
| `strata-kit-scan.svg` | `scan` | `c77f0e634b2451a14a262a59b61d75bbcea3994b16eeba1f5753e86b33282a4d` |
| `strata-kit-zoom-in.svg` | `zoom-in` | `91206ae20013e2c84f2f5d09db5a7200f11698a84fc4e857b06fab2ce7c933f1` |
| `strata-kit-zoom-out.svg` | `zoom-out` | `28e7849b55bb9b34cb24be83b4e8ef58f2ae164fc451fbd60e026bfb507d2921` |

The Mocha and Latte palettes in `src/icons/lucide/generate.rs` are Strata's (`src/assets.rs` at the same revision),
from the Catppuccin palette.

## What the build changes

The SVG files stay as they are; the generator (`src/icons/lucide/generate.rs`, which
`build.rs` runs) changes the bodies it makes, following Strata's own renderer
(`src/assets.rs`):

- Paint attributes on the root `<svg>` move to a wrapping `<g>`, as Rune's icon bodies have no
  `<svg>`.
- Strata's tint placeholders in `lucide/` (`#8bc9eb`, `#22d3ee`, `#2e3436`) become
  `currentColor`, so components tint them. So does the white of `strata-check-on-primary`
  only: it sits on an accent fill, and its consumer tints it with an on-accent colour. `strata-folder`
  gets Strata's 15 % fill in its own colour.
- Each `var(--vscode-ctp-*)` colour in `catppuccin/` resolves three ways: the Mocha colour, the
  Latte colour, or `currentColor` with a 15 % fill on unfilled shapes (Strata's Theme colors).

## What Rune doesn't draw under Theme colors

Theme colors bodies draw through Rune's single-colour rasterizer, which ignores three
attributes. Brand colors draw these icons through resvg, which keeps them. Rune draft U-K3-3
asks for them.

| Attribute | Sources | Under Theme colors |
| --- | --- | --- |
| `stroke-dasharray` | `strata-ctp-go-template` | a solid line where the source dashes |
| `stroke-miterlimit` | `strata-ctp-dub-selections`, `strata-ctp-yarn` | Rune's default miter limit |
| `clip-path` | `strata-ctp-humans` | unclipped; its clip is the whole icon, so nothing changes |

`strata-ctp-humans` (a white clip path) and `strata-ctp-poetry-lock` (a gradient's white stop)
also keep a raw colour outside `var()`, so they draw as colour images in every catalog.

## Licences and credits

Each notice below must ship with any copy of these icons, including a binary that embeds
strata-kit. The crate's own licence stays MIT; the data keeps its own licences.

- **Lucide**: ISC, Copyright (c) 2026 Lucide Icons and Contributors. The icons Lucide derived
  from Feather are MIT, Copyright (c) 2013-present Cole Bemis; the file lists them. Full text,
  both notices: [`LICENSE-Lucide-ISC.txt`](LICENSE-Lucide-ISC.txt), from Strata's
  `data/licenses/Lucide-ISC.txt`.
- **Catppuccin VSCode Icons**: MIT, Copyright (c) 2023 Catppuccin and Copyright (c) 2023
  thang-nm. Full text: [`LICENSE-Catppuccin-VSCode-Icons-MIT.txt`](LICENSE-Catppuccin-VSCode-Icons-MIT.txt).
- **Strata**: MIT, Copyright (c) 2026 LGSE Ltd. Its curation, its edits to Lucide (the outlined
  `-symbolic` icons and the filled sort caret) and its rendering rules. Full text:
  [`LICENSE-Strata.txt`](LICENSE-Strata.txt).

## Refreshing

1. Copy `data/icons/scalable/actions/*.svg` and `data/icons/catppuccin/*.svg` from a newer
   Strata revision over `lucide/` and `catppuccin/` (remove files Strata removed), and update
   the revisions above and the licence files from Strata's `data/licenses/`.
2. Build. The build stops on a source outside the rules in `src/icons/lucide/generate.rs`:
   an element, attribute or root attribute it doesn't list, a reference outside the file, a
   named colour, a colour that isn't a tint placeholder in `lucide/`, a raw colour or unknown
   variable in `catppuccin/`, a name in two folders. `src/icons/lucide/semantic.rs` fails to
   compile when an icon it maps is gone. Update the Rune-ignored table above if those
   attributes appear in new places.
3. Run `mise run test icons::` and `mise run test k3`, then bless and review the `k3` goldens.

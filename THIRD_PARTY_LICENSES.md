# Third-party notices

Strata includes or derives assets from the following projects.

## MathJax

- Projects: <https://github.com/mathjax/MathJax-src>, <https://github.com/mathjax/MathJax-fonts>
- Version: MathJax and MathJax TeX SVG font data 4.1.3
- Copyright: 2017–2026 The MathJax Consortium
- License: Apache License 2.0
- Included asset: `data/math-renderer.js` (bundled/minified base and AMS renderer with TeX SVG glyph data)
- Full license: [`data/licenses/MathJax-Apache-2.0.txt`](data/licenses/MathJax-Apache-2.0.txt)

The bundle adds Strata's bounded offline rendering entry point. Build inputs and
regeneration instructions are in [`packaging/math-renderer/`](packaging/math-renderer/README.md).

## JetBrains Mono

- Project: <https://github.com/JetBrains/JetBrainsMono>
- Version: 2.304
- Copyright: JetBrains s.r.o.
- License: SIL Open Font License 1.1
- Included asset: `data/fonts/JetBrainsMono[wght].ttf`
- Full license: [`data/licenses/JetBrainsMono-OFL-1.1.txt`](data/licenses/JetBrainsMono-OFL-1.1.txt)

The font is distributed unmodified. Strata materializes the embedded font in its private cache at runtime so it is available without changing the user's system font installation.

## Lucide

- Project: <https://github.com/lucide-icons/lucide>
- Version: 1.35.0
- Copyright: Lucide Contributors and Feather Icons contributors
- License: ISC
- Included assets: curated and namespaced SVG icons under `data/icons/scalable/actions/`
- Full license: [`data/licenses/Lucide-ISC.txt`](data/licenses/Lucide-ISC.txt)

The SVGs retain Lucide geometry except for the trash fill-level and open-lid variants, which use modified paths. The table sort caret uses the Lucide triangle with a filled foreground, rotated by GTK for descending order. Their foreground color was changed from `currentColor` to GTK's symbolic foreground color so GTK can recolor them according to the active theme.

## Tinted Theming schemes

- Project: <https://github.com/tinted-theming/schemes>
- Revision: `fdca32a0d14ec80ad83a78a9ccb85592ca6cb9e1`
- Copyright: Tinted Theming and the scheme authors identified by the upstream files
- License: MIT
- Derived asset: `data/themes/catalog.toml`
- Full license: [`data/licenses/Tinted-Theming-MIT.txt`](data/licenses/Tinted-Theming-MIT.txt)

The bundled catalog contains curated Base16 palettes derived from this project alongside Strata's original themes. The Base16 mapping to Strata's semantic UI tokens is documented in the catalog.

## GTK and GStreamer media-runtime patch kit

- Projects: <https://gitlab.gnome.org/GNOME/gtk> and <https://gitlab.freedesktop.org/gstreamer/gstreamer>
- Source versions: GTK 4.22.4 and gst-plugins-bad 1.28.6
- License of the patched source files: LGPL-2.0-or-later (GTK's project license is LGPL-2.1-or-later)
- Included material: source-context patches under `packaging/media-runtime/patches/`
- Full licenses: [GTK](packaging/media-runtime/licenses/GTK-COPYING) and [GStreamer](packaging/media-runtime/licenses/GSTREAMER-COPYING)

The experimental patch kit preserves upstream source notices and identifies pinned
source archives. It does not bundle toolkit binaries or alter Strata's release
artifacts. See its [build and redistribution requirements](packaging/media-runtime/README.md).

## UnRAR

- Project: <https://www.rarlab.com/rar_add.htm>
- Bundled source: UnRAR 7.01 (2024-05-12), statically linked by `unrar_sys` 0.5.8
- Copyright: Alexander Roshal
- License: custom UnRAR license; **not** MIT or Apache-2.0
- Full license: [`data/licenses/UnRAR.txt`](data/licenses/UnRAR.txt), also shipped as `UnRAR.txt` in release archives

UnRAR permits RAR extraction but prohibits using its source to develop a
RAR-compatible archiver or recreate the proprietary RAR compression algorithm.
The wrapper crates' permissive metadata does not cover this native implementation.
The pinned crate archive has SHA-256
`8b77675b883cfbe6bf41e6b7a5cd6008e0a83ba497de3d96e41a064bbeead765`.
Strata does not modify its vendored source.

## Rust dependencies

Rust dependency licenses are declared in each package's metadata and are validated in CI with `cargo-deny`. To review the current dependency graph locally, run:

```bash
cargo deny check licenses
```

### arrayref

The bundled icon renderer uses `arrayref` 0.3.9 through `resvg` / `tiny-skia`.
Its BSD-2-Clause license requires the following notice in binary distributions.
This document is included in release archives and installed license materials.

Source: <https://github.com/droundy/arrayref>

```text
Copyright (c) 2015 David Roundy <roundyd@physics.oregonstate.edu>
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the
   distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

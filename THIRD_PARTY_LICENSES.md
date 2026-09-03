# Third-party notices

Strata includes or derives assets from the following projects.

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

The SVGs retain Lucide geometry. Their foreground color was changed from `currentColor` to GTK's symbolic foreground color so GTK can recolor them according to the active theme.

## Tinted Theming schemes

- Project: <https://github.com/tinted-theming/schemes>
- Revision: `fdca32a0d14ec80ad83a78a9ccb85592ca6cb9e1`
- Copyright: Tinted Theming and the scheme authors identified by the upstream files
- License: MIT
- Derived asset: `data/themes/catalog.toml`
- Full license: [`data/licenses/Tinted-Theming-MIT.txt`](data/licenses/Tinted-Theming-MIT.txt)

The bundled catalog contains curated Base16 palettes derived from this project alongside Strata's original themes. The Base16 mapping to Strata's semantic UI tokens is documented in the catalog.

## Rust dependencies

Rust dependency licenses are declared in each package's metadata and are validated in CI with `cargo-deny`. To review the current dependency graph locally, run:

```bash
cargo deny check licenses
```

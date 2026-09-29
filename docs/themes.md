# Themes

Strata styles the interface with nine semantic color tokens. Bundled themes are the fallback on any Linux desktop; Azure Glow is the default. Settings presents all 95 bundled themes in one searchable, light/dark-filterable scrolling catalog.

Tinted Base16 entries map colors to Strata tokens as follows: `base00` to background, `base01` to surface, `base05` to text, `base0D` to accent, `base08` to danger, `base02` to muted and highlight, `base03` to border, and `base04` to dim text. Source revision and licensing details are recorded in [`THIRD_PARTY_LICENSES.md`](../THIRD_PARTY_LICENSES.md).

## Custom theme files

Custom themes are TOML files in:

```text
~/.config/strata/themes/<theme-id>.toml
```

The settings configurator writes the same format, so generated themes can be edited or shared:

```toml
name = "Ocean Blue"
background = "#0c1a2b"
surface = "#122438"
text = "#c9deed"
accent = "#4fd6ff"
danger = "#ff6b7a"
muted = "#1e3a52"
highlight = "#244d68"
border = "#315b75"
dim_text = "#6f8da3"
```

Strata discovers valid `.toml` files in this directory on startup and displays them under **Your themes**. If a custom filename matches a bundled theme ID, the custom theme replaces that bundled entry so saved preferences and selection always use the user’s palette.

## Syntax colors

All 95 bundled themes include explicit code-preview palettes. Tinted Base16 palettes map `base0E` to keywords, `base0B` to strings, `base09` to constants, `base0A` to types, and `base0C` to preprocessor directives. Catppuccin and Tokyo Night use the pinned `catppuccin-mocha` and `tokyo-night-dark` syntax palettes; Azure Glow and Omarchy Light have original curated palettes.

Under **Settings → Theme & appearance → Add a theme**, the syntax color pickers start with the selected theme's colors. Each picker previews changes immediately in open code previews; Cancel restores the selected theme, and Add theme saves all five syntax colors with the interface palette. **Dim text / comments** controls comments as well as dim interface text. Markdown headings continue to use the accent color.

Custom TOML files can override any syntax role independently:

```toml
syntax_keyword = "#ff7b72"
syntax_string = "#7ee787"
syntax_constant = "#79c0ff"
syntax_type = "#ffa657"
syntax_preprocessor = "#d2a8ff"
```

Recognized source languages keep syntax highlighting regardless of the loaded text's byte count, line count, or individual line length. Source text still loads incrementally, and the separate 1 MiB preview-read cap remains in effect. Large plain-text files without a recognized language can still use virtualized rendering.

These fields accept GTK CSS color formats. Omitted fields retain the previous accent/text-derived colors, so existing files remain valid. The editor initializes omitted fields from those derived colors when creating a theme. Invalid colors cause a custom file to be excluded at startup. Preview schemes canonicalize colors to opaque `#rrggbb`.

## Omarchy Quattro

On Omarchy Quattro, Strata detects the active theme from:

```text
~/.local/state/omarchy/current/theme.name
~/.local/state/omarchy/current/theme/colors.toml
```

The application maps Quattro's `background`, `foreground`, `accent`, `selection`, and `color8` values into its semantic tokens and monitors the current-theme state for changes. It defaults to following Omarchy on first launch.

Syntax colors follow Quattro's named `magenta`, `green`, `orange`, `cyan`, and `yellow` colors. Older terminal palettes can supply `color5` for keywords, `color2` for strings, `color9` for constants, and `color3` for types. Missing colors retain the existing source-palette or derived fallback.

The system option is not shown when a valid Quattro current-theme state is unavailable. If that state disappears while Strata is running, following turns off and Strata returns to the selected built-in theme. Legacy Omarchy theme layouts and alacritty-based color extraction are intentionally unsupported.

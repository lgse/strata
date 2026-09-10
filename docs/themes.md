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

### Code preview colors

Code and Markdown previews derive their syntax colors from the theme: keywords use the accent color, comments use dim text, and strings, constants, and types are blends of accent and text. A theme can override each slot with four optional tokens:

```toml
syntax_keyword = "#ff66f6"
syntax_string = "#00f59b"
syntax_constant = "#ff9a8f"
syntax_type = "#ffea00"
```

Every syntax token is optional and independent; any slot you omit keeps its derived color. Values may use any CSS color format and are written to the preview scheme as `#rrggbb`. Invalid values fail theme validation like the nine required tokens. Bundled themes do not set syntax tokens and always use the derived palette.

## Omarchy Quattro

On Omarchy Quattro, Strata detects the active theme from:

```text
~/.local/state/omarchy/current/theme.name
~/.local/state/omarchy/current/theme/colors.toml
```

The application maps Quattro's `background`, `foreground`, `accent`, `selection`, and `color8` values into its semantic tokens and monitors the current-theme state for changes. It defaults to following Omarchy on first launch.

When Quattro provides the standard terminal palette, Strata also maps `color5` to syntax keywords, `color2` to syntax strings, `color3` to syntax types, and `color9` to syntax constants, following the conventional Base16 roles for those slots. Slots the palette omits keep their derived colors.

The system option is not shown when a valid Quattro current-theme state is unavailable. Legacy Omarchy theme layouts and alacritty-based color extraction are intentionally unsupported.

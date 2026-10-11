# Themes

Strata styles the interface with nine semantic color tokens. Bundled themes are the fallback on any Linux desktop; Azure Glow is the default. Settings presents all 95 bundled themes in one searchable, light/dark-filterable scrolling catalog.

Tinted Base16 entries map colors to Strata tokens as follows: `base00` to background, `base01` to surface, `base05` to text, `base0D` to accent, `base08` to danger, `base02` to muted and highlight, `base03` to border, and `base04` to dim text. Source revision and licensing details are recorded in [`THIRD_PARTY_LICENSES.md`](../THIRD_PARTY_LICENSES.md).

## GTK CSS precedence

Strata defines its named colors as `@strata_bg`, `@strata_surface`,
`@strata_accent`, and other `@strata_*` tokens. These replace the former
`@theme_*` names; user stylesheets referencing those names must be updated.
Custom theme TOML keys are unchanged.

Strata's token provider takes precedence over GTK theme defaults, but user
`~/.config/gtk-4.0/gtk.css` retains GTK's higher user priority. The namespace
prevents accidental collisions with other stylesheets' `@theme_*` colors;
it does not override deliberate user selectors or `@strata_*` definitions.

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

Strata discovers valid `.toml` files in this directory on startup and displays them under **Your themes**. If a custom filename matches a bundled theme ID, the custom theme replaces that bundled entry so saved preferences and selection always use the user’s palette. Colors load as `#rrggbb`, or `#rrggbbaa` when translucent, so GTK-only forms such as 9-digit hex still apply; the file is not rewritten. Interface icons ignore the alpha.

Theme files may be symlinks into a dotfiles repository. The configurator never overwrites an existing theme file: when `<theme-id>.toml` already exists, whether it loaded or not (for example an invalid file or a link to a missing or broken dotfile), the new theme is saved as the next free `<theme-id>-2.toml`, `<theme-id>-3.toml`, and so on.

## Syntax colors

All 95 bundled themes include explicit code-preview palettes. Tinted Base16 palettes map `base0E` to keywords, `base0B` to strings, `base09` to constants, `base0A` to types, and `base0C` to preprocessor directives. Catppuccin and Tokyo Night use the pinned `catppuccin-mocha` and `tokyo-night-dark` syntax palettes; Azure Glow and Omarchy Light have original curated palettes.

Under **Settings → Theme & appearance → Add a theme**, the syntax color pickers start with the selected theme's colors. Each picker previews changes immediately in open code previews; Cancel restores the selected theme, and Add theme saves all five syntax colors with the interface palette. Closing Settings by any route (Escape, Close settings, a click outside it, or closing the window) discards an unsaved preview the same way. Opening the editor again starts from the selected theme's colors. Changing text size or glow while previewing keeps the preview until it is cancelled or saved. **Dim text / comments** controls comments as well as dim interface text. Markdown headings continue to use the accent color.

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

The application maps Quattro's `background`, `foreground`, `accent`, `selection`, and `color8` values into its semantic tokens and monitors the current-theme state for changes, including edits to `colors.toml`. It defaults to following Omarchy on first launch.

Under **Settings → Appearance → Follow Omarchy**, **Omarchy variant** offers:

- **Original** (default): the existing palette mapping, unchanged.
- **Darker**: dark surfaces based on the terminal background instead of the brighter ANSI gray, with readable text and accents. Light palettes also become dark interpretations.
- **High contrast**: stronger text and boundary contrast, retaining a light palette's light appearance or a dark palette's dark appearance.

The variants keep palette hues and adjust brightness where necessary; they do not invert colors or substitute another theme. Darker and High contrast derive muted/hover surfaces from the same palette, adjust semantic foregrounds for at least 4.5:1 contrast against those surfaces, and adapt preview syntax colors. High contrast targets 7:1 for primary text and 3:1 for borders. These are semantic-token targets, not a guarantee for every composited or disabled widget state.

The choice is saved as `omarchy_variant = "original"`, `"darker"`, or `"high_contrast"` in Strata's `settings.toml`. It applies before Settings opens and live across windows, menus, dialogs, icons, and previews. Changing the active Omarchy theme retains the variant. Turning off Follow Omarchy disables the selector but remembers its value; bundled and custom themes are unaffected.

Syntax colors follow Quattro's named `magenta`, `green`, `orange`, `cyan`, and `yellow` colors. Older terminal palettes can supply `color5` for keywords, `color2` for strings, `color9` for constants, and `color3` for types. Missing colors retain the existing source-palette or derived fallback. Quattro values load in the same canonical hex form as custom theme files; values GTK cannot parse count as missing.

The system option is not shown when a valid Quattro current-theme state is unavailable. If that state disappears while Strata is running, following turns off and Strata returns to the selected built-in theme. If it only becomes invalid, for example midway through an Omarchy theme switch, the last applied Omarchy palette stays in use until the next valid change. Legacy Omarchy theme layouts and alacritty-based color extraction are intentionally unsupported.

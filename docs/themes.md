# Themes

Strata styles the interface with nine semantic color tokens. Bundled themes are the fallback on any Linux desktop; Tokyo Night is the default, and Azure Glow is used when a saved theme ID is not recognized. Settings presents all 95 bundled themes in one searchable, light/dark-filterable scrolling catalog.

## System light/dark mode

**Follow system light/dark mode** follows only the desktop's light/dark preference; it never loads the desktop's widget theme, icon theme, or accent color. Its primary source is the `color-scheme` key in the `org.freedesktop.appearance` namespace of `org.freedesktop.portal.Settings`, which uses `1` for dark, `2` for light, and `0` for no preference. This supports KDE and sandboxed Flatpak installations. A dark or light portal value is authoritative. For no preference or an unavailable portal, Strata reads GNOME's `org.gnome.desktop.interface` `color-scheme` key without writing it: `prefer-dark` selects dark, while `default`, `prefer-light`, and unexpected values select light. When the switch is on, Strata waits up to 500 ms for the first portal result before applying its initial palette. It uses the GSettings result, or light when absent, after a portal failure, no preference, or timeout; a late reply updates the palette. Changes apply live to Strata's look tokens, interface and file icons, document views, source highlighting, previews, and open settings pages. The setting is hidden and a saved system selection is repaired only after both sources are unavailable.

The saved `theme` remains the family anchor. The effective selection can therefore be its paired theme, and Settings shows that effective palette as both **Current theme** and the selected catalog card. Clicking a catalog card while following the system changes the anchor and keeps following enabled. A custom theme file that overrides a paired stable ID participates in the same family. Unpaired bundled and custom IDs deliberately resolve to themselves for both appearances.

Curated pairs cover Atelier Cave, Dune, Forest, Heath, Lakeside and Seaside; Ayu; Catppuccin Mocha/Latte; Classic; Everforest Hard and Soft/Medium; Flexoki; GitHub default and high contrast; Google; Grayscale; Gruvbox Hard and Material Medium; Horizon; Material Darker/Lighter; Neovim; Nord; One; Oxocarbon; PaperColor; Rosé Pine/Dawn; Solarized; and Tomorrow/Night.

The serialized theme `mode` accepts `"theme"`, `"system"`, and `"omarchy"`. If that schema or key is unavailable, Strata hides the switch and repairs a saved `"system"` mode to `"theme"` at startup. Light/dark mode following and Omarchy following are mutually exclusive.

Tinted Base16 entries map colors to Strata tokens as follows: `base00` to background, `base01` to surface, `base05` to text, `base0D` to accent, `base08` to danger, `base02` to muted and highlight, `base03` to border, and `base04` to dim text. A few bundled entries override the mapped foreground values so body text stays readable: text is at least 4.5:1 on background and 3:1 on surface; the catalog comments each override next to the value. Where the upstream dim text is also unreadable, loading the bundled theme lifts it to the same floors, stopping as soon as they are met so it stays dimmer than text. Source revision and licensing details are recorded in [`THIRD_PARTY_LICENSES.md`](../THIRD_PARTY_LICENSES.md).

## Appearance isolation and look tokens

Strata draws its entire interface with Rune and `strata-kit::Look` tokens rather than adopting desktop widget themes:

- Desktop widget themes, icon themes, and accent colors are never applied to Strata's chrome (application icons in Open With still come from the desktop icon theme via `strata-platform`'s `icon-theme` shim).
- Interface icons (Lucide) and code/config file-type icons (Catppuccin) are bundled into `strata-kit` and follow the active theme live.
- Menus, popovers, and tooltips render inside Strata's GPU window surface, keeping their corners and shadows everywhere.
- **Variable surface transparency & frosted glass** (**Settings → Appearance → Transparency**) scale main background and raised chrome surface alpha independently while keeping text, icons, thumbnails, selection fills, and media at full opacity, paired with Rune's GPU backdrop blur and native Wayland compositor blur (`ext_background_effect_v1` / KWin blur).

## Token overrides

To customize individual look or component tokens beyond the nine semantic theme colors, create an optional TOML file at:

```text
~/.config/strata/overrides.toml
```

It overrides `strata-kit` tokens live after the active theme, Omarchy following, corner roundness, density, and text size are resolved:

```toml
radius.md = 4
color.border_focus = "#ff9e64"

[sidebar.row]
height = 32

[sidebar.hover]
fill = "alpha(@color.text_base, 0.06)"

[dark]
color.bg_base = "#101018"
```

Strata re-reads `overrides.toml` when it changes, so edits apply to open windows without a restart; deleting it restores the active theme's default tokens. Unknown tokens, invalid values, and syntax errors are reported with line and column numbers in **Settings → Appearance** while valid entries still apply. (When upgrading from an older 1.x `~/.config/strata/style.css`, Strata converts supported rules once into `overrides.toml` without modifying `style.css`; see `crates/strata-kit/src/overrides/MIGRATION.md`.)

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

Strata discovers valid `.toml` files in this directory on startup, re-reads them when they change (see [Live updates](#live-updates)), and displays them under **Your themes**. If a custom filename matches a bundled theme ID, the custom theme replaces that bundled entry so saved preferences and selection always use the user’s palette. Colors load as `#rrggbb`, or `#rrggbbaa` when translucent; the file is not rewritten. Interface icons ignore the alpha.

The configurator derives a new file's ID from the theme name, lowercased, keeping ASCII letters and digits and joining the rest with `-` (for example `Ocean Blue` becomes `ocean-blue.toml`). Names without ASCII letters or digits are saved as `custom-theme.toml`. The `name` field keeps the name exactly as typed, so names such as `日本語` are still shown as written.

Theme files may be symlinks into a dotfiles repository. The configurator never replaces an existing theme: when the ID matches a bundled theme or a loaded custom theme, or `<theme-id>.toml` already exists, whether it loaded or not (for example an invalid file or a link to a missing or broken dotfile), the new theme is saved as the next free `<theme-id>-2.toml`, `<theme-id>-3.toml`, and so on. Saving a theme named `Dracula` therefore creates `dracula-2.toml` and leaves the bundled Dracula theme in place.

## Live updates

Strata watches `~/.config/strata/themes/` while it runs, along with the directory of each file a `.toml` symlink there points to, so a theme kept in a dotfiles repository also updates live when its target is rewritten. When a theme file changes, the custom themes are read again; if the theme currently selected resolves to different colors, it is re-applied to every open window, including icon colors and code-preview syntax colors. No restart is needed.

This makes dynamic themes work: a generator can rewrite a theme file in place, for example the [matugen template](#matugen-template) below, which follows the wallpaper and writes `~/.config/strata/themes/matugen.toml` on every change. Select that theme once (Settings → Appearance, or `theme = "matugen"` in `settings.toml`) and open windows follow the file.

Writes are debounced, so a generator touching several files in a row triggers one re-read. If a file is caught mid-write, invalid, or removed, the current appearance stays in place; the next read that produces valid colors applies them.

### Matugen template

[matugen](https://github.com/InioX/matugen) builds a Material You palette from the wallpaper and fills templates with it. This template, contributed in [arqueon/strata-extras](https://github.com/arqueon/strata-extras/tree/main/themes/matugen) (MIT), maps that palette to Strata's interface and syntax colors. It was written against matugen 4.2; older template syntax may differ.

1. Save it as `~/.config/matugen/templates/strata-theme.toml`:

   ```toml
   name = "Matugen"
   background = "{{colors.surface.default.hex}}"
   surface = "{{colors.surface_container.default.hex}}"
   text = "{{colors.on_surface.default.hex}}"
   accent = "{{colors.primary.default.hex}}"
   danger = "{{colors.error.default.hex}}"
   muted = "{{colors.surface_container_high.default.hex}}"
   highlight = "{{colors.secondary_container.default.hex}}"
   border = "{{colors.outline_variant.default.hex}}"
   dim_text = "{{colors.on_surface_variant.default.hex}}"
   syntax_keyword = "{{colors.tertiary.default.hex}}"
   syntax_string = "{{colors.secondary.default.hex}}"
   syntax_constant = "{{colors.tertiary_fixed_dim.default.hex}}"
   syntax_type = "{{colors.primary.default.hex}}"
   syntax_preprocessor = "{{colors.on_surface_variant.default.hex}}"
   ```

2. Register it in `~/.config/matugen/config.toml`:

   ```toml
   [templates.strata]
   input_path = '~/.config/matugen/templates/strata-theme.toml'
   output_path = '~/.config/strata/themes/matugen.toml'
   ```

3. Run matugen once, for example `matugen image ~/Pictures/wallpaper.png`, then select **Matugen** in **Settings → Appearance**.

From then on, every matugen run recolors open windows. Shells that run user matugen templates on each wallpaper change, such as DankMaterialShell, keep Strata in sync with no further setup.

| Strata key | Material color |
| --- | --- |
| `background` | `surface` |
| `surface` | `surface_container` |
| `text` | `on_surface` |
| `accent` | `primary` |
| `danger` | `error` |
| `muted` | `surface_container_high` |
| `highlight` (selection) | `secondary_container` |
| `border` | `outline_variant` |
| `dim_text` | `on_surface_variant` |
| `syntax_keyword` / `syntax_string` / `syntax_constant` / `syntax_type` / `syntax_preprocessor` | `tertiary` / `secondary` / `tertiary_fixed_dim` / `primary` / `on_surface_variant` |

## Syntax colors

All 95 bundled themes include explicit code-preview palettes. Tinted Base16 palettes map `base0E` to keywords, `base0B` to strings, `base09` to constants, `base0A` to types, and `base0C` to preprocessor directives. Catppuccin and Tokyo Night use the pinned `catppuccin-mocha` and `tokyo-night-dark` syntax palettes; Azure Glow and Omarchy Light have original curated palettes.

Under **Settings → Appearance → Add theme**, the syntax color pickers start with the selected theme's colors. Each picker previews changes immediately in open code previews; Cancel restores the selected theme, and Add theme saves all five syntax colors with the interface palette. Closing Settings by any route (Escape, Close settings, a click outside it, or closing the window) discards an unsaved preview the same way. Opening the editor again starts from the selected theme's colors. Changing text size or glow while previewing keeps the preview until it is cancelled or saved. **Dim text / comments** controls comments as well as dim interface text. Markdown headings continue to use the accent color.

Each color swatch in the editor opens Strata's themed color picker (hex entry, hue/saturation picker, and presets), which follows the active theme and updates live, as does the same picker in the custom folder color dialog.

Custom TOML files can override any syntax role independently:

```toml
syntax_keyword = "#ff7b72"
syntax_string = "#7ee787"
syntax_constant = "#79c0ff"
syntax_type = "#ffa657"
syntax_preprocessor = "#d2a8ff"
```

Recognized source languages keep syntax highlighting (via `syntect`) regardless of the loaded text's byte count, line count, or individual line length. Source text still loads incrementally, and the separate 1 MiB preview-read cap remains in effect. Large plain-text files without a recognized language use virtualized rendering.

These fields accept hex (`#rrggbb` / `#rrggbbaa`) and standard CSS color formats. Omitted fields retain the previous accent/text-derived colors, so existing files remain valid. The editor initializes omitted fields from those derived colors when creating a theme. Invalid colors cause a custom file to be excluded at startup. Preview schemes canonicalize colors to opaque `#rrggbb`.

## Omarchy Quattro

On Omarchy Quattro, Strata detects the active theme from:

```text
~/.local/state/omarchy/current/theme.name
~/.local/state/omarchy/current/theme/colors.toml
```

The application maps Quattro's `background`, `dark_background`, `lighter_background`, `foreground`, `accent`, `selection`, and `muted` values into its semantic tokens: the window uses `background` unchanged, panes and headers use `dark_background`, hover surfaces use `lighter_background`, and borders use `muted`. Missing optional values are derived from `background` and `foreground`; ANSI `color8` is not used, because its meaning varies between themes. Strata monitors the current-theme state for changes, including edits to `colors.toml`. It defaults to following Omarchy on first launch.

Under **Settings → Appearance → Follow Omarchy**, **Omarchy variant** offers:

- **Lighter**: the mapping above. File columns use `background`; the sidebar, headers, and preview use `dark_background`.
- **Darker** (default): file columns move onto `dark_background`, so the whole window, including the sidebar, headers, and preview, uses the darker color.
- **Contrast**: the Darker layout with foregrounds raised to contrast targets, retaining a light palette's light appearance or a dark palette's dark appearance (`mode`, or the background's brightness when `mode` is absent).

Lighter and Darker use the theme's colors as authored; Darker only changes which surface each area uses. Contrast keeps the theme's surfaces and selection and raises text to 7:1, dim text, accents, and errors to 4.5:1, and borders to 3:1 against the window, pane, hover, and selection surfaces, adapting preview syntax colors to 4.5:1. A mid-tone surface is darkened or lightened only when no foreground could reach the text target on it. These are semantic-token targets, not a guarantee for every composited or disabled widget state.

The choice is remembered separately for each Omarchy theme, saved under `[omarchy_variants]` in Strata's `settings.toml` keyed by theme name (for example `tokyo-night = "lighter"`). It applies before Settings opens and live across windows, menus, dialogs, icons, and previews. Switching Omarchy themes restores the variant last chosen for that theme, or Darker for a theme without one. Turning off Follow Omarchy disables the selector but remembers its value; bundled and custom themes are unaffected.

Syntax colors follow Quattro's named `magenta`, `green`, `orange`, `cyan`, and `yellow` colors. Older terminal palettes can supply `color5` for keywords, `color2` for strings, `color9` for constants, and `color3` for types. Missing colors retain the existing source-palette or derived fallback. Quattro values load in the same canonical hex form as custom theme files; unparsable values count as missing.

The system option is not shown when a valid Quattro current-theme state is unavailable. If that state disappears while Strata is running, following turns off and Strata returns to the selected built-in theme. If it only becomes invalid, for example midway through an Omarchy theme switch, the last applied Omarchy palette stays in use until the next valid change. Legacy Omarchy theme layouts and alacritty-based color extraction are intentionally unsupported.

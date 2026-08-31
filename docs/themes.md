# Themes

Strata styles the interface with nine semantic color tokens. Bundled themes are the fallback on any Linux desktop; Azure Glow is the default.

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

Strata discovers valid `.toml` files in this directory on startup and displays them under **Your themes**.

## Omarchy Quattro

On Omarchy Quattro, Strata detects the active theme from:

```text
~/.local/state/omarchy/current/theme.name
~/.local/state/omarchy/current/theme/colors.toml
```

The application maps Quattro's `background`, `foreground`, `accent`, `selection`, and `color8` values into its semantic tokens and monitors the current-theme state for changes. It defaults to following Omarchy on first launch.

The system option is not shown when a valid Quattro current-theme state is unavailable. Legacy Omarchy theme layouts and alacritty-based color extraction are intentionally unsupported.

# Strata planned features

Features and enhancements tracked for Strata on Rune, alongside the optimization
work recorded in [performance/large-directory-open.md](performance/large-directory-open.md)
and [performance-baseline.md](performance-baseline.md).

## Variable surface transparency

**Status:** Implemented.

### Goal

Allow users to dial the transparency of Strata's window background surfaces live — without
washing out text, file icons, thumbnails, selection highlights, or image/video previews — so
translucent themes and compositors with native backdrop blur (such as Hyprland on Omarchy)
look sharp and readable.

### How it fits Rune

- **Transparent swapchain at window creation:** `WindowOptions { transparent: true, .. }` is
  passed when constructing windowed Rune surfaces (`crates/strata-app/src/host/windowed.rs` and
  `chooser_window.rs`). On Wayland/wgpu this selects `PreMultiplied` alpha when the compositor
  supports it and falls back cleanly to `Opaque` on uncomposited surfaces.
- **Zero-restart live adjustment:** Because the swapchain carries an alpha channel, disabling
  transparency keeps every surface at 100% opacity (pixel-identical to an opaque window), and
  adjusting any slider or toggling focus updates live without recreating the window.
- **Layered surface-only alpha scaling:** `strata_kit::transparency` scales main background
  surfaces (`bg_base`, `page`) and raised chrome surfaces (`bg_deep`, `surface_raised`,
  sidebar, header, tab strip, footer) independently, keeps floating overlays above a legibility
  floor, and leaves foreground text, icons, thumbnails, media frames, and selection fills at
  full opacity. Regions paint their own surface once to avoid double-compositing.
- **Frosted glass & unfocused multiplier:** When enabled, floating overlays (dialogs, context
  menus, popovers, progress cards) apply Rune's GPU `El::backdrop_blur` alongside a subtle
  frosted border sheen that pairs with native Wayland/Hyprland compositor blur behind
  translucent chrome bands. Losing window focus (`app.window.active`) applies a live multiplier
  to transparency percentages on the fly.
- **Preferences & UI:** All five controls (**Transparency effects**, **Background
  transparency**, **Raised surface transparency**, **Frosted glass**, and **Unfocused window
  multiplier**) live under **Settings → Appearance → TRANSPARENCY**, persist in `settings.toml`
  via `strata_desktop::settings::TransparencyPreferences`, and are localized across all 13
  `data/locales/` catalogs.

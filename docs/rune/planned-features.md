# Strata on Rune: planned features

Features and enhancements planned for the Rune port **after the GTK parity sweep completes**
and **before shipping the Rune release**, alongside the optimization work tracked under
[performance/](performance/README.md).

## Variable surface transparency

**Timing:** Post-parity, pre-release (alongside the large-directory performance work).

### Goal

Allow users to dial the transparency of Strata's window background surfaces live — without
washing out text, file icons, thumbnails, selection highlights, or image/video previews — so
translucent themes and compositors with native backdrop blur (such as Hyprland on Omarchy)
look sharp and readable.

### How it fits Rune

- **Transparent swapchain at window creation:** Pass `WindowOptions { transparent: true, .. }`
  when constructing windowed Rune surfaces (`crates/strata-app/src/host/windowed.rs` and
  `chooser_window.rs`). On Wayland/wgpu this selects `PreMultiplied` (or `PostMultiplied`)
  alpha when the compositor supports it and falls back cleanly to `Opaque` on uncomposited
  surfaces.
- **Zero-restart live adjustment:** Because the swapchain always carries an alpha channel,
  setting surface opacity to 100% renders identically to an opaque window, and dragging an
  opacity slider updates live at full frame rate without recreating the window.
- **Surface-only alpha scaling:** Scale the alpha of background surface tokens in
  `strata_kit::look::Look` (`bg_base`, `bg_deep`, `bg_surface`, header/sidebar/pane/preview
  backgrounds) while keeping foreground text, icons, thumbnails, media frames, and selection
  fills at 100% opacity.
- **Compositor backdrop blur:** On Hyprland and other Wayland compositors with blur enabled,
  any window region painted with `alpha < 1.0` automatically receives native compositor blur
  behind it.

### Open design choices to settle at implementation time

1. **Uniform vs. layered opacity:** A single opacity slider applied uniformly across window
   surfaces, or layered opacity (e.g., sidebar/header chrome slightly more translucent than
   the main file panes, or independent sliders).
2. **Legibility floor for floating chrome:** Keep popovers, context menus, tooltips, and
   modal dialogs either opaque or clamped to a higher minimum opacity so overlapping text
   remains legible over translucent panes.
3. **Preference & UI placement:** Persist the opacity preference in `strata-desktop`
   settings (`Preferences`), expose a live slider in **Settings → Appearance** and/or the
   **Appearance** popover, and localize all new labels across all 13 `data/locales/` catalogs.
4. **CSD & corner interaction:** Verify client-side decorations, rounded window corners, and
   `clear_color()` behavior when `transparent: true` is active across composited Wayland and
   uncomposited fallback sessions.

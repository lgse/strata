# strata-platform

Linux platform shims for [Rune](https://github.com/anomalyco/rune) apps: the Wayland and X11
pieces Rune doesn't ship yet. Any Rune app on Linux can use it. Of Rune's crates it depends on
`rune` only, and each shim adds optional dependencies of its own. It has no app-specific names in
its API.

Each shim is a module behind its own cargo feature, so an app builds only what it uses. Most are
asked of Rune upstream, and a shim is deprecated once Rune ships its feature (see
[Shim lifecycle](#shim-lifecycle)). One isn't: the GLib bridge is outside Rune's scope.

The crate keeps the name `strata-platform` (the owner's ruling; `docs/decisions/S5-upstream.md`,
"Crate name"). The name is taken on crates.io by an unrelated crate, so releases are git tags
only (see [Pinning a release](#pinning-a-release)).

## Features

None is on by default. `dnd`, `clipboard`, `foreign-parent`, `glib-bridge`, `csd`, `settings`, `icon-theme`,
`single-instance` and `media-audio` are implemented; the other modules are placeholders until their work packages land. "Issue" is the upstream ask, drafted in
[`docs/decisions/S5-upstream.md`](../../docs/decisions/S5-upstream.md#draft-index), with its link
once filed.

| Feature | Module | What Rune lacks on Linux | Issue |
| --- | --- | --- | --- |
| `dnd` | `dnd` | System drag and drop: `drag_out` is macOS-only, and Wayland drops reach no handler. Neither takes MIME types beyond file paths. Implemented: drag sources and drop targets with any MIME types (lazy writers), copy, move, ask (and link on X11) following Ctrl and Shift, an image or view-snapshot icon, the outcome, and a typed payload between the app's own windows, on Wayland (`wl_data_device`) and X11 (XDND). On Wayland a drag from another app takes the compositor's action (sway and Hyprland map no modifiers), and only `Dnd::cancel_drag` cancels a drag. Needs libwayland 1.23 on Wayland. | [U1](../../docs/decisions/S5-upstream.md#u1), [U2](../../docs/decisions/S5-upstream.md#u2), [U3](../../docs/decisions/S5-upstream.md#u3), [U9](../../docs/decisions/S5-upstream.md#u9), [U10](../../docs/decisions/S5-upstream.md#u10), [U-P1-1](../../docs/decisions/S5-upstream.md#u-p1-1), [U-P1-2](../../docs/decisions/S5-upstream.md#u-p1-2); upstream [#171](https://github.com/anomalyco/rune/issues/171) |
| `clipboard` | `clipboard` | Custom MIME types, several forms in one write, typed reads, and a notice when another app takes the clipboard. Implemented: copies of any MIME types (lazy writers, images), the offered types and typed reads with limits, change and loss notices, clearing, the file-manager formats (`Files`), and on X11 the hand-off to a clipboard manager at exit, over Wayland data control, `wl_data_device` (mutter) or X11 selections (X11, or Xwayland when Wayland has no route). | [U5](../../docs/decisions/S5-upstream.md#u5), [U9](../../docs/decisions/S5-upstream.md#u9) |
| `csd` | `csd` | Interactive resize, minimise and the window menu for a borderless window, and its maximised state. Implemented: borderless windows on Wayland and X11, a header that moves the window, and window buttons routed to Rune or the window manager. X11 resizes from the edges and has minimise and the window menu through its own connection (mutter and KWin untested). Wayland has no edge resize, minimise or window menu until Rune ships them. | [U6](../../docs/decisions/S5-upstream.md#u6), [U7](../../docs/decisions/S5-upstream.md#u7), [U8](../../docs/decisions/S5-upstream.md#u8), [U9](../../docs/decisions/S5-upstream.md#u9); upstream [#145](https://github.com/anomalyco/rune/issues/145) |
| `foreign-parent` | `foreign_parent` | Making a window transient for another app's window, before it maps, and marking it modal (X11). Implemented: a portal's `wayland:` and `x11:` parent handles parsed, then on Wayland `zxdg_importer_v2` and `set_parent_of` on winit's display, and on X11 `WM_TRANSIENT_FOR`, the dialog type and `_NET_WM_STATE_MODAL`, with an outcome naming the fallback when it can't. Open the window hidden and apply from `App::on_window`. Wayland has no modality (Rune hands out no `xdg_toplevel`), an `x11:` parent leaves a Wayland window parentless, and libwayland below 1.23 (Ubuntu 24.04) leaves it parentless. | [U11](../../docs/decisions/S5-upstream.md#u11), [U9](../../docs/decisions/S5-upstream.md#u9) |
| `settings` | `settings` | Settings portal keys beyond `color-scheme`, and the desktop's double-click interval. Implemented: the text scale, double-click interval, drag threshold, caret blink, animations switch, `reduced-motion` and recent-files switch from the Settings portal, with their changes on Rune's loop. Rune's own double press keeps its fixed 0.5 s. The portal is the only source: a session without one gets the defaults. | [U12](../../docs/decisions/S5-upstream.md#u12), [U13](../../docs/decisions/S5-upstream.md#u13) |
| `icon-theme` | `icon_theme` | The user's icon theme: `ImageCache::app_icon` reads hicolor only (no theme, no `Inherits`, no `index.theme` sizes), takes no icon names, and fails a missing icon like a broken file. Implemented: icons by name, with fallback names, or by file, at a size and scale, as GTK 4's `GtkIconTheme` finds them (`Inherits` then hicolor, `index.theme` sizes, scale directories, unthemed icons and pixmaps), looked up on a thread and decoded by Rune's `ImageCache` into `rune::Pixels`, with "not found" apart from a broken file. Symbolic icons draw as their files paint them, not recoloured; no XPM and no `icon-theme.cache`. | Upstream [#21](https://github.com/anomalyco/rune/issues/21); [U-P6-1](../../docs/decisions/S5-upstream.md#u-p6-1), [U-P6-2](../../docs/decisions/S5-upstream.md#u-p6-2) |
| `media-audio` | `media` | Any audio output: `Player` has no Linux backend, and nothing plays PCM or reports a playback clock. Implemented: an audio sink for interleaved PCM through GStreamer's `appsrc`, whose clock is the device's position (play, pause, seek, flush, volume, events for underruns, stalls, the end and a lost device), and a frame pacer that writes the frame the clock is at into a `rune::Pixels` with no view commit. Needs GStreamer and gst-plugins-base (`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev` to build), and gst-plugins-good for the default output. | Related: upstream [#30](https://github.com/anomalyco/rune/issues/30), [#112](https://github.com/anomalyco/rune/issues/112); [U-P7-1](../../docs/decisions/S5-upstream.md#u-p7-1) (draft, in the P7 issue) |
| `single-instance` | `single_instance` | Single instance on Linux: Rune starts every launch as a new process. Implemented: one primary per app id on the session bus, with a later launch's arguments, working directory and activation token forwarded over GApplication's wire protocol, `org.freedesktop.Application`, and an opt-in `org.freedesktop.FileManager1`. On X11, focusing a window already open ignores the startup id. | Upstream [#22](https://github.com/anomalyco/rune/issues/22); [U-P8-1](../../docs/decisions/S5-upstream.md#u-p8-1) |
| `glib-bridge` | `glib_bridge` | A GLib main loop: Rune never iterates a `GMainContext`, so GIO's async calls, signals and monitors never dispatch. Implemented: `GlibThread` runs one on its own thread, and answers land in app state for `poll`. Needs GLib and GIO (`libglib2.0-dev` to build). | None: outside Rune's scope |

Each module's own docs say what Rune lacks in detail, and the limits of the shim.

## Supported systems

On Wayland, libwayland-client 1.23 or later. `dnd`, `clipboard` and `foreign-parent` reach
winit's display through `wl_proxy_get_display`, which 1.23 added. An older libwayland (Ubuntu
24.04 ships 1.22) is unsupported. The shims still degrade there rather than fail: no Wayland
drag and drop, the clipboard through data control or Xwayland, and a parentless window
(`Reason::NoDisplay`).

## Supported Rune revs

Each release tag supports exactly one Rune rev. Pin that rev, in full, beside the tag.

| Tag | Rune rev | Rust | Released |
| --- | --- | --- | --- |
| Unreleased (`main`) | [`2a44156b21d63b0342a366b26d9222a6edd4622c`](https://github.com/anomalyco/rune/commit/2a44156b21d63b0342a366b26d9222a6edd4622c) | 1.99.1 | — |

## Pinning a release

Rune isn't on crates.io, so neither is this crate: crates.io takes no git dependencies. Releases
are git tags named `strata-platform-v<version>+rune.<rev>`, where `<rev>` is the first seven
digits of the Rune rev the release supports. Pin the tag, and pin Rune at that same rev:

```toml
[dependencies]
rune = { git = "https://github.com/anomalyco/rune", rev = "2a44156b21d63b0342a366b26d9222a6edd4622c" }
strata-platform = { git = "https://github.com/wmfeht/strata-components", tag = "strata-platform-v0.1.0+rune.2a44156", features = ["glib-bridge"] }
```

The tag shown is an example; take a real one from [Supported Rune revs](#supported-rune-revs).

- **Copy the rev exactly, all 40 digits.** Cargo treats `rev = "5d421fd"` and the full rev as two
  different sources. It then builds Rune twice, and this crate's Rune types don't match your
  app's. `cargo tree -e normal --prefix none --format '{p}' --color never | grep '^rune v' | sort -u`
  must print one Rune, ignoring the ` (*)` cargo adds to repeats.
- Your features of `rune` are your own: cargo merges them with this crate's.
- Write the tag with a plain `+` in `Cargo.toml`. Only web links write it as `%2B`.
- To move to a new Rune rev, move both pins together, to a tag that names the new rev.

## Versioning

The crate follows semver. Moving to a new Rune rev is a minor release, because Rune's types
appear in the API. The [changelog](../../CHANGELOG.md) lists every release, with the Rune rev and
Rust version it supports.

## Shim lifecycle

When Rune ships a feature that a module shims:

1. The release that moves to that Rune rev deprecates the replaced items, naming the Rune API to
   use instead. The module docs and the changelog say the same.
2. The next minor release removes them, or the whole module and its feature if Rune replaces
   all of it.

If Rune ships only part of a feature, only the replaced items go. The full checklist is in
[`docs/decisions/S5-upstream.md`](../../docs/decisions/S5-upstream.md#shim-deprecation-checklist).

## License

MIT, like Rune.

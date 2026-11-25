# strata-platform

Linux platform shims for [Rune](https://github.com/anomalyco/rune) apps: the Wayland and X11
pieces Rune does not ship yet. Of Rune's crates it depends on `rune` only, and each shim adds
optional dependencies of its own behind its own cargo feature. Its API carries no app-specific
names.

In Strata's workspace the crate is a workspace member pinned to the same Rune rev as the root
`Cargo.toml` (`3ea44d821ca36473be760af1e462b793434d4b4c`). Most shims correspond to upstream
Rune asks and are deprecated once Rune ships the capability natively; the GLib shims are outside
Rune's scope.

## Features

None is enabled by default. Verify feature isolation with `mise run compile-features`.

| Feature | Module | What Rune lacks on Linux | Upstream issue |
| --- | --- | --- | --- |
| `dnd` | `dnd` | System drag and drop: Wayland (`wl_data_device`) and X11 (XDND) drag sources and drop targets with arbitrary MIME types (lazy writers), copy/move/ask/link actions, image or snapshot icons, outcomes, and typed intra-app payloads. | [#171](https://github.com/anomalyco/rune/issues/171), [#532](https://github.com/anomalyco/rune/issues/532) |
| `clipboard` | `clipboard` | Custom MIME types, multi-format writes, typed reads with limits, app API for `PRIMARY`, change/loss notices, file-manager `Files` formats, and X11 clipboard-manager hand-off on exit, over Wayland data control, `wl_data_device`, or X11 selections. | — |
| `csd` | `csd` | Borderless window controls on Wayland and X11, drag-move headers, and X11 edge resize, minimise, and window menu. | [#145](https://github.com/anomalyco/rune/issues/145), [#309](https://github.com/anomalyco/rune/issues/309), [#310](https://github.com/anomalyco/rune/issues/310), [#313](https://github.com/anomalyco/rune/issues/313) |
| `foreign-parent` | `foreign_parent` | Parenting a portal/dialog window to another app's exported `wayland:` (`zxdg_importer_v2`) or `x11:` (`WM_TRANSIENT_FOR` / `_NET_WM_STATE_MODAL`) handle before mapping. | — |
| `backdrop-blur` | `backdrop_blur` | Native Wayland compositor backdrop blur behind translucent surfaces via `ext_background_effect_v1` and Plasma/KWin `org_kde_kwin_blur_manager`, degrading cleanly when unsupported. | — |
| `settings` | `settings` | XDG Settings portal keys beyond Rune's native `color-scheme`, double-click interval, `animations`, and `reduced-motion` (text scale, drag threshold, caret blink, recent-files, plus exposing the double-click interval getter to custom pointer machines) streamed onto Rune's loop. | — |
| `icon-theme` | `icon_theme` | FreeDesktop icon theme lookup (`Inherits`, hicolor fallback, `index.theme` sizes, scale directories, unthemed icons/pixmaps) decoded via Rune's `ImageCache` into `rune::Pixels`. | [#21](https://github.com/anomalyco/rune/issues/21) |
| `media-audio` | `media` | Zero-GStreamer Linux PCM audio output (`S16Le`/`F32Le`) on a dedicated worker thread with bounded ring backpressure, live volume/mute, latency-adjusted monotonic playback clock, pause/resume rewind, seek flush, EOF drain, and device-loss recovery via runtime `dlopen` of `libpulse-simple.so.0` (PipeWire/PulseAudio) and `libasound.so.2` (ALSA), degrading to a silent real-time clock when headless. | [#525](https://github.com/anomalyco/rune/issues/525) |
| `single-instance` | `single_instance` | D-Bus single-instance primary per app ID with arbitrary byte-string arguments, working directory, activation tokens, `org.freedesktop.Application`, and opt-in `org.freedesktop.FileManager1`. | [#22](https://github.com/anomalyco/rune/issues/22) |
| `glib-bridge` | `glib_bridge` | Dedicated GLib `GMainContext` worker thread for GIO async operations, signals, and monitors, delivering results back for `poll`. | Outside Rune's scope |
| `glib-loop` | `glib_loop` | Embedding the UI thread's GLib `GMainContext` into Rune's loop via a descriptor-waiting helper thread and `AppHandle::update_if` so covered Wayland windows keep dispatching without polling. | [#139](https://github.com/anomalyco/rune/issues/139) |

Each module's Rustdoc documents its exact contract and limits.

## Supported systems

On Wayland, shared-display shims (`dnd`, `clipboard`, `foreign-parent`, `backdrop-blur`) reach
winit's `wl_display` directly through Rune's `WindowHandle::with_raw_display_handle`, working
across `libwayland-client` versions (including 1.22 on Ubuntu 24.04).

## Supported Rune rev

| Workspace | Rune rev | Rust |
| --- | --- | --- |
| `strata` | [`3ea44d821ca36473be760af1e462b793434d4b4c`](https://github.com/anomalyco/rune/commit/3ea44d821ca36473be760af1e462b793434d4b4c) | 1.99.1 |

## Shim lifecycle

When Rune ships a feature natively that a module shims:

1. Move the workspace's pinned Rune rev and deprecate the replaced shim items, naming the
   native Rune API to use instead.
2. Migrate callers in `strata-app` and remove the deprecated shim and feature in a follow-up.

## License

MIT, like Rune.

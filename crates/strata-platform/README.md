# strata-platform

Linux platform shims for [Rune](https://github.com/anomalyco/rune) apps: the Wayland and X11
pieces Rune does not ship yet. Of Rune's crates it depends on `rune` only, and each shim adds
optional dependencies of its own behind its own cargo feature. Its API carries no app-specific
names.

In Strata's workspace the crate is a workspace member pinned to the same Rune rev as the root
`Cargo.toml` (`7a7ed14e93ef00782a6c7c9a672608a9ba778cdb`). Most shims correspond to upstream
Rune asks and are deprecated once Rune ships the capability natively; the GLib shims are outside
Rune's scope.

## Features

None is enabled by default. Verify feature isolation with `mise run compile-features`.

| Feature | Module | What Rune lacks on Linux | Upstream issue |
| --- | --- | --- | --- |
| `dnd` | `dnd` | System drag and drop: Wayland (`wl_data_device`) and X11 (XDND) drag sources and drop targets with arbitrary MIME types (lazy writers), copy/move/ask/link actions, image or snapshot icons, outcomes, and typed intra-app payloads. | [#171](https://github.com/anomalyco/rune/issues/171), [#532](https://github.com/anomalyco/rune/issues/532) |
| `clipboard` | `clipboard` | Custom MIME types, multi-format writes, typed reads with limits, app API for `PRIMARY`, change/loss notices, file-manager `Files` formats, and X11 clipboard-manager hand-off on exit, over Wayland data control, `wl_data_device`, or X11 selections. | — |
| `foreign-parent` | `foreign_parent` | Parenting a portal/dialog window to another app's exported `wayland:` (`zxdg_importer_v2`) or `x11:` (`WM_TRANSIENT_FOR` / `_NET_WM_STATE_MODAL`) handle before mapping. | — |
| `single-instance` | `single_instance` | Extends Rune's `org.freedesktop.Application` (`Activate`/`Open`, shipped in `#22`) with opt-in `org.freedesktop.FileManager1`, arbitrary `OsString` (byte-string) arguments and forwarded `cwd`, activation tokens, and `org.gtk.Application` `CommandLine` wire interop with remote exit status. | — |
| `glib-bridge` | `glib_bridge` | Dedicated GLib `GMainContext` worker thread for GIO async operations, signals, and monitors, delivering results back for `poll`. | Outside Rune's scope |
| `glib-loop` | `glib_loop` | Embedding the UI thread's GLib `GMainContext` into Rune's loop via a descriptor-waiting helper thread and `AppHandle::update_if` so covered Wayland windows keep dispatching immediately without polling. | Outside Rune's scope |

Each module's Rustdoc documents its exact contract and limits.

## Supported systems

On Wayland, shared-display shims (`dnd`, `clipboard`, `foreign-parent`) reach
winit's `wl_display` directly through Rune's `WindowHandle::with_raw_display_handle` and
dispatch their secondary queue on the UI thread via `rune::watch_display_queue`, working
across `libwayland-client` versions (including 1.22 on Ubuntu 24.04).

## Supported Rune rev

| Workspace | Rune rev | Rust |
| --- | --- | --- |
| `strata` | [`7a7ed14e93ef00782a6c7c9a672608a9ba778cdb`](https://github.com/anomalyco/rune/commit/7a7ed14e93ef00782a6c7c9a672608a9ba778cdb) | 1.99.1 |

## Shim lifecycle

When Rune ships a feature natively that a module shims:

1. Move the workspace's pinned Rune rev and deprecate the replaced shim items, naming the
   native Rune API to use instead.
2. Migrate callers in `strata-app` and remove the deprecated shim and feature in a follow-up.

## License

MIT, like Rune.

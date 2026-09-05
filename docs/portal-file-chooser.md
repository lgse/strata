# Strata system file chooser

Strata can serve the XDG Desktop Portal FileChooser interface for portal-aware applications. Native file pickers and applications that do not use the portal are unchanged.

The chooser is deliberately limited to local files and folders. It uses the main app's sidebar, Columns/Grid/Explorer views, type grouping, filters, metadata, previews, and themed controls. Overwrite confirmation uses the same in-window modal as the app.

Wayland applications can provide an exported parent handle. X11 parent handles are not attached; these requests appear as standalone windows.

## Opt in through the app or installer

On the first normal launch after updating to a version with this feature, Strata
asks once whether to replace your Open and Save dialogs. **Nothing changes without
consent.** “Not now”, Escape, closing the offer, or clicking outside it keeps your
current chooser. Portal requests themselves never show the offer.

You can always enable Strata later through **Settings → General → System file
chooser → Configure…**. The same control restores your previous chooser when
Strata is configured. File chooser replacement is separate from making Strata the
default file manager, folder handler, or “Open file location” handler.

The installer asks separately, defaulting to **No**, after placing the binary at
its permanent path. A declined installer offer is remembered so the app does not
ask again. For unattended installation, `--with-file-chooser` opts in;
`--without-file-chooser` keeps your current chooser and suppresses the in-app offer.
A plain `--non-interactive` install does neither: the app can still ask on its first
normal launch. These options require a release containing portal support.

The offer is remembered in `${XDG_CONFIG_HOME:-~/.config}/strata/portal-opt-in-v1`,
shared by the installer, CLI, and all app windows. Administrators can suppress only
the offer with `strata --dismiss-portal-prompt`, without changing portal preferences.

## Per-user installation

Ensure `xdg-desktop-portal` is installed. The Arch/Omarchy installer includes it
when file chooser integration is selected. Close active file dialogs before
enabling or restoring the chooser: setup restarts the portal frontend.

Install Strata at a stable absolute path, then run:

```bash
strata --install-portal
```

This installs the portal metadata and D-Bus activation service below `$XDG_DATA_HOME`, makes Strata the preferred FileChooser while retaining the active backends as fallbacks, reloads D-Bus, and restarts the portal frontend. If no user portal configuration exists, Strata copies the active desktop configuration before changing the FileChooser preference. The command records whether that user override was created or modified so it can be removed safely later.

The generated D-Bus service contains the absolute path of the command being run. Move Strata to its permanent location before installing the portal. For safe activation, every component of the canonical executable path must be owned by the current user or root and must not be writable by other users. D-Bus service-file argument parsing is not shell quoting, so the installer also rejects executable paths containing whitespace, quotes, or backslashes.

### Manual installation

The equivalent commands below use the default XDG locations and an existing installation at `~/.local/bin/strata`:

```bash
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
strata_executable="$(readlink -f "$HOME/.local/bin/strata")"
strata_replacement="${strata_executable//\\/\\\\}"
strata_replacement="${strata_replacement//&/\\&}"
strata_replacement="${strata_replacement//|/\\|}"

install -d "$data_home/xdg-desktop-portal/portals" \
  "$data_home/dbus-1/services" \
  "$config_home/xdg-desktop-portal"
install -m 644 portal/strata.portal \
  "$data_home/xdg-desktop-portal/portals/strata.portal"
sed "s|@STRATA_EXECUTABLE@|$strata_replacement|" \
  portal/org.freedesktop.impl.portal.desktop.strata.service.in \
  > "$data_home/dbus-1/services/org.freedesktop.impl.portal.desktop.strata.service"
chmod 644 "$data_home/dbus-1/services/org.freedesktop.impl.portal.desktop.strata.service"
gdbus call --session \
  --dest org.freedesktop.DBus \
  --object-path /org/freedesktop/DBus \
  --method org.freedesktop.DBus.ReloadConfig
```

The generated D-Bus service must contain an absolute `Exec=` path. If that path contains whitespace, quotes, or backslashes, install Strata somewhere else; D-Bus service-file argument parsing is not shell quoting.

Open `$config_home/xdg-desktop-portal/portals.conf`, preserve its existing `[preferred]` section and settings, and merge Strata into the FileChooser preference:

```ini
[preferred]
org.freedesktop.impl.portal.FileChooser=strata;<existing-backend>;
```

Replace `<existing-backend>` with the backend already configured for the desktop, such as `gtk` or `gnome`. Do not install the placeholder literally and do not replace unrelated portal preferences. The archive's `portal/portals.conf` is an example, not a complete desktop configuration.

Restart the frontend so it rereads portal metadata and preferences:

```bash
systemctl --user restart xdg-desktop-portal.service
```

On a desktop that does not manage the frontend as a systemd user unit, log out and back in instead.

## Keyboard navigation

- File selection fills, keyboard cursor outlines, and pointer-hover suppression
  use the main app's shared input-ownership styling, including when switching back
  to keyboard navigation after using the mouse.
- Open dialogs start with visible keyboard focus. Save dialogs focus
  and select the suggested filename. If focus is lost, an arrow restores it without
  requiring a click.
- Tab/Shift+Tab traverse controls; arrows move between toolbar icons and options.
- Up from the first file row reaches the pane toolbar; Down returns to files.
- Grid arrows follow the visual rows and columns. Explorer arrows follow the
  displayed order, including type grouping. Shift+arrows extend or shrink a range
  across groups; plain arrows select only the focused item.
- Space/Enter activate focused buttons and toggles. Down or Enter opens a focused
  dropdown; its arrows and Enter select an option.
- F2 renames a single selected file or folder. Escape cancels the name editor
  without closing the chooser. Right-click an item for Rename or Properties;
  right-click empty pane space for New Folder. Alt+Enter opens Properties from
  the file list. Properties preserves multi-selection,
  while Rename is disabled for multiple selected items.
- Text fields keep their cursor keys and Ctrl+A. Ctrl+L edits the location;
  Ctrl+F opens the pane filter; Ctrl+Shift+N creates a folder.
- Left from the outer file-list edge or a leftmost Grid cell focuses the visible
  sidebar. Ctrl+Shift+B also focuses it; Right returns to files without changing
  selection. Up from Home reaches the sidebar toggle in the top bar.
- Space on a file toggles preview. Ctrl+Enter accepts a selected folder in a
  folder-selection request.
- Escape dismisses the innermost menu, inline edit, filter, preview, or confirmation
  before cancelling the request. Confirmation dialogs initially focus Cancel.

The X11 keyboard and context-menu regression tests require `xdotool` (or
`STRATA_TEST_XDOTOOL`) and isolated XDG directories. Run each alone under a test display:

```bash
cargo test keyboard_only_controls_and_file_navigation_work_in_every_chooser_view -- --ignored
cargo test chooser_context_menus_and_rename_work_in_every_view -- --ignored
```

## Verification

Confirm that D-Bus can activate Strata and that it advertises FileChooser version 4:

```bash
gdbus introspect --session \
  --dest org.freedesktop.impl.portal.desktop.strata \
  --object-path /org/freedesktop/portal/desktop
gdbus call --session \
  --dest org.freedesktop.impl.portal.desktop.strata \
  --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.DBus.Properties.Get \
  org.freedesktop.impl.portal.FileChooser version
```

The second command should report `uint32 4`. Then open or save a file from a portal-aware application. Only local locations appear in this initial picker; entering a remote URI shows an unsupported-location error.

Portal backend selection happens before a request is sent. Keeping the existing backend after `strata;` lets the frontend choose it when Strata's `.portal` metadata is absent. It does not provide live failover if an already-selected Strata backend crashes during a request.

## Local test tools

### Test a build without changing your desktop portal

From the repository root, use `make run-chooser-dev` to rebuild and open an isolated Save chooser with application choices. Requires Python with PyGObject/Gio and `dbus-daemon`.

```bash
make run-chooser-dev
make run-chooser-dev CHOOSER_CASE=multiple CHOOSER_ARGS="--view grid --group-by-type"
make run-chooser-dev CHOOSER_ARGS="--choices --theme classic-light"
```

This target disables accessibility integration only for the test session, whose private bus does not provide a working accessibility registry. `make run-dev` still launches the normal app.

You can also build Strata and run the dedicated client directly:

```bash
cargo build
python3 scripts/portal-test.py single --binary target/debug/strata
python3 scripts/portal-test.py multiple --binary target/debug/strata --view grid --group-by-type
python3 scripts/portal-test.py directory --binary target/debug/strata --view columns
python3 scripts/portal-test.py filters --binary target/debug/strata
python3 scripts/portal-test.py save --binary target/debug/strata --choices
python3 scripts/portal-test.py savefiles --binary target/debug/strata --choices
```

`--binary` starts a private session bus and backend with disposable settings, cache, and sample files. It never installs portal metadata, changes your preferences, or restarts your desktop services. Closing the chooser prints the actual D-Bus response (`0` for success, `1` for cancellation) and cleans up the private backend. The client returns destinations but does not write to them.

Use `--folder /absolute/path` for your own files, `--theme classic-light` for a light theme, or `--cancel-after 1` to exercise `Request.Close`. Omit `--binary` to call an already-running Strata backend on your session bus. This client tests the backend directly, not portal frontend routing.

Check these interactions:

- Single-selection requests remain single-selection with Ctrl/Shift clicks, including grouped Grid sections. Multiple-selection requests return all selected files.
- Ctrl+L edits the location; Ctrl+F opens the browser filter; F5 refreshes; Ctrl+H or Ctrl+. toggles hidden files. Remote locations show an error.
- Space opens/closes a preview. Escape dismisses a filter/menu/preview before cancelling the chooser.
- Ctrl+Shift+N or the **New Folder** icon beside Refresh in the browser toolbar creates a directory inline. In folder requests, Ctrl+Enter accepts the current folder when the file view has focus.
- The SaveFile fixture suggests an existing filename. **Save** opens a themed overwrite confirmation; cancelling it leaves the chooser open. **Replace** returns the destination.
- File filters and application choices share a compact row beneath the filename, wrapping on narrower windows, and preserve the selected values in the response. Ctrl+A in the filename entry selects the text, not browser files.

### Recreated browser test page

The five-case page from the original PR is checked in at [`scripts/portal-test.html`](../scripts/portal-test.html):

```bash
python3 -m http.server 8765 --bind 127.0.0.1 --directory scripts
```

Open `http://localhost:8765/portal-test.html` in a portal-aware Chromium browser **after enabling Strata as the preferred FileChooser**. It exercises single open, multiple open, directory selection, image/text filters, and saving `strata-portal-demo.txt`. The SaveFile button explicitly writes a short test file to the destination you choose. Each row reports success, cancellation, or an error; the page shows the returned filenames.

The browser must expose the File System Access API, and its Linux file picker must use the portal. If a different chooser appears, check browser portal support and the configured frontend backend preference. Browsers do not expose the portal's `SaveFiles` or application-defined choices; use the dedicated client for those cases.

## Uninstall

Run the matching per-user command:

```bash
strata --uninstall-portal
```

It removes Strata's metadata and activation service and restores the previous user portal configuration. If the configuration changed after installation, it preserves those changes and removes only Strata from the FileChooser preference. It then reloads D-Bus and restarts the portal frontend. If the frontend cannot be restarted automatically, log out and back in.

For a complete Strata uninstall, also remove the application binary and desktop entry as described in the main installation guide.

### Manual uninstall

Remove the Strata metadata and activation service:

```bash
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
rm -f "$data_home/xdg-desktop-portal/portals/strata.portal" \
  "$data_home/dbus-1/services/org.freedesktop.impl.portal.desktop.strata.service" \
  "$data_home/strata/portal-install/state.toml"
rmdir "$data_home/strata/portal-install" 2>/dev/null || true
gdbus call --session \
  --dest org.freedesktop.DBus \
  --object-path /org/freedesktop/DBus \
  --method org.freedesktop.DBus.ReloadConfig
```

Edit `${XDG_CONFIG_HOME:-$HOME/.config}/xdg-desktop-portal/portals.conf`, remove `strata;` from the FileChooser preference while retaining the previous backend, then restart the portal:

```bash
systemctl --user restart xdg-desktop-portal.service
```

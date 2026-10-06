# Strata system file chooser

Strata can serve the XDG Desktop Portal FileChooser interface for portal-aware applications. Native file pickers and applications that do not use the portal are unchanged.

The chooser is deliberately limited to local files and folders. It uses the main app's sidebar, Columns/Icons/List views, List type grouping, filters, metadata, previews, and themed controls. Recent appears when enabled in sidebar preferences and supported by the desktop's recent-files backend; only local targets are listed. Overwrite confirmation uses the same in-window modal as the app.

File-open requests include a **Name** field. Type an existing filename in the current folder, or paste a direct `http://` or `https://` file URL, then click **Open** or press Enter. A URL downloads to a temporary local file returned to the requesting application. A compact circular progress indicator and cancel button appear at the bottom-left of the action bar; downloads without a known size show a spinner and bytes downloaded. Escape cancels the download before the chooser itself. Chooser validation errors, including Name-field file lookup and URL download failures, appear as plain error text in the action bar rather than adding a row above it. The address bar remains navigation-only, and downloads are unavailable in folder and Save requests.

Downloads are named from `Content-Disposition` or the URL path and persist under a `strata-download-*` folder in the temp directory after the chooser closes, so the requesting app can still open them. Folders older than a day are swept when the portal starts or a new download begins. URLs with credentials and automatic redirects are rejected; use the direct file URL.

If a downloaded JPEG, BMP, single-frame GIF or static WebP has no canonical filename matching the **selected filter**, but PNG would match, Strata validates the image in its sandbox and offers **Convert to PNG** / **Cancel**. Detection uses file contents, not the URL extension or HTTP content type. BMP detection requires `BM`, zero reserved bytes in its file header and a known DIB header size (12, 40, 52, 56, 108 or 124 bytes); `BM` alone is not enough.

Extension repair runs only when the bytes sniff as a supported image and a canonical filename for that kind matches the selected filter. If neither that kind's canonical name nor a PNG name matches, the normal filename filter still judges the download: it can be returned under its original server-provided name, such as JPEG bytes named `attachment.pdf` with a `*.pdf` filter. A `.png` name alone does not satisfy a PNG-only filter: JPEG, BMP, static WebP and single-frame GIF bytes still offer conversion, and unrecognized non-image bytes are rejected. Bytes sniffed as PNG open without conversion or sandbox validation, after any filename repair; that includes APNG and corrupt PNG data with a PNG signature. The helper retains its separate PNG validation as defense in case `ConvertImage` is ever handed a PNG; the chooser never sends an accepted PNG to it.

Conversion preserves full resolution, transparency, orientation and compatible embedded colour profiles; animation and other conversion inputs (including TIFF, SVG, AVIF and HEIC) are unsupported. An embedded ICC profile recognized by its `acsp` signature must have a colour space matching the decoded image, or conversion aborts. This intentionally rejects grayscale JPEGs carrying RGB profiles and CMYK profiles even when the JPEG decoder has emitted RGB pixels. An unprofiled CMYK JPEG skips that check and can be converted using the decoder's RGB output; Strata does not add colour conversion or strip incompatible profiles.

Image processing is cancellable and limited to 32 MiB input, 16 megapixels, 16,384 pixels per edge and the sandbox's memory/CPU/time limits. The separate 32 MiB encoded PNG output cap also applies: 16 megapixels of RGBA exceeds 32 MiB before compression, so an image within the pixel cap can still fail with “The converted PNG exceeds the 32 MiB output limit”. Either cap can reject conversion. The decoder's 128 MiB allocation budget is best-effort, not a hard memory bound; pre-decode dimensions and sandbox process limits provide the resource boundary.

Cancel or conversion failure leaves the chooser open without returning the incompatible original. Clicking Open again on the same URL reuses the downloaded file. Changing the name or selected filter invalidates pending image processing or confirmation. After successful conversion, both the cached original and the returned PNG remain in their temporary folders until the one-day sweep; closing the chooser does not remove either file.

Folder-only requests hide regular files in both directory listings and recursive results. File requests keep folders available for navigation. Changing a file-type filter refreshes the current results without clearing the search query; selection and acceptance follow the new filter.

In Save dialogs, selecting a file copies its name into the name input without accepting the dialog. Names that are not valid UTF-8 display with replacement characters, but Save still targets the selected file's exact name while the name input shows it unchanged. The automatic initial selection does not change the suggested name or destination. Selecting a folder changes the destination without changing the name. In Recent, select a file to save in its containing folder, or navigate to a local folder first.

Resizing a Miller column or a List heading in the chooser saves that width as the chooser default, so the next request opens with it. Regular Strata windows also remember widths, using separate browser defaults.

Wayland applications can provide an exported parent handle. X11 parent handles are not attached; these requests appear as standalone windows.

### Initial size in split-window layouts

On Hyprland, requests with a Wayland parent and an application ID can use the
requesting application's window size as an initial sizing hint. Strata queries
Hyprland's local IPC socket before loading the requested directory, and uses the
hint only when exactly one window's current or initial class matches the app ID
(case-insensitively). The query is read-only, limited to 100 ms and a 1 MiB reply,
and does not depend on which window has keyboard focus. Monitor dimensions still
cap the result. Moving the chooser between monitors no longer reapplies its
initial default size over a manual resize.

The exported Wayland handle does not expose parent geometry. Other compositors,
X11 requests, missing or differently named app IDs, multiple matching windows,
and unavailable IPC retain monitor-based sizing. This is a best-effort improvement,
not guaranteed parent-relative sizing on every desktop. GTK's compositor bounds
and the controls' minimum usable size continue to apply.

### Initial placement on Hyprland

On native Wayland under Hyprland, floating choosers open at the center of their
monitor by default, rather than at the center of the calling application. The portal process identifies its
windows as `io.github.lgse.Strata.FileChooser`, separate from the normal file
manager's `io.github.lgse.Strata` identity.

Before showing a chooser, Strata registers the named runtime rule
`strata-file-chooser-center` through Hyprland's IPC socket. The rule matches only
the chooser identity and sets `center`; it does not force floating, resize the
window, or remove its parent/modal relationship. No Hyprland configuration files
are edited. The same rule is refreshed before each chooser, so it also works
after a compositor configuration reload without accumulating rules.

Both Lua and legacy configurations with named window-rule support are handled.
The entire placement request has a 100 ms deadline; unsupported rules, unavailable
IPC, and other compositors retain compositor-default placement. Centering does
not require identifying the calling application's size.

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

The chooser backend is activated on demand and exits after about two minutes without an active request (checked every 10 seconds). An open dialog (including a pending request) keeps it running; closing the last dialog starts the idle countdown again. In-app updates stop old Strata windows and chooser backends, then automatically relaunch the updating window with the replacement binary. External package-manager updates and manual binary replacements do not invoke the in-app retirement step; restart remaining Strata processes or log out after those updates.

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
- Icons arrows follow the visual rows and columns. List arrows follow the
  displayed order, including type grouping. Shift+arrows extend or shrink a range
  across groups; plain arrows select only the focused item.
- Space/Enter activate focused buttons and toggles. Down or Enter opens a focused
  dropdown; its arrows and Enter select an option.
- Enter on a focused file accepts the request. In a multiple-selection request
  with files selected, it returns every selected file, the same as **Open**.
  Enter on a folder opens it.
- F2 renames a single selected file or folder. Escape cancels the name editor
  without closing the chooser. Right-click an item for Rename or Properties;
  right-click empty pane space for New Folder. Alt+Enter opens Properties from
  the file list. Properties preserves multi-selection,
  while Rename is disabled for multiple selected items.
- Text fields keep their cursor keys and Ctrl+A. Ctrl+L edits the location;
  Ctrl+F opens the pane filter; Ctrl+Shift+N creates a folder.
- Left from the outer file-list edge or a leftmost Icons cell focuses the visible
  sidebar. Ctrl+Shift+B also focuses it; Right returns to files without changing
  selection. Up from Home reaches the sidebar toggle in the top bar.
- Space on a file toggles preview. Ctrl+Enter accepts a selected folder in a
  folder-selection request.
- Escape dismisses the innermost menu, inline edit, filter, preview, or confirmation
  before cancelling the request. Confirmation dialogs initially focus Cancel.
- With [10xer mode](10xer-mode.md#file-chooser) on, the chooser uses that keymap
  and footer within the request's limits: **Enter** / **o** choose a file, and
  **Esc** cancels only after dismissing prompts, filters, search, and preview.
  Save dialogs start in the files: **r** edits the name, and **Enter** saves in
  the current folder.

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

The second command should report `uint32 4`. Then open or save a file from a portal-aware application. Only local locations appear in the picker; entering a remote URI in the address bar shows an unsupported-location error. File-open requests accept direct `http(s)` URLs through the Name field.

Portal backend selection happens before a request is sent. Keeping the existing backend after `strata;` lets the frontend choose it when Strata's `.portal` metadata is absent. It does not provide live failover if an already-selected Strata backend crashes during a request.

## Local test tools

### Test a build without changing your desktop portal

From the repository root, use `mise run chooser-dev` to rebuild and open an isolated Save chooser with application choices. Requires Python with PyGObject/Gio and `dbus-daemon`.

```bash
mise run chooser-dev
CHOOSER_CASE=multiple CHOOSER_ARGS="--view list --group-by-type" mise run chooser-dev
CHOOSER_ARGS="--choices --theme classic-light" mise run chooser-dev
```

This task disables accessibility integration only for the test session, whose private bus does not provide a working accessibility registry. `mise run dev` still launches the normal app.

You can also build Strata and run the dedicated client directly:

```bash
cargo build
python3 scripts/portal-test.py single --binary target/debug/strata
python3 scripts/portal-test.py multiple --binary target/debug/strata --view list --group-by-type
python3 scripts/portal-test.py directory --binary target/debug/strata --view columns
python3 scripts/portal-test.py filters --binary target/debug/strata
python3 scripts/portal-test.py png --binary target/debug/strata
python3 scripts/portal-test.py save --binary target/debug/strata --choices
python3 scripts/portal-test.py savefiles --binary target/debug/strata --choices
```

`--binary` starts a private session bus and backend with disposable settings, cache, and sample files. It disables accessibility integration for that isolated backend so it cannot replace the desktop's accessibility bus. It never installs portal metadata, changes your preferences, or restarts your desktop services. Closing the chooser prints the actual D-Bus response (`0` for success, `1` for cancellation) and cleans up the private backend. The client returns destinations but does not write to them.

Use the `png` case to test image conversion: its only filter is **PNG images**, so a pasted JPEG, BMP, static WebP or single-frame GIF URL should offer conversion to PNG. The general `filters` case starts with **Text files**; its **Images** option accepts both JPEG and PNG and therefore does not offer JPEG conversion. To serve your own test images locally, run `python3 -m http.server 8765 --bind 127.0.0.1 --directory /path/to/images`, then paste a direct URL such as `http://127.0.0.1:8765/photo.jpg` into **Name** and click **Open**.

Use `--folder /absolute/path` for your own files, `--theme classic-light` for a light theme, or `--cancel-after 1` to exercise `Request.Close`. Omit `--binary` to call an already-running Strata backend on your session bus. This client tests the backend directly, not portal frontend routing.

Check these interactions:

- Single-selection requests remain single-selection with Ctrl/Shift clicks, including grouped List sections. Multiple-selection requests return all selected files.
- Ctrl+L edits the location; Ctrl+F opens the browser filter; F5 refreshes; Ctrl+H or Ctrl+. toggles hidden files. Remote locations in the address bar show an error. For file-open requests, paste a direct `http(s)` URL in Name and click Open to download and select a temporary local file.
- Space opens/closes a preview. Escape dismisses a filter/menu/preview before cancelling the chooser.
- Ctrl+Shift+N or the **New Folder** icon beside Refresh immediately creates `new folder` (or the first free `new folder (1)`, `(2)`, etc.) and selects its entire name for editing. Enter or clicking away commits a valid name. Escape or an empty/invalid name keeps the allocated default name; the directory is not deleted. Existing files and folders follow the same rename rules; files retain extension-aware name selection. In folder requests, Ctrl+Enter accepts the current folder when the file view has focus.
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

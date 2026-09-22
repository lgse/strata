<div align="center">

<img src="docs/assets/logos/strata-tokyo-night.svg" alt="Strata logo" width="160">

# Strata

**Navigate every layer.** A fast, keyboard-first file manager for modern Linux desktops.

[![CI](https://github.com/lgse/strata/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/lgse/strata/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/lgse/strata?display_name=tag&sort=semver)](https://github.com/lgse/strata/releases/latest)
[![License: MIT](https://img.shields.io/github/license/lgse/strata)](LICENSE)
[![Linux](https://img.shields.io/badge/platform-Linux-FCC624?logo=linux&logoColor=black)](#technical-specifications)

<picture>
  <source media="(prefers-reduced-motion: no-preference)" srcset="docs/assets/strata-demo.gif">
  <img src="docs/assets/strata-columns.png" alt="Strata showing Columns, Icons, and List views, search, a context menu, file properties, and settings" width="1280">
</picture>

<sub>The animation respects reduced-motion preferences. View the [static preview](docs/assets/strata-columns.png).</sub>

</div>

Strata combines spatial Miller-column navigation with familiar Icons and List views, instant fuzzy filename search, rich previews, and native Linux desktop integration. It is designed for Omarchy and works on compatible GTK4 Linux environments.

## Contents

- [Features](#features)
- [Installation](#installation)
  - [Interactive installation](#interactive-installation)
  - [AI-assisted installation](#ai-assisted-installation)
  - [Manual installation](#manual-installation)
- [Usage and desktop integration](#usage-and-desktop-integration)
  - [Desktop entry](#desktop-entry)
  - [Make Strata the Omarchy file manager](#make-strata-the-omarchy-file-manager)
  - [Unlock encrypted volumes on Omarchy](#unlock-encrypted-volumes-on-omarchy)
  - [Network shares](#network-shares)
- [Custom actions and script authoring](#custom-actions-and-script-authoring)
- [Theming](#theming)
  - [Follow Omarchy Quattro](#follow-omarchy-quattro)
  - [Bundled themes](#bundled-themes)
  - [Custom themes](#custom-themes)
- [Release channels](#release-channels)
- [Under the hood](#under-the-hood)
- [Technical specifications](#technical-specifications)
- [Development and documentation](#development-and-documentation)
- [Contributors](#contributors)
- [License](#license)

## Features

- **Three browser modes:** navigable Columns, an Icons grid, and a sortable List table.
- **Keyboard-first control:** directional-key movement, navigation history, location entry, pane filtering, fuzzy search, file operations, and quick previews. An optional footer and F1 shortcut reference help you learn each mode; the footer also highlights when files are available to paste. See [keyboard navigation and paste destinations](docs/keyboard-navigation.md). Optional experimental [minimal mode](docs/minimal-mode.md) hides window and pane chrome and uses Yazi-style keys.
- **Fast recursive search:** press <kbd>Ctrl</kbd>+<kbd>K</kbd> to find files and directories by name or path while the tree is still being indexed. Global search covers Home and all mounted local drives, regardless of the current folder. Hover the search field to see the included locations. The dialog warns when results are incomplete; folder-scoped filtering/search remains separate. URI-native remote shares are not yet included.
- **Rich previews and thumbnails:** native rendered Markdown and static HTML, plus bounded previews for text, source code, images, camera RAW, PDF, audio, and video, with native parser-backed formats isolated from the application. File Properties shows available media resolution, duration, bitrate, codecs, and audio/video rates.
- **Responsive filesystem work:** cancellable directory loading, bounded streaming, incremental monitoring, stable selection, and virtualized large directories.
- **Everyday file operations:** create folders, rename, cut, copy, paste, trash, permanent delete, sorting, hidden files, pins, and history.
- **Remote locations:** browse GIO/GVfs locations such as authenticated SMB shares from the location field.
- **Adaptive appearance:** compact or airy density, six bundled themes, custom themes, and live Omarchy Quattro theme following.
- **Updates in the app:** opt-in automatic checks, release notes, verified downloads, and in-place installation for release binaries.
- **Custom actions:** add your own scripts to the file and folder context menus, with a manager in **Settings → Actions** and background progress in the Jobs dashboard. See [Custom actions](docs/custom-actions.md).
- **System file chooser:** opt in through **Settings → General → System file chooser**, the installer, or `strata --install-portal`; see [portal setup](docs/portal-file-chooser.md).
- **Encrypted-volume unlock:** opt in through the installer, **Settings → General → Desktop integration** on Omarchy, or `strata --install-udiskie-unlock`; restore with `strata --uninstall-udiskie-unlock` (Settings **Restore default** on Omarchy only).

## Installation

Arch Linux and Omarchy are the primary supported environments. Current binaries require **glibc 2.39 or newer** and the runtime libraries listed below.

### Interactive installation

The interactive installer detects the Linux architecture, glibc version, Arch
Linux, and Omarchy 3 or 4. It installs the latest verified stable release and
offers optional desktop-menu, default-folder-handler, "Open file location", system
file chooser, encrypted-volume unlock, SMB, broader image/RAW, and Omarchy keybind
integration:

```bash
curl -fsSL https://raw.githubusercontent.com/lgse/strata/main/install.sh | bash
```

The installer shows every privileged package operation before asking to run it.
It verifies both the published SHA-256 digest and GitHub Actions provenance before
installing anything from the release archive. The binary is installed per-user at
`~/.local/bin/strata`.

For an unattended Arch or Omarchy installation, pass `--non-interactive`. This
installs required dependencies and the binary without prompting; optional
integrations remain disabled unless explicitly selected:

```bash
curl -fsSL https://raw.githubusercontent.com/lgse/strata/main/install.sh \
  | bash -s -- --non-interactive \
      --with-smb \
      --with-raw \
      --with-desktop-entry \
      --with-folder-association \
      --with-omarchy-keybinds \
      --with-udiskie-unlock
```

Each `--with-*` flag implies `--non-interactive`, and folder association implies
the desktop entry and `--with-file-manager`. Use `--with-file-manager` by itself
to enable only "Open file location" integration. File chooser replacement is
separate: use `--with-file-chooser` to opt in, or `--without-file-chooser` to keep
your current chooser and dismiss the one-time in-app offer. Neither folder
association nor an unattended install enables the chooser automatically.
Encrypted-volume unlock is offered on Omarchy 3 or 4 (the installer asks; default
No), and on generic Arch only when `udiskie` is already on `PATH`. The installer
never installs the `udiskie` package. Unattended installs need
`--with-udiskie-unlock`; `--non-interactive` alone and other `--with-*` flags
leave it declined. Restore on Arch with `strata --uninstall-udiskie-unlock`, not
Settings.

Non-interactive package installation requires passwordless sudo or cached credentials. Run `./install.sh --help` for the full option list.

Phone backends are not installed by the script. For optional iPhone/iPad or Android
access, follow [Connecting phones](#connecting-phones) after installation.

### AI-assisted installation

Use this option to have a coding agent install and verify the latest release archive.

Give this prompt to a coding agent with terminal access:

```text
Install the latest stable Strata release from https://github.com/lgse/strata safely.

Before changing anything:
1. Confirm this is a glibc-based Linux system with a graphical GTK4 environment.
2. Detect whether the machine is x86_64 or aarch64 and select only the matching
   *-unknown-linux-gnu archive from the canonical lgse/strata GitHub release.
3. Show me the runtime packages you need and ask before using sudo or changing
   my default file-manager association.

Then:
- Install the required GTK4, GtkSourceView 5, Poppler GLib, Fontconfig, Bubblewrap,
  FFmpeg/GStreamer, and desktop-integration runtime dependencies using the system
  package manager. Add gvfs-smb only if I want SMB support.
- Ask whether I want phone access. On Arch/Omarchy, add gvfs-afc and usbmuxd for
  iPhone/iPad app documents, gvfs-gphoto2 for camera/PTP photo access, or gvfs-mtp
  for Android file transfers, only if requested. Other distributions need their
  equivalent GVfs backends.
- Download the archive and its matching .sha256 file from the latest GitHub release.
- Verify the checksum with sha256sum --check. If GitHub CLI is installed and
  authenticated, also verify GitHub Actions provenance with
  `gh attestation verify <archive> --repo lgse/strata`. Stop if either attempted
  verification fails; never install a binary with an invalid checksum.
- Extract it and install `strata` to ~/.local/bin/strata without overwriting an
  unrelated file. Ensure ~/.local/bin is on PATH.
- Ask whether I want a per-user desktop entry and inode/directory association;
  if yes, install the archive's io.github.lgse.Strata.desktop and io.github.lgse.Strata.svg
  under ~/.local/share, pointing Exec at the installed binary, then refresh the
  desktop database and icon cache.
- Separately ask whether Strata should handle "Open file location" requests. If
  yes, verify no other per-user service provides org.freedesktop.FileManager1,
  then install the archive's io.github.lgse.Strata.FileManager1.service under
  ~/.local/share/dbus-1/services with Exec pointing at the installed binary.
- Ask before editing udiskie config to use Strata for encrypted-volume unlock.
  Do not install the udiskie package. Do not send Arch users to Settings for
  this row; restore on Arch with `strata --uninstall-udiskie-unlock`.
- Launch `strata`, report its installed version/source release, and verify the
  desktop association if one was requested. Do not weaken the preview sandbox.
```

### Manual installation

Install the release archive directly if you prefer to perform each step yourself.

#### 1. Check the architecture and install dependencies

```bash
case "$(uname -m)" in
  x86_64)  target=x86_64-unknown-linux-gnu ;;
  aarch64) target=aarch64-unknown-linux-gnu ;;
  *) echo "Strata has no prebuilt release for $(uname -m)" >&2; exit 1 ;;
esac
printf 'Use the %s release archive.\n' "$target"
getconf GNU_LIBC_VERSION   # must report glibc 2.39 or newer
```

On Arch Linux or Omarchy:

```bash
sudo pacman -S --needed bubblewrap ffmpeg ffmpegthumbnailer fontconfig \
  gstreamer gst-libav gst-plugins-base gst-plugins-good gtk4 gtksourceview5 gvfs poppler-glib
# Optional SMB, AppImage icon, and broader camera RAW support:
sudo pacman -S --needed gvfs-smb imagemagick libraw dcraw squashfs-tools
```

GTK **4.12 or newer** and glibc **2.39 or newer** are required. Other glibc-based distributions may work when they provide equivalent runtime libraries, but their package names and binary compatibility vary. Systems with an older glibc must [build Strata from source](#development-and-documentation).

Device discovery requires the GVfs UDisks2 volume monitor (`gvfs` on Arch and
Fedora; `gvfs-daemons` on Debian/Ubuntu). Without that backend, removable drives
may be absent from Devices. SMB support remains optional. Phones need additional
backends; see [Connecting phones](#connecting-phones).

#### 2. Download and verify

From the [latest release](https://github.com/lgse/strata/releases/latest), download the `.tar.gz` matching `$target` and its identically named `.sha256` file over HTTPS. Then verify its digest:

```bash
cd ~/Downloads
archive="strata-<version>-${target}.tar.gz"
sha256sum --check "${archive}.sha256"
```

If GitHub CLI is installed and authenticated, you can additionally verify the
archive's signed GitHub Actions provenance before extracting it:

```bash
gh attestation verify "$archive" --repo lgse/strata
```

Every verification you run must succeed. Extract the archive, install the binary,
and confirm it starts:

```bash
tar -xzf "$archive"
install -Dm755 "${archive%.tar.gz}/strata" "$HOME/.local/bin/strata"
command -v strata
strata
```

If `command -v` fails, add `$HOME/.local/bin` to your shell's `PATH`. Every archive contains `SOURCE_COMMIT`, identifying the exact source revision used by GitHub Actions.

#### Debug a release crash

Download the matching `strata-<version>-<target>.debug` asset from the same release and place it beside the installed `strata` binary, keeping its filename unchanged. Then run `coredumpctl debug strata`; GDB will load its Rust function names and source lines.

#### 3. Update or uninstall

For a manual installation, use **Settings → Updates** for verified in-app updates, or repeat the download, verification, and `install` steps for a newer release. An in-app update also refreshes an already installed desktop entry and application icon from the new archive; it never creates desktop metadata that was not installed before. If the user opted into Strata's system file chooser, the update restarts the portal frontend so subsequent dialogs use the newly installed build. Package-managed installations are updated only by their system package manager. To remove a per-user installation, run `strata --uninstall-udiskie-unlock` before deleting the binary if you opted into encrypted-volume unlock, then:

```bash
rm -f ~/.local/bin/strata \
  ~/.local/share/applications/io.github.lgse.Strata.desktop \
  ~/.local/share/dbus-1/services/io.github.lgse.Strata.FileManager1.service \
  ~/.local/share/icons/hicolor/scalable/apps/io.github.lgse.Strata.svg
update-desktop-database ~/.local/share/applications 2>/dev/null || true
gtk-update-icon-cache -qtf ~/.local/share/icons/hicolor 2>/dev/null || true
```

User preferences and custom themes remain under the XDG configuration directories so an uninstall does not destroy personal settings.

## Usage and desktop integration

Launch Strata with an optional local directory:

```bash
strata                      # home directory
strata ~/Documents          # a specific directory
strata --unlock-volume DEVICE  # unlock an encrypted volume
strata --install-udiskie-unlock   # install Strata as the udiskie unlock handler
strata --uninstall-udiskie-unlock # restore the previous udiskie configuration
strata --version            # print the installed version
```

In the default map, useful shortcuts include <kbd>Ctrl</kbd>+<kbd>K</kbd> for recursive search, <kbd>Ctrl</kbd>+<kbd>L</kbd> for a path or URI, <kbd>Ctrl</kbd>+<kbd>F</kbd> to filter the current pane, <kbd>Ctrl</kbd>+<kbd>Z</kbd> to undo the latest reversible file operation, <kbd>Space</kbd> for preview, <kbd>F2</kbd> to rename, and <kbd>Alt</kbd>+arrow keys for history and parent navigation.

Toggle experimental [minimal mode](docs/minimal-mode.md) with <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>M</kbd> for Yazi-style navigation and footer prompts; <kbd>q</kbd> returns to the default map. In minimal mode, <kbd>l</kbd>/<kbd>→</kbd> enters preview, <kbd>h</kbd> returns to the listing without closing it, and <kbd>i</kbd> toggles it.

In narrow or highly scaled windows, the preview uses the full content area rather than disappearing. Close it with <kbd>Space</kbd> (default map), <kbd>i</kbd> (minimal mode), or its close button to return to browsing. Widening the window restores the side-by-side preview and its session width without reloading the file.

### Desktop entry

Every release archive ships `io.github.lgse.Strata.desktop` and the Strata application icon `io.github.lgse.Strata.svg`. Install both to give Strata a per-user launcher with its own icon in launchers, docks, task switchers, and window overviews, and optionally make Strata the default directory handler:

```bash
cd ~/Downloads/"${archive%.tar.gz}"
install -Dm644 io.github.lgse.Strata.svg \
  ~/.local/share/icons/hicolor/scalable/apps/io.github.lgse.Strata.svg
install -d ~/.local/share/applications
sed "s|^Exec=strata |Exec=$HOME/.local/bin/strata |" io.github.lgse.Strata.desktop \
  > ~/.local/share/applications/io.github.lgse.Strata.desktop
update-desktop-database ~/.local/share/applications
gtk-update-icon-cache -qtf ~/.local/share/icons/hicolor 2>/dev/null || true
xdg-mime default io.github.lgse.Strata.desktop inode/directory
xdg-mime query default inode/directory
```

The final command should print `io.github.lgse.Strata.desktop`. The desktop entry's filename matches the `io.github.lgse.Strata` application ID that Strata's windows report, so desktop shells match a running window to this entry and draw its `Icon` value. Log out and back in if a shell caches launcher icons.

When building from source, `mise run install-local` installs the binary, icon, and desktop entry in the same locations, and `mise run uninstall-local` removes them.

### "Open file location" from other applications

Browsers and GTK/GNOME applications reveal a file by calling the `org.freedesktop.FileManager1` D-Bus interface instead of consulting the `inode/directory` association. The interactive installer offers this separately; for unattended installation, pass `--with-file-manager`. Folder association enables it automatically.

For a source installation, enable Strata as the per-user activatable provider explicitly:

```bash
mise run install-file-manager
```

For an AUR package, copy its inactive service template into your per-user service directory:

```bash
install -Dm644 /usr/share/strata/io.github.lgse.Strata.FileManager1.service \
  ~/.local/share/dbus-1/services/io.github.lgse.Strata.FileManager1.service
```

For a release archive installation, install the included service manually instead:

```bash
cd ~/Downloads/"${archive%.tar.gz}"
install -d ~/.local/share/dbus-1/services
sed "s|^Exec=/usr/bin/strata |Exec=$HOME/.local/bin/strata |" \
  io.github.lgse.Strata.FileManager1.service \
  > ~/.local/share/dbus-1/services/io.github.lgse.Strata.FileManager1.service
```

A per-user provider takes precedence over system providers shipped by other file managers. Before enabling Strata manually, remove any other per-user service whose `Name` is `org.freedesktop.FileManager1`; two providers for the same name in one service directory are chosen arbitrarily. If another file manager already owns the bus name, exit it before testing. Use `mise run uninstall-file-manager` for a source installation, or remove the per-user service file, to disable Strata again.

Strata then answers `ShowFolders`, `ShowItems`, and `ShowItemProperties`, opening the directory that holds the named items with those items selected:

```bash
busctl --user call org.freedesktop.FileManager1 /org/freedesktop/FileManager1 \
  org.freedesktop.FileManager1 ShowItems ass 1 "file://$HOME/Downloads" ""
```

### Make Strata the Omarchy file manager

The XDG association above handles folders opened by applications. On current Lua-based Omarchy releases, also override the stock Nautilus shortcuts in `~/.config/hypr/bindings.lua` so Omarchy launches Strata directly.

First inspect the active bindings and back up your user configuration:

```bash
omarchy menu keybindings --print | grep -i "file manager"
cp ~/.config/hypr/bindings.lua ~/.config/hypr/bindings.lua.bak.$(date +%s)
```

Append these overrides to `~/.config/hypr/bindings.lua`:

```lua
-- Use Strata instead of Nautilus for Omarchy's file-manager shortcuts.
hl.unbind("SUPER + SHIFT + F")
hl.unbind("SUPER + ALT + SHIFT + F")
o.bind("SUPER + SHIFT + F", "File manager", { launch = "strata" })
o.bind("SUPER + ALT + SHIFT + F", "File manager (cwd)",
  "uwsm-app -- strata \"$(omarchy-cmd-terminal-cwd)\"")
```

The stock shortcut is <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd>, not <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd>. To support the Ctrl chord too, first confirm that it is not assigned to another action, then optionally append:

```lua
o.bind("CTRL + SHIFT + F", "File manager", { launch = "strata" })
```

Apply and validate the configuration:

```bash
hyprctl reload
hyprctl configerrors
omarchy menu keybindings --print | grep -i "file manager"
```

`hyprctl configerrors` should produce no errors. These user overrides survive Omarchy updates; do not edit files under `/usr/share/omarchy/`.

### Unlock encrypted volumes on Omarchy

On Omarchy 3 or 4, the installer asks whether to use Strata for encrypted-volume unlock (default No). **Settings → General → Desktop integration** includes **Unlock encrypted volumes**. Choose **Use Strata** so plugging in an encrypted drive opens Strata's password prompt instead of udiskie's dialog. **Restore default** returns udiskie to its previous configuration.

On generic Arch, the installer offers the same step only when `udiskie` is already on `PATH`. Restore there with `strata --uninstall-udiskie-unlock`; Settings for this row stays Omarchy-only.

The same setup is available unattended:

```bash
strata --install-udiskie-unlock
strata --uninstall-udiskie-unlock
```

This edits `~/.config/udiskie/config.yml` and records restore state in `~/.local/share/strata/udiskie-install/state.toml`. It is not a saved Strata preference. The Settings row appears only when Omarchy is detected and `udiskie` is on your `PATH`; the CLI flags are unattended and are not Omarchy-gated. Unattended installer opt-in is `--with-udiskie-unlock`.

### Connecting phones

Strata discovers and mounts phones through GIO/GVfs. The required phone backends
are optional and are not installed by `install.sh`. On Arch Linux or Omarchy,
install only the support you need:

```bash
# iPhone or iPad app documents (AFC):
sudo pacman -S --needed gvfs-afc usbmuxd
# Camera/PTP photo access, including compatible iPhones:
sudo pacman -S --needed gvfs-gphoto2
# Android (MTP):
sudo pacman -S --needed gvfs-mtp
```

Other distributions need equivalent GVfs AFC, gphoto2/PTP, or MTP backends;
package names vary.

- **iPhone/iPad:** unlock the device, connect it with a USB data cable, and accept
  **Trust This Computer** (enter the device passcode if requested). AFC can expose
  an app document-sharing view containing folders named after apps; this is not
  the photo library. For photos, install the gphoto2 backend and look for a
  separate camera/device entry with **DCIM**, if exposed by the phone. Accept any
  photo-access prompt. Photos stored only in iCloud may not be available over USB.
  Access depends on the iOS version and backend support; iOS does not expose
  unrestricted internal storage.
- **Android:** unlock the device, connect it with a USB data cable, and select
  **File transfer / Android Auto** or **MTP** in its USB preferences rather than
  charging-only mode. Accept any file-access prompt. Only storage exposed by the
  phone is available, not protected system files or private app data.

After installing a backend, fully quit Strata (all windows) and reopen it.
Reconnect the unlocked phone if necessary, then click its entry under **Devices**.
The camera/PTP entry opens a single **Photos** view: files appear progressively
from storage/date folders, prioritizing newer date-folder names, without requiring
you to open each folder. The backend may finish a folder's metadata before
returning its first batch. This is
a virtual listing of JPEG, HEIC/HEIF, MOV, MP4, and recognized camera RAW files,
not a reorganization of the phone. Sidecars such as `.AAE` and other formats are
hidden only from this Photos view; their originals remain untouched and visible
in normal folder browsing. Duplicate filenames remain
separate files with their original locations; use **Copy path** or **Properties**
to distinguish their sources. Preview, copy, and delete act on those originals.
This is the USB-exposed collection, not iOS's Albums hierarchy. The current
backend does not provide album membership for a reliable Albums/Camera Roll split.

Camera batches yield to interface input and redraws. In List view, file-type
grouping is applied after discovery finishes; the saved grouping setting is
preserved while the live listing stays ungrouped.

Discovery reads metadata, not every photo's contents. It skips symlinks, avoids
revisiting discovered directories, and preserves hidden-file filtering. Photos
keeps loading in batches until the whole exposed library is indexed, you navigate
away or refresh, or the device reports an error. There is no overall scan
deadline or fixed file, folder, or nesting limit. Refresh to rescan after a
phone-side change; not every device supports live change notifications.

Android MTP and iPhone app-document/AFC entries retain normal folder browsing.
For Android, open **Internal storage** (the label varies by device).

Still photos (including HEIC) and MOV/MP4 videos can use the preview pane or
<kbd>Space</kbd> quick preview directly from the phone, without a manual copy.
Strata first downloads a private temporary input for sandboxed decoding: up to
64 MiB in 30 seconds for images or 256 MiB in 60 seconds for videos, with at most
four staged inputs per process. Video playback waits for that download, then
reuses it for seeking and resizing. Closing or changing the preview cancels the
request; temporary files are removed after the player and its workers release
them. Remote PDFs, animated GIFs, audio, and other video formats still need a
local copy for preview. HEIC decoding requires an installed HEIC-capable image
decoder, such as ImageMagick with libheif. Camera/PTP thumbnails use small previews
provided by the camera, with bounded retrieval and sandboxed decoding; if the
camera cannot supply a thumbnail, Strata keeps the file icon instead of downloading
the original. Camera thumbnails are cached only in memory. See
[Remote previews](docs/preview-sandbox.md#remote-still-image-previews)
for cleanup, caching, and sandbox details.

If the phone is missing, check the backend package, try another data cable or USB
port, and confirm the trust/file-transfer setting. On Arch/Omarchy, `lsusb` (from
`usbutils`) can confirm USB detection, but detection alone does not establish file
access. For iPhone/iPad, also check `systemctl status usbmuxd.service` while the
phone is connected. If the newly installed backend still is not discovered after
restarting Strata, log out and back in to refresh the desktop's GVfs services.

#### iPhone appears but photo storage is empty

Some recent iPhones can expose an empty camera/PTP store with libgphoto2 2.5.34,
even when unlocked, trusted, and holding locally stored photos. This is a known
[upstream libgphoto2 issue](https://github.com/gphoto/libgphoto2/issues/1254), not
necessarily an empty photo library or a Strata display problem. The backend
mishandles the folder-parent information returned by these devices.

A read-only test with an iPhone reporting iOS 26.6.1 reproduced the problem:
unmodified libgphoto2 2.5.34 listed **0 folders**, while the same version with
[upstream fix `9f5d4f9`](https://github.com/gphoto/libgphoto2/commit/9f5d4f9ca0a7f58bac7987180a48154ea07c090f)
listed **117 folders**. Both builds were temporary, with their actual library
loading verified; no photos were downloaded or modified. This confirms folder
listing with the fix on that device, not complete transfer or Strata GUI coverage.

Use a distribution libgphoto2 update or backport containing that fix when
available. Merely reinstalling `gvfs-gphoto2` or restarting Strata will not fix an
affected libgphoto2 build. Strata's GVfs camera backend must load the corrected
library; setting library paths only for Strata may not affect the separately
launched GVfs process. This documentation change does **not** bundle or install
the fix. Avoid replacing system libraries manually; any locally built workaround
should be isolated and reversible.

### Network shares

Press <kbd>Ctrl</kbd>+<kbd>L</kbd>, enter an address such as `smb://server/share`, and press <kbd>Enter</kbd>. Strata uses GIO/GVfs and prompts for credentials when required. Install your distribution's SMB GVfs backend (`gvfs-smb` on Arch) to enable SMB browsing.

## Custom actions and script authoring

Use **Settings → Actions → New action…** to create an action, or ask a coding
agent to create the files below. **Script → Library** supplies editable Python
and Bash recipes, sets their runtime, file filters, and **Whole selection / Per
item** mode, and never saves or executes them merely by selecting them.

For Python and Bash, the Script tab reports whether the interpreter can be found.
Python uses the script's shebang, or `python3` when there is none. Availability is checked
automatically when the dialog opens; reopen it after installing a runtime.
Bash and Command do not need Python. This checks
executable availability, **not** script syntax, safety, or dependencies such as
ImageMagick, FFmpeg, or ExifTool.

### Files and manifest

Actions are ordinary, user-owned folders under
`${XDG_CONFIG_HOME:-$HOME/.config}/strata/actions/`:

```text
strata/actions/log-selection/
├── action.toml
└── main.py
```

The folder name and manifest `id` must match. Use a new lowercase kebab-case id;
do not overwrite an existing action. Create directories with mode `0700` and
manifest/script files with mode `0600`. Entry points are regular files with plain
filenames, not symlinks or paths. Scripts need no executable bit: Strata invokes
the interpreter. Command actions have only a manifest. Import/export transfers
the manifest and declared entrypoint, not arbitrary helper files.

Example `action.toml`:

```toml
schema_version = 1
id = "log-selection"
name = "Log selected paths"
description = "Print the selection without modifying files"
icon = "terminal"
enabled = false
menu = "submenu"

[when]
kinds = ["file"]
min_items = 1

[run]
runtime = "python"
entrypoint = "main.py"
mode = "whole-selection"
on_error = "continue"
working_directory = "parent"
confirm = true
```

Keep generated actions disabled until the user reviews them. After hand-writing
files, restart Strata to reload them, then review/enable the action in Settings.
All selected entries must match the filters. Scripts are trusted local programs
with the user's permissions—**not sandboxed**.

### Python context

Put this in `main.py`; the helper is supplied by Strata at invocation time, with
no pip installation or copied helper module:

```python
#!/usr/bin/env python3
from pathlib import Path
from strata_actions import context

ctx = context()
for index, path in enumerate(ctx.paths, start=1):
    ctx.log(f"{index}: {Path(path).name}")
    ctx.progress(index, ctx.count, "Logging selected files")
```

- `ctx.paths`: absolute paths for this invocation; `ctx.paths_bytes()` preserves
  native filename bytes. `ctx.count` is their count, **not** the total per-item job
  size; `ctx.single` is the sole path or `None`.
- `ctx.position` / `ctx.total`: 1-based job position and job size in **Per item**
  mode, otherwise `None`. Whole-selection scripts enumerate `ctx.paths` themselves.
- `ctx.parent`: invoking folder. `ctx.directory`: stored action folder.
  `ctx.run_directory`: private temporary scratch, removed after the invocation.
- `ctx.log()`, `ctx.progress(processed, total=None, message=None)`, and
  `ctx.output(absolute_path)` report to Jobs. `output()` only reports a location;
  the script must create it. Import `require_tool(name)` from `strata_actions` to
  explain missing dependencies.

See the complete maintained [`context()` reference](data/actions/context-api.txt)
for identity, metadata, mode/source, and tool lookup. The **Batch rename** and
**Lowercase file names** recipes additionally pass a per-file naming context to
`new_name(context)`; its `filename` and 1-based `index` are recipe-specific, and
`context.batch` exposes the general Strata context.

### Bash and commands

For a Bash script, replace the manifest's `[run]` section with
`runtime = "bash"`, `entrypoint = "run.sh"`, and `mode = "whole-selection"`.
Example `run.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
while IFS= read -r -d '' path; do
    printf 'Selected: %q\n' "$path"
done < "$STRATA_ACTION_PATHS"
```

`STRATA_ACTION_PATHS` is the **name of a file containing NUL-delimited paths**,
not a whitespace-separated list. Always quote paths; never use `for path in
$(cat ...)` or `eval`. `STRATA_ACTION_COUNT` is invocation-local;
`STRATA_ACTION_POSITION` is the 1-based per-item position. The invoking folder is
stored byte-exactly in the file named by `STRATA_ACTION_PARENT`. See the
[environment and progress protocol](docs/custom-actions.md#how-an-invocation-runs)
for context JSON, scratch paths, and progress reporting without Python.

For an installed executable, replace `[run]` with this—no script file:

```toml
[run]
runtime = "command"
program = "sha256sum"
args = ["--", "{paths}"]
mode = "whole-selection"
```

Programs receive direct argv, not shell source: pipes, redirection, `$VARIABLE`,
and globbing are not expanded. `{paths}` supplies separate arguments in whole
selection mode; `{path}` is for per-item mode; `{parent}` is the invoking folder.
Tokens must occupy an entire argument. Use Bash if shell syntax is needed.

**Choose the mode deliberately:** whole selection runs once with all paths;
per item runs once for each path, with Continue/Stop controlling subsequent
failures. Rename/lowercase/count-lines recipes use whole selection; conversions,
checksums, and EXIF stripping use per item. Library selections apply these modes
automatically, including when replacing a recipe from the other mode.

**Agent handoff checklist:** choose a fresh id, write the matching manifest and
entrypoint, declare required tools, preserve originals/refuse overwrites, and
keep the action disabled for review. Test on disposable copies through Strata's
context menu, then inspect Jobs output. Exit `0` means success; nonzero means
failure. Cancelling stops remaining work but does not undo filesystem changes.
The [full custom-actions guide](docs/custom-actions.md) documents validation,
matching rules, storage, examples, and limitations.

## Theming

Open **Settings → Appearance** from the gear menu or with <kbd>Ctrl</kbd>+<kbd>,</kbd>. Theme changes apply immediately across the interface.

Use **Search settings** to filter options across pages and navigate to the closest match, including keywords such as “font size.” In compact windows, the magnifying-glass button opens the search field. Clear the query or press <kbd>Esc</kbd> in the field to restore all settings.

![Strata Theme and appearance settings showing Omarchy following, six bundled themes, and the Add a theme option](docs/assets/strata-themes.png)

### Follow Omarchy Quattro

On **Omarchy Quattro**, turn on **Follow Omarchy** under **Settings → Appearance**. Strata maps the active Omarchy palette to its semantic colors, monitors the current theme, and updates live whenever Omarchy's theme changes.

This integration supports Omarchy Quattro only. The switch is hidden when Strata cannot find a valid Quattro current-theme state; legacy Omarchy theme layouts are not supported.

### Bundled themes

Choose any included theme from **Settings → Appearance**: Azure Glow, Tokyo Night, Catppuccin, Everforest, Rosé Pine, or Omarchy Light. Selecting a bundled theme turns off Omarchy following and keeps that theme active across restarts.

### Custom themes

Select **Add a theme**, enter a name, and choose the semantic colors for the background, surfaces, text, accent, danger, muted and highlighted elements, borders, and dimmed text. Strata previews edits live and saves completed themes under **Your themes**.

Custom themes are stored as shareable TOML files in `~/.config/strata/themes/`. See [Themes](docs/themes.md) for the schema, file location, and Omarchy color mapping.

## Release channels

Strata defaults to the **Stable** channel: only final tagged releases are ever offered, and a Stable install never receives, sees, or is notified about a prerelease.

To try upcoming changes early, choose a channel in **Settings → Updates**. **Preview** receives curated alpha, beta, and release-candidate builds but excludes nightlies. **Nightly** receives every recognised prerelease, including daily development builds. The update dialog and release notes always identify the exact build kind.

When a prerelease installation selects **Stable**, the Updates card immediately offers the newest stable release as the channel target—even when that requires a semantic downgrade—and labels the action **Return to stable**. Preview and Nightly selections use the same card for ordinary forward updates, so channel changes never create a separate competing rollback card.

See [Releasing](docs/releasing.md) for the tag grammar these channels rely on and, for maintainers, how a release candidate is cut and promoted.

## Under the hood

### Why search stays fast

Strata walks and indexes the selected directory tree on a background thread, never on GTK's UI thread. Results appear progressively during that walk. Names and relative paths are normalized once as index entries are created, and each query maintains only the best **100** fuzzy-ranked matches instead of sending an unbounded result set to the interface.

Rapid keystrokes are coalesced to the newest query. During indexing, result publication is throttled to 50 ms intervals; the UI consumes those bounded updates on its own timed loop, keeping rendering aligned with responsive frame-sized work. Exact and contiguous matches, word/path boundaries, and names rank ahead of loose path subsequences.

The deliberate tradeoff: this is fast **filename and path** search, not file-content or metadata search.

### How previews contain untrusted parsers

Files shown while browsing are untrusted. Image, camera RAW, PDF, thumbnail, and media parsing therefore runs out of process through **Bubblewrap**, not inside the main Strata process. Each short-lived helper receives namespace isolation, a minimal read-only runtime, exactly one canonicalized input file, private output and temporary directories, no network, and no capabilities. Memory, CPU/wall time, input, file, and parent-side output limits bound the work.

Only media helpers may receive allowlisted GPU render devices, and only for accelerated decoding; image, PDF, and thumbnail helpers receive no device mounts. Images are normalized to bounded PNG images. Media arrives incrementally as validated raw RGBA frames and fixed-format PCM: GTK presents textures and GStreamer outputs raw audio, without opening the original file or decoding a compressed clip. Four media sessions per process, bounded queues, and paused-worker cleanup limit concurrent work. Cancellation or timeout kills the process group and Bubblewrap PID namespace, tearing down descendants. Missing isolation, crashes, malformed output, timeouts, and permission failures all fail closed to a normal icon or **Preview unavailable**—Strata never silently retries an untrusted native parser without the sandbox.

Plain-text and source previews are different: they stay in process because they do not invoke a native format parser, and reads are capped at 1 MiB. See [Preview sandbox](docs/preview-sandbox.md) for provider ordering, exact mounts, formats, and resource budgets.

## Technical specifications

| Area | Details |
| --- | --- |
| Platform | 64-bit Linux with glibc 2.39+; designed for Omarchy and Wayland. GTK may use another backend supplied by the host, but Wayland is the primary display stack. |
| Release architectures | `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` |
| UI and runtime | Rust 2024, GTK 4.12+, GIO/GLib, Cairo, GtkSourceView 5, Poppler GLib, GDK Pixbuf, GStreamer, and Fontconfig |
| Filesystems | Native Linux paths (including non-UTF-8 names) and GIO/GVfs locations; remote protocol availability depends on installed GVfs backends |
| Preview boundary | Bubblewrap is mandatory for native parser-backed previews; helpers have no network and fail closed. Plain text is read in process with a 1 MiB cap. |
| Optional preview tools | `ffmpegthumbnailer`/`ffmpeg` for video; ImageMagick, classic `dcraw`, and LibRaw `simple_dcraw` expand camera RAW support; `squashfs-tools` (`unsquashfs`) extracts icons embedded in AppImages |
| Hardware acceleration | Media-only VA-API or Vulkan decoding with software fallback; GPU and codec support depend on host drivers |
| Scale targets | Virtualized browser models and bounded asynchronous updates are tested with deterministic directories up to 100,000 entries |
| Packaging | Dynamically linked release archive with SHA-256 digest, GitHub build-provenance attestation, and `SOURCE_COMMIT` |

## Development and documentation

Build requirements are the latest stable Rust toolchain, a C toolchain, `pkg-config`, GTK 4.12+, GtkSourceView 5, Poppler GLib, Fontconfig, and GStreamer 1.20+ (including its app/base development libraries). [mise](https://mise.jdx.dev) pins that toolchain locally (`mise install`). On Arch:

```bash
sudo pacman -S --needed base-devel bubblewrap ffmpeg ffmpegthumbnailer fontconfig \
  gstreamer gst-libav gst-plugins-base gst-plugins-good gtk4 gtksourceview5 gvfs poppler-glib
mise run start-dev        # rebuild and restart as files change
mise run dev              # build and launch the main app once
mise run chooser-dev      # build and open an isolated Save chooser with choices
mise run check            # format, compile, Clippy, tests, and policy checks
```

Start with [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. Deeper references:

- [Keyboard navigation](docs/keyboard-navigation.md)
- [Minimal mode](docs/minimal-mode.md)
- [Architecture principles](docs/architecture.md)
- [Preview sandbox](docs/preview-sandbox.md)
- [Performance baseline](docs/performance-baseline.md)
- [Custom actions](docs/custom-actions.md)
- [Themes and Omarchy integration](docs/themes.md)
- [Unsafe code policy](docs/unsafe-code.md)
- [Releasing](docs/releasing.md)

## Contributors

<a href="https://github.com/lgse/strata/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=lgse/strata" alt="Avatars of Strata contributors linked to the contributors graph">
</a>

This image is generated from GitHub contribution data so new contributors appear without a manual README update. Thank you to everyone who reports issues, improves the documentation, tests releases, and contributes code.

## License

Strata is free software licensed under the **[MIT License](LICENSE)**. Bundled fonts, icons, and other third-party components retain their own notices in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).

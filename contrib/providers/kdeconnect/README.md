# KDE Connect file provider

An optional [external file provider](../../../docs/file-providers.md) that puts
KDE Connect device choice inside Strata's context menu:

- No paired, reachable devices with file sharing enabled: no sharing entry.
- One eligible device: **Send to Pixel via KDE Connect** sends directly.
- Multiple eligible devices: **Send via KDE Connect ▸** opens a device submenu
  on hover or with the keyboard. Choosing a device sends to that device.

The entire selected file set is submitted once. No device-picker window opens.
An accepted submission shows a brief notice; errors and uncertain submissions
use Strata's provider error dialog. KDE Connect manages transfer progress.

## Install or remove

Requires Strata 1.0.0 or later, a running KDE Connect desktop daemon supporting
`shareUrls`, a paired device, and `/usr/bin/python3` (3.10+) with PyGObject/Gio.
For example, install `python-gobject` on Arch Linux or `python3-gi` on Debian or
Ubuntu, alongside KDE Connect. Enable KDE Connect's file-sharing plugin for each
device. This provider does not start the daemon or change pairing settings.

From a trusted Strata source checkout, opt in explicitly:

```bash
/usr/bin/python3 contrib/providers/kdeconnect/install.py
```

This copies the program, message catalogs, and registration into
`${XDG_CONFIG_HOME:-$HOME/.config}/strata/providers/kdeconnect/`. It preserves
other providers, actions and settings. Replaced provider files are backed up
under `${XDG_STATE_HOME:-$HOME/.local/state}/strata-kdeconnect/backups/`.
Close **all** Strata windows and reopen Strata, then right-click selected files.

The installer is independent of Strata's release installer; normal installations
have no new KDE or Python dependency. As with every external provider, this is a
trusted program running with your user permissions. Review it before installing.
The installer refuses symbolic-link destinations and unprotected directories.

To remove it, delete the `kdeconnect` provider directory and restart Strata.
If you previously added a custom KDE Connect action, disable it yourself in
**Settings → Actions** to avoid duplicate entries.

The provider's messages follow the inherited system locale (`LC_ALL`,
`LC_MESSAGES`, then `LANG`) with English fallback, independently of Strata's
language preference. `messages.json` includes all eleven Strata languages.
Device names, filenames and protocol identifiers are never translated.

## Selection and failure behavior

Discovery uses `deviceNames(true, true)` and `hasPlugin("kdeconnect_share")`.
Signals invalidate open menus, with Strata's periodic refresh as a fallback.
Duplicate device names get device-ID hash suffixes. Each leaf binds its exact
selection, file metadata, device ID, and the daemon's unique D-Bus owner into a
short-lived, single-use context token. Activation revalidates those identities
and device eligibility. It never redirects to another device or automatically
retries an uncertain submission.

Only native UTF-8 regular files are supported: no folders, symbolic links,
background actions or remote-only files. The whole selection must be eligible;
files are never silently omitted. Strata limits selections to 200 files and
menus to 64 nodes, so the provider supports up to 63 devices and hides the entry
instead of truncating a larger list. D-Bus work has a five-second request budget.

Files go to the chosen device as encoded local URIs in one
[`shareUrls`](https://invent.kde.org/network/kdeconnect-kde/-/blob/master/plugins/share/shareplugin.h)
call, pinned to the daemon's unique owner. A successful D-Bus reply confirms
**submission**, not per-file receipt. A lost reply is uncertain even if the device
received some files; check KDE Connect and the device before retrying.

Metadata checks reject edited, replaced or removed files before submission.
KDE Connect accepts paths rather than file descriptors, so a file can still
change between the final check and the daemon opening it. This provider does not
lock files or create immutable snapshots. Folder recursion and confirmed receipt
are outside its contract.

## Tests

With the system Python and PyGObject installed:

```bash
/usr/bin/python3 -m unittest discover -s contrib/providers/kdeconnect -v
```

Unit tests cover menu selection, literal URI batches, stale files/devices/tokens,
service restarts, known and uncertain failures, bounds, locale interpolation,
framed subprocess I/O, installation preservation and backups. The D-Bus tests
start a private daemon and a simulated KDE Connect service. No selected content
is read by the mock and no files are transferred to a physical device.

The maintained GUI scenarios use Strata's canonical container runner, private
Xvfb, private session/accessibility buses and a throwaway configuration:

```bash
./scripts/e2e.sh tests/e2e/scenarios/test_kdeconnect_provider.py
```

They exercise direct sending, hover and keyboard submenus, selected-device
routing, whole-selection submission, live device removal and notice feedback.
Mock acceptance does not establish delivery to a phone. For a real transfer,
install the provider, select disposable files, send to your chosen device, and
verify their contents there before using it for documents.

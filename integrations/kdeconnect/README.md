# KDE Connect file provider

An optional [external file provider](../../docs/file-providers.md) that lists
KDE Connect devices in Strata's **Send to…** menu:

- Every paired, reachable device with file sharing enabled appears by name in
  the **KDE Connect** submenu of **Send to…**. Choosing one sends to that device.
- Without eligible devices there is no KDE Connect submenu, and **Send to…**
  disappears when nothing else offers a destination.
- Devices that connect or disconnect while the menu is open are added or
  withdrawn live.

The entire selected file set is submitted once. No device-picker window opens.
An accepted submission shows a brief notice; errors and uncertain submissions
use Strata's provider error dialog. KDE Connect manages transfer progress.

## Install or remove

Install it from **Settings → Integrations → Devices**. It needs a running KDE
Connect desktop daemon supporting `shareUrls`, a paired device, and a system
Python 3.10 or later with PyGObject/Gio; the page checks for `kdeconnect-cli`
and PyGObject and shows the install command for your distribution. Enable KDE
Connect's file-sharing plugin for each device. This provider does not start the
daemon or change pairing settings.

Installing copies the program and message catalogs into
`${XDG_CONFIG_HOME:-$HOME/.config}/strata/providers/kdeconnect/` and takes
effect at once. Normal installations have no new KDE or Python dependency. As
with every external provider, this is a trusted program running with your user
permissions. Review it before installing.

Turn it off with its switch, or select **Remove**, on the same page. If you
previously added a custom KDE Connect action, disable it yourself in
**Settings → Actions** to avoid duplicate entries.

The provider's messages follow the inherited system locale (`LC_ALL`,
`LC_MESSAGES`, then `LANG`) with English fallback, independently of Strata's
language preference. `messages.json` includes all thirteen Strata languages.
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
menus to 64 nodes, so the provider supports up to 63 devices and offers none
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
cd integrations/kdeconnect
/usr/bin/python3 -m unittest discover -s tests -v
```

Unit tests cover menu selection, literal URI batches, stale files/devices/tokens,
service restarts, known and uncertain failures, bounds, locale interpolation
and framed subprocess I/O. The D-Bus tests start a private daemon and a
simulated KDE Connect service. No selected content is read by the mock and no
files are transferred to a physical device. The `Integrations` workflow runs
them whenever files under `integrations/` change.

There is no GUI scenario for this provider yet. Strata's **Send to…** menu
routing is covered by the file provider QA scenarios in
`tests/qa/scenarios/file_providers/`, run in the pinned container with
`./scripts/qa.sh`; device routing, whole-selection submission, live device
removal and notice feedback rest on the unit and D-Bus tests above.
Mock acceptance does not establish delivery to a phone. For a real transfer,
install the provider, select disposable files, send to your chosen device, and
verify their contents there before using it for documents.

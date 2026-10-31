# LocalSend file provider

An optional [external file provider](../../docs/file-providers.md) that hands
selected files and folders to the [LocalSend](https://localsend.org) app:

- **Send to… ▸ LocalSend** appears for any selection of regular files and
  folders. It is a single entry, not a submenu: LocalSend picks the device.
- Choosing it runs LocalSend with the selected paths. A running LocalSend,
  including one in the tray, takes them over and shows its **Send** tab with
  them selected; otherwise LocalSend starts and opens them there. Choose the
  device in LocalSend.

LocalSend discovers devices, asks the receiver to accept, and shows progress and
errors. Strata shows nothing on success. If LocalSend cannot be started or exits
at once, Strata's provider error dialog says so.

## Install or remove

Install it from **Settings → Integrations → Devices**. It needs the LocalSend
desktop app and a system Python 3.9 or later; it uses only the standard library.
No official distribution repository packages LocalSend, so the page shows the
check without an install command. Install it from the AUR (`localsend` or
`localsend-bin`), the upstream `.deb` from the
[releases page](https://github.com/localsend/localsend/releases), or your
distribution's own repository where it has one, such as Omarchy's.

The program is looked up as `localsend`, then `localsend_app` (the upstream
`.deb`), in Strata's trusted system directories, as Strata checks every
integration requirement: `/usr/bin/localsend` may point into `/opt` or
`/usr/lib` only when root owns the target and every directory above it, none
writable by group or others. The provider then runs that target directly, never
the link. Flatpak, Snap and AppImage installs are not found: their launchers
live outside those directories, and a Flatpak would also need the paths
forwarded through the document portal.

Installing copies the program and message catalogs into
`${XDG_CONFIG_HOME:-$HOME/.config}/strata/providers/localsend/` and takes effect
at once. As with every external provider, this is a trusted program running
with your user permissions. Review it before installing. Turn it off with its
switch, or select **Remove**, on the same page.

The provider's messages follow the inherited system locale (`LC_ALL`,
`LC_MESSAGES`, then `LANG`) with English fallback, independently of Strata's
language preference. `messages.json` includes all thirteen Strata languages.
Filenames and protocol identifiers are never translated.

## Selection and failure behavior

Only native UTF-8 regular files and folders are offered: no symbolic links,
background (current folder) menus or remote-only files. The whole selection is
passed in one launch; nothing is silently omitted. Folders are sent with their
contents, as LocalSend's own folder picker does.

The leaf's context is a short-lived, single-use token bound to the exact
selection and each item's identity, size and timestamps. Activation rejects a
stale token, a changed selection, or an item that was replaced, edited, removed
or became unreadable, and consumes the token before launching so a lost reply
cannot open the selection twice.

LocalSend runs in a new session with no access to the provider's protocol
socket, so stopping or restarting Strata never closes it. The provider waits
up to 1.5 seconds: an exit with status 0 means a running instance took over the
paths, and a process still running is the new instance, which loads them as it
starts. Either outcome is reported as accepted. A launch error or another exit
status is rejected. LocalSend adds the paths to anything already selected in
its Send tab.

Acceptance means LocalSend received the paths, not that anything was sent.
LocalSend reads the files when you choose a device, so later edits are what it
sends.

## Tests

With the system Python installed:

```bash
cd integrations/localsend
/usr/bin/python3 -m unittest discover -s tests -v
```

Unit tests cover menu eligibility, literal path arguments, stale files, tokens
and selections, missing and failing launches, the new session, executable
lookup and ownership checks, locale interpolation and framed subprocess I/O.
They launch small stand-in programs, never LocalSend. The `Integrations`
workflow runs them whenever files under `integrations/` change. Strata's
**Send to…** menu routing itself is covered by the file provider QA scenarios.

To try it with the real app, install the provider, select a few disposable
files and a folder, choose **Send to… ▸ LocalSend**, and
check that LocalSend's Send tab lists them, with LocalSend both closed and
already running.

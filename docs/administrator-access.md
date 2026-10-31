# Administrator access

Strata opens local folders the user can't read or enter, such as a backup tool's root-owned
folder on an external drive, through GVfs's `admin://` backend. GVfs runs every operation in
those folders as root after polkit authorizes it, so Strata itself never runs as root.

## Opening a folder as administrator

A folder the user can't read or enter shows a lock icon, and assistive technology hears
"Folder that requires administrator access". Strata learns this with the rest of the folder's
metadata, using `access(2)` so that ACLs count.

- **Opening it** switches the tab to the folder's `admin://` form, such as
  `admin:///run/media/user/drive/backups`, as GNOME Files does. Opening means the click that opens
  folders in that view (set under **Settings → General → Opening items**; a single click in
  Columns by default), <kbd>Enter</kbd>, <kbd>→</kbd> or the context menu's Open.
- **Moving the selection onto it** with the arrow keys never prompts. In Columns the next column
  shows the permission error with **Open as Administrator** and Retry.
- **Typed paths, sidebar places, bookmarks, pins and command-line arguments** keep the permission
  error. Its dialog, column or pane offers **Open as Administrator**.
- **The location bar** also accepts `admin://` addresses typed directly.

Opening an `admin://` address mounts it if needed, then reads the folder before replacing the
current one, so the polkit prompt appears while the previous folder is still open. Cancelling or
failing the prompt leaves that folder open and shows nothing. Strata never shows its own password
form for `admin://`.

**Open as Administrator** is offered only where GIO supports the `admin` scheme, and never in the
system file chooser.

## What GVfs does

- **The prompt.** The first mount starts `gvfsd-admin` through `pkexec`. The session's polkit
  authentication agent shows the prompt, not Strata.
- **The helper.** It keeps running as root after its clients exit. While it runs, a new mount
  needs no prompt.
- **Every operation is authorized separately.** GVfs asks polkit about each one for the calling
  process, and polkit keeps that answer for about five minutes. After that, the next operation
  prompts again. A first visit may therefore prompt twice: once for the helper and once for the
  first read.
- **How a refusal looks.** A refused or dismissed prompt fails the operation with
  `PERMISSION_DENIED`, which Strata treats as "not authorized" rather than as an error.
- **The mount is private to Strata.** Each process gets its own mount, which other programs, the
  sidebar's devices and GIO's unmount calls can't see, so Strata can't unmount it.

## While browsing as administrator

The breadcrumbs start at **Administrator**, and a notice above the listing says that new items
belong to root and that deleted items skip Trash.

- **Writing.** Rename, New Folder, New File, paste, drag and drop and permission changes work
  through GIO, as they do on other non-local locations. Items created here belong to root.
- **No Trash.** GVfs's admin backend has no trash operation, although it reports
  `access::can-trash`.
  - The context menu shows **Permanently delete** instead of Move to Trash.
  - <kbd>Delete</kbd> opens the permanent-deletion confirmation with Cancel focused and the
    reason stated.
  - The sidebar's Trash refuses items dragged from an admin folder.
  - Copies and new items get no Undo entry, because undoing them moves them to Trash. Rename and
    move keep theirs.
- **Replacing.** Replace overwrites a file in place after the conflict dialog warns that replaced
  items will be deleted. A folder is never replaced, and Merge isn't offered.
- **Open in Terminal, custom actions, compress, extract and running programs** stay unavailable,
  as on every non-local location.
- **Session restore** skips admin tabs, so Strata never prompts at startup.

## Disabled on purpose

- **Thumbnails.** Files in admin folders get none. A thumbnail of a root-only image would be saved
  in `~/.cache/thumbnails`, where any program running as the user can read it and where it
  outlives the admin mount. Generating thumbnails is also background reading that could bring
  up a polkit prompt the user didn't cause.
- **Open and Open With.** Opening a file explains why instead of launching another application.
  GVfs gives the admin mount only to the process that asked for it, and `admin://` files have no
  local path. Another application would have to ask for administrator access on its own, with
  another prompt, or couldn't read the file at all if it only accepts paths. To use a file
  elsewhere, preview it in Strata or copy it to a folder you own.

Previews still work. Image and video previews copy the file to a private temporary file (mode
`0600`) that is deleted when the preview changes, like every other non-local preview; see
[Remote still-image previews](preview-sandbox.md#remote-still-image-previews).

## Requirements

- GVfs with its `admin` backend (`gvfsd-admin`, part of `gvfs` on Arch Linux and of
  `gvfs-backends` on Debian and Ubuntu).
- A polkit authentication agent in the desktop session. Without one, polkit refuses every
  request and the folder stays closed.

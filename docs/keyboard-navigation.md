# Column focus and command destinations

Columns have three independent signals:

- **Selection:** filled rows are the items selected in that directory. Other columns retain a quieter selection when you leave them.
- **Keyboard cursor:** a text-contrast outline identifies the current item in the keyboard-focused list. Only that list shows a cursor; range selections can contain several filled rows.
- **Open path:** the chevron identifies the folder whose child column is open, without an extra border. This is navigation context, not another keyboard cursor.

The destination column has an accent rule across its header, including when the directory is empty. Columns no longer reserve a separate bottom margin for the horizontal scrollbar.

## Keyboard layouts

Every shortcut on this page works on any keyboard layout. Letters follow the
key's label, or the Latin letter the key carries on Russian and other non-Latin
layouts, so **Ctrl+C** is the key that types **с** in Russian. Digits count by
their position on the top row, so **Ctrl+1** works on AZERTY without Shift, and
Caps Lock never changes a shortcut. See [keyboard shortcuts](keyboard-shortcuts.md)
for the full rules.

## Browser tabs

In regular and 10xer modes, **Ctrl+T** opens a tab at the active location,
**Ctrl+W** closes the active tab, and **Ctrl+Tab / Ctrl+Shift+Tab** cycles tabs.
**Ctrl+Shift+T** reopens the last closed tab at its previous location and position.
Repeat it to reopen up to 20 recently closed tabs in reverse closure order.
Closed-tab history belongs to the current window and is cleared when it closes;
reopened tabs start with fresh selection, navigation history, and preview state.
**Ctrl+Page Up / Ctrl+Page Down** selects the previous / next tab in strip order,
wrapping at either end, like the tab shortcuts in web browsers.
**Ctrl+Shift+Page Up / Ctrl+Shift+Page Down** moves the active tab one position
left / right without switching tabs. Reordering stops at either end of the strip.
Hold **Ctrl+Shift** to display numbers beside the first ten labels; press
**Ctrl+Shift+1–9** to select tabs 1–9 or **Ctrl+Shift+0** for tab 10.
In 10xer mode, **t**, then **n** creates a tab, **t**, then **x** closes it,
and **t**, then **1–9 / 0** selects tabs 1–10. **t**, then **t** selects the
previous tab in strip order, wrapping from first to last. Escape cancels a pending chord.
The last tab's close shortcut closes the window.
The regular-mode terminal shortcut is **Ctrl+Alt+T**; 10xer keeps **;**, then **t**.

Each tab retains its location, selection, navigation history, preview and search
state. Tab locations persist across restarts: a plain launch (no folder
argument, reveal request, or unlock target) reopens the previous tabs in order
with the previously active tab selected, when **Settings → General → Startup →
Restore open tabs** is on (the default). Credential-bearing URIs and transient
locations such as trashed-item children, non-root Recent entries, and camera
roots are skipped; with nothing restorable the window opens at the default
directory as before. Every saved folder gets its tab at once and is checked off
the interface thread: a folder proven missing or not a folder closes its tab,
and one that cannot be checked, such as on a server that does not answer, keeps
its tab and shows its error. Windows opened for an explicit folder,
reveal request, or unlock target never overwrite the saved session; with
several plain-launch windows open the most recently changed one wins. Selection, history, and preview state stay
in-memory. Appearance preferences and the clipboard remain
shared. File operations prevent closing their tab or window until they finish
or are cancelled.

Tab names follow the active directory, not a child column shown only as a keyboard
preview. A pending single-click folder activation keeps the previous tab name until
navigation resolves; cancelled clicks and drags restore normal focus-based naming.

The Material-style strip appears when there is more than one tab. The plus and
window-close controls move into it and return to the normal header when only
one tab remains. Scroll the strip horizontally (or use the wheel) when it
outgrows the window; selecting a tab brings it into view without a scrollbar.
Drag a tab label to reorder tabs; an accent-colored insertion line shows where
it will land. Sidebar locations offer **Open in New Tab** in their context menu
and open in a new tab on middle-click. Dragging a sidebar location or folders
from a file view marks on the strip where new tabs will open; drop there to
create tabs at that position. **+** also accepts these drags, and is the drop
target when a single tab leaves the strip hidden. Escape cancels the drag.
Folder-only drags from a file view open tabs even over an existing tab header;
they never move or copy the folders into that tab. Opening sidebar locations
also leaves files and sidebar pins unchanged. These tab actions are not offered
in file choosers.

Drop files (or a mixed selection of files and folders) from a listing on a tab
to transfer into its current directory, or hover there during a file drag to
switch tabs and choose a folder in its listing. Existing copy/move modifiers
and conflict handling apply. Escape cancels a file drag before it dismisses
anything else: nothing transfers on release, and the selection, preview, and
filter stay as they were.

## Input precedence

The last navigation input determines the destination of Ctrl+V:

1. Moving, clicking, or scrolling the pointer restores pointer control. Paste targets the directory column under it, not an individual hovered file or folder.
2. Keyboard navigation (arrows, h/j/k/l, Tab, page movement, entering/leaving folders) or Select All restores keyboard control. Paste targets the focused column, falling back to the active column when browser widgets do not hold focus.
3. Ctrl+V itself does **not** change ownership. A parked pointer cannot override subsequent keyboard navigation. Layout/scroll changes underneath an unmoving pointer do not count as pointer motion.
4. Outside the columns, pointer mode falls back to the focused/active directory. Stale depths are discarded. Icons and List continue to use their single active directory.

The terminal shortcut uses the same directory fallback, but still prefers an explicitly selected directory: the one selected in the active column, or in Columns the folder selected in the pointer's column when it is open as the active column, such as a folder just clicked open. New Folder remains keyboard-focus scoped. Context-menu paste and drag-and-drop retain their explicit destinations.

Keyboard navigation suppresses stale row-hover effects and pending folder peeks until deliberate pointer input resumes. It does not erase selection or the open folder path.

## Focus without selection changes

Pressing blank column content focuses that directory, including empty directories, without closing descendants. A plain press on empty space clears file selections across columns immediately; releasing it does not clear them again. When returning to an inactive column, it also selects that column's first visible entry as the range anchor. Ctrl/Shift presses keep the selection until marquee intent is resolved, so a drag can extend it. A plain click on blank space or a disabled control in a preview that is showing something leaves the selection alone; the empty reserved preview slot clears like other empty space. Row clicks, controls, scrollbars, context menus, and marquee selection keep their own interactions. Returning to a column by keyboard preserves a multi-selection; Ctrl+A selects the focused column, not the deepest open column.

**Shift+Up/Down** extends the selection from the range anchor by one item, **Shift+Page Up/Page Down** by one page. After Escape clears the selection, the range starts at the cursor.

Copy/cut use the selection in the focused column, never a hovered row. In Columns, Delete/Shift+Delete with no selected items does nothing: an open parent-path marker is not an implicit deletion target. In List and Icons, Delete acts on the selection only. Delete in a location without Trash support, or after a Trash attempt fails because Trash is unsupported, opens the permanent-deletion confirmation with Cancel focused and the reason stated; Shift+Delete is unchanged. GIO reports Trash on most writable FUSE mounts (sshfs, rclone, cloud drives), so a refusal there is confirmed by a quick, read-only check of the mount and then remembered for that mount until Reload or a remount: later Deletes confirm up front and Move to Trash is hidden. Empty trash folders left by the failed attempt are removed. On network and cloud locations the confirmation says the items are deleted without Trash and that recovery depends on the storage provider, instead of promising permanent deletion; local disks, including `fuseblk` drives, keep the permanent wording, and a mixed selection uses it too. Folders opened as administrator are known to lack Trash, so Delete there confirms permanent deletion up front, with the local wording; see [Administrator access](administrator-access.md).

Background selection updates from directory loading must not move keyboard focus to an inactive column.

## Returning to a visited directory

Icons and List remember the selection, keyboard cursor, and scroll position of the
last 128 directories left in that browser. Every route back to a visited directory
restores it after its entries load, including nested parents: Back, Forward, Up,
breadcrumbs, and typed paths. Arrow-key navigation continues from the restored row.
Entries are matched by location, not their previous row numbers; deleted entries are
not selected accidentally. The selection and cursor carry over between Icons and
List; the exact scroll position comes back only in the view it was left in (and, for
Icons, at the same width), otherwise the cursor is scrolled into view. This is
temporary browsing state, not a saved preference. New input in the file view cancels
an in-progress restoration. Back, Forward and Up move keyboard focus into the restored
listing only when focus was inside the pane being left, its Ctrl+F field included:
from a header button or with the sidebar focused, they restore the selection and
leave focus where it is. Sidebar places, breadcrumbs and typed paths focus the
listing, as on a first visit.

In Columns, Back, Forward, Up and breadcrumbs that return to an ancestor of the
current directory select the folder you came from, with the cursor on it, and leave
its column closed. When that folder is gone or hidden, the first visible entry is
selected instead; hidden files stay hidden. Icons and List do the same when they
have no remembered position for the ancestor.

A navigation that names a target — a typed file path, a Ctrl+K result opened with
Enter or Alt+Enter, Open file location, or an `org.freedesktop.FileManager1`
request — selects that target instead of restoring the remembered position.
A typed file path also opens Quick Preview on the file once it is selected.

## Refreshing a directory

F5, the pane's Refresh button, the rescan after a burst of external changes, and
the re-read that a check of a network folder starts when it finds a change keep
the selection, the keyboard cursor and the keyboard focus in
every view: a focused row stays focused, and a focused Ctrl+F field keeps focus and
its text. Icons and List also keep the scroll position. When the item under the
cursor is gone, the cursor moves to the item now in its place without selecting it.
On a mount that stopped answering, F5 and Refresh keep the rows as they are
instead of clearing them; see
[Unresponsive mounts](filesystem-compatibility.md#unresponsive-mounts).

## Creating files and folders

In Columns, List, and Icons, **Ctrl+Shift+N** or background menu → **New Folder**
immediately creates `new folder`. Background menu → **New File** immediately
creates an empty `new file`. If the default name is occupied by any item, creation
tries `new folder (1)` / `new file (1)`, then `(2)`, and so on without overwriting
anything. The pane filter is cleared and the entire allocated default name is
selected: one Backspace clears it, and typing replaces it.

Item menu → **New Folder with Selection** (or **Ctrl+Alt+N**) creates `new folder`
in the same directory, moves the selected items into it, and names it in place.
With nothing selected **Ctrl+Alt+N** falls back to a plain `new folder`. The item
is hidden in Trash, Recent, and while a recursive search is open.

For **any file or folder rename**, Enter, clicking outside the field (even empty
pane space), or moving keyboard focus away commits a valid name. Switching to
another window, or an input method switcher taking the keyboard, leaves the
edit open. Escape keeps the original name. Finishing with an empty or invalid name also keeps the
original. Cancelling the initial rename does **not** delete the new item: it
remains under its allocated default name. File contents are preserved.
While a name is being edited, **F5**, **Ctrl+K**, **Ctrl+Shift+K**, **Ctrl+Alt+T**
and **Ctrl+\\** do nothing; finish or cancel the edit first.

Clicking inside the field continues editing. Existing files retain extension-aware
selection (the stem is selected); folder names containing dots are selected in full.

Names containing `/` or NUL, `.`/`..`, and whitespace-only names (including
Unicode whitespace) are invalid. Valid names are used exactly as typed,
including spaces around a nonblank name, hidden-file prefixes, and Unicode.
Name conflicts, names the location would store as another (see [Names the user
types](filesystem-compatibility.md#names-the-user-types)), filesystem-specific
limits, and permission errors retain the original item and report an error.

## Filename patterns while filtering

Use **Ctrl+F** to filter a pane. Plain text matches a substring of the filename,
ignoring case. Queries of at least four letters or digits also allow one inserted,
missing, substituted, or adjacent swapped character within a whole filename word.
Words are separated by punctuation or spaces. For example, `trahs` finds
`strata-trash.svg`, but `trash` does not find `strata-search.svg`. Shorter queries
and queries containing punctuation remain literal. A query with spaces first
matches the whole phrase, then names that contain every word in any order, so
`duck txt` also finds `ducks_in_the_pool.txt`. Indexed results rank literal
matches ahead of typo matches and scattered words; parent paths do not qualify
a result.

Add `*` for a whole-filename pattern without typo tolerance:

- `*.MOV` matches `clip.MOV`, but not `clip.MOV.bak`.
- `IMG*` matches names beginning with `IMG`.
- `IMG*.MOV` combines a prefix and an extension.
- `*holiday*` matches names containing `holiday`; `*` matches any name.

Each `*` stands for zero or more characters. Other punctuation (including `?`,
`[]`, and regex syntax) is literal. Patterns match file and folder names, not
parent paths, in Columns, Icons, List, and the file chooser. Hidden-file visibility
and [Include subfolders](preferences.md#filter-scope) still control the scope;
a wildcard does not enable recursive search. Existing result limits still apply.
Clear the input, or press Escape in the input or on a focused result, to restore
the directory listing with focus on its cursor. **Ctrl+K** global fuzzy search is
unchanged.

Results follow the watched folders live: the open folder, and in Columns every open
column. A matching file that another program creates or renames there appears within
about a second, and one that another computer adds to a network folder at its next
check, at first every 3 s in a focused window and less often otherwise
([Cadence](filesystem-compatibility.md#cadence) says when checks slow down or stop).
A deleted one leaves the results as soon as the listing drops it.
With [Include subfolders](preferences.md#filter-scope) on, changes in folders below
those reach the results on **F5**, the rescan after a burst of external changes, or
when the filter is cleared and opened again.
When another program renames the open folder itself, the view follows it and a filter
typed in that folder ends, while filters and **Ctrl+K** searches rooted above it follow
the rename. When the open folder is deleted or moved into another folder, the view
returns to its nearest parent that still exists, or reloads the folder if it already
exists again; either way the filter ends. Neither adds a history entry.

## Preview while filtering

In the browser and file chooser, **Enter** in the Ctrl+F input opens the top result. While the input has focus, the top result is outlined to show it, and stays scrolled into view as results arrive until you scroll the list yourself. Moving to or clicking another result makes Enter open that one instead, and takes the outline off. Changing the query drops that choice and the results' selection, so the outline and Enter return to the new top result. In a folder without a local path, such as a network share, the input narrows the listing itself instead of showing separate results; there a new query keeps the folder's selection, so Space and file actions still act on it, but Enter returns to the new top result all the same. The outline is not a selection: Space and file actions still need a selected result. The [10xer](10xer-mode.md) **f** and **s** prompts keep their own two-step Enter and show no outline.

**Down** from the Ctrl+F input focuses the outlined top result, or the selected result once you have chosen one. **Up/Down** then navigate the results; **Up** from the first result returns to the input without clearing the query. **Ctrl+F** also returns to the input. With no matches, Down leaves focus in the input.

**Menu/Shift+F10** on a focused result opens its file menu. While the input itself is focused, its text-editing menu remains available. **Space** toggles quick preview for a selected file result in Columns, Icons, and List, including after returning to the query. Previewing a file keeps the query, selection, and current directory intact; on a selected folder result, Space navigates into the folder instead.

While the input is focused, Space types into the query if no result is selected. **Shift+Space** inserts a space there even with a result selected. Space acts on a selected result only from the input and the results; the location bar and other fields type it, and the sidebar and toolbar buttons keep their own Space. Space opens a selected folder in every view without opening or loading the preview pane; unsupported files do not open a preview.

With a result focused, the first **Escape** dismisses the filter even while its quick preview is open; a second Escape closes the preview. **Ctrl+1/2/3** keep focus in the filter: in the input with the caret after the query, or on the focused result. With a query typed and focus elsewhere, the results take focus once they show. Loading or refreshing the folder never moves focus out of the input or its results. Neither does another program changing the folder, and that also holds for the [10xer](10xer-mode.md) **f** and **s** prompts.

## Navigating an archive preview

Quick Look on a local ZIP, 7z, TAR, or TAR.GZ opens the archive's member tree
instead of extracting it. In Columns (**Ctrl+1**), automatically previewing an
archive shows its contents on the right without selecting a member or taking
keyboard focus. **Up/Down** continue through the current column. **Right**,
**Enter**, or **Space** explicitly enter the archive preview and highlight its
first member. Enter opens the preview even when automatic previews are disabled;
double-click and the archive's context menu still offer extraction. In Icons
and List, Enter retains its archive extraction behavior.

In every view, moving the cursor onto an archive while the preview is open
updates the preview without taking keyboard focus: a plain archive shows its
members with none highlighted, and a password-protected archive shows its
password prompt unfocused. Opening the preview on an archive with **Space**
enters it directly, highlighting the first member or focusing the password
field. In List and Columns modes, arrow keys act on the focused pane,
not merely on an open preview. From the listing, **Right** enters the open
preview. While the preview owns focus, its header shows the accent top border
instead of the Miller column. Returning to the listing restores its cursor
and column header indicator without changing the listing's selection.

Inside the preview, **Up/Down** (or **k/j**) move the highlight, **Right/l/Enter**
opens the highlighted folder, and **Left/h** returns to the parent. A single
click highlights a member and opens a folder; hovering never changes the
highlight or the keyboard cursor. **Left** at the archive root returns focus to
the listing without closing the preview. Up/Down then move through listing
items; Right enters the preview again. In Columns mode, another Left from the
listing moves to the parent column. Right/Enter on a member file does nothing.
Navigating never extracts anything or touches the filesystem; **Space** and
**Escape** still close the preview. In [10xer mode](10xer-mode.md#preview-keyboard-ownership),
**h** at the archive root returns to the listing, **Space** is swallowed, and
**Shift+Tab** / **Esc** return or close.

## Media preview keys

Audio and video previews show a timeline; **Tab** or a click focuses it. While
it has focus, **Left/Right** seek 5 s, and **Shift+Left/Right** or **Page
Down/Page Up** seek 30 s. **Home** or **0** returns to the start; **End** or
**G** jumps to the last frame, since seeking to the very end would stop
playback. **+** / **=** and **-**, including the keypad's, change the volume by
0.1; reaching zero mutes and keeps the volume to restore. Unlike **Left/Right**,
the paging, jump and volume keys have no **Ctrl+Alt** form. The volume slider
steps 0.1 with the arrow keys and 0.2 with Page Up/Down; **Home** mutes and
**End** sets full volume. Other playable files show a seek slider instead of a
timeline: the arrow keys step 5 s, Page Up/Down 30 s, and Home/End jump to the
start and the last frame. Keys and the mouse wheel seek at once; a drag seeks
when released. In
[10xer mode](10xer-mode.md#preview-keyboard-ownership), the same keys, plus
**Up/Down** for volume, work whenever the media preview owns the keys.

## Shortcut footer

Every mode has a compact footer with **F1 · Shortcuts** on the left and clipboard status and the item count on the right. **Settings → General → Browsing → Show F1 Shortcuts button** controls the button's visibility (on by default). The preference is saved and updates all open windows immediately. Item counts and clipboard status remain visible when the button is hidden. F1 always opens the complete, mode-specific reference; closing it restores the button's configured visibility. F1, Escape, or a click outside it closes the reference, which blocks file operations and commands such as **F5**, **Ctrl+K** and **Ctrl+,** while open. Its search field still takes text-editing and input method keys, such as **Ctrl+Left** and **Ctrl+Space**. It stays open when the window loses focus. The reference is the only in-app keybinding list; it shows only the currently active map and live-updates when 10xer mode changes.

## 10xer mode

**Settings → General → Browsing → 10xer mode** (off by default,
toggle with **Ctrl+Shift+M**) hides window Search and
pane Close/filter/refresh/sort chrome and installs Yazi-style keys.
**Ctrl+Shift+M** is the only key that leaves the mode. While the mode is on, the footer shows
**10X** at the right, immediately before the item count.
Typed input uses the footer prompt, never the pane filter revealer or the
global search dialog. See [10xer mode](10xer-mode.md) for the keymap.

**Ctrl+Shift+M** also works while a browser text field has focus. Modal dialogs
retain their own input handling. Leaving 10xer mode clears retained footer
filters/search and forced recursion in all windows, so default **Ctrl+F** obeys
**Include subfolders** again. It ends visual/preview keyboard ownership without
clearing the ordinary listing's fill or closing the preview. **Esc** dismisses
one interaction at a time, including an open preview; recursive results have a
separate order described in [Escape precedence](10xer-mode.md#escape-precedence).

The F1 / `~` reference panel lists the active map and live-updates when the mode
changes. Its navigation section is specific to the current view, and in the
portal file chooser it lists only the keys that request allows.
Context-menu hints use `x`, `y`, `p`, `d` / `D`, `r`, `i`, and `M` / `C` / `R`
(Move to, Copy to, Restore) only when those commands perform the action. `i` toggles a file's preview without taking focus, or is the next column or folder peek for a directory.
Until those verbs run, the menu keeps the shortcuts that still work and hides
the unbound defaults (`y` for copy path, `Space` for preview, and `Ctrl+R` for
rename). Planned commands are not shown as working. In the default map, the
`y` (copy path) and `p` (pin) hints show only while **Type to search** is off,
because type-to-search claims those letters otherwise.

While a pane shows its search-results page (including filtered results), the footer
shows the displayed result total in both default and 10xer mode, including
**0 items** on a miss.
Selecting results does not replace that total with the hidden directory's selection;
its accessible description gives the result file/folder breakdown. Dismissing results restores the
ordinary directory count or selection summary. Until the search for a typed query
reports back, the footer keeps describing the directory. The `filter:` / `search:`
mark and the hit path at the footer's left end show only in 10xer mode, which
hides the pane's filter input; the default map shows the total alone.

With no selection in the ordinary listing, the footer shows the directory's item count. Selections show a folder/file breakdown, such as **1 folder, 2 files selected (64 MB)**. Sizes sum available metadata for selected files only; folder contents are not scanned or included. Missing file sizes are marked incomplete or unavailable.

After copying or cutting files, a highlighted **Files on clipboard** pill appears to the left of the item count. It reflects the file clipboard, including compatible copies from other applications, rather than assuming every clipboard contains files. It stays available after copying/pasting, and disappears when a completed cut consumes the clipboard or it is cleared/replaced with text. The clipboard status and paste shortcut remain available when the F1 Shortcuts button is hidden. Copied listing items show the Lucide copy icon in place of their thumbnail; cut items keep scissors, which wins if both would apply. Clearing the clipboard or unyank removes the copy overlay.

The hints describe file-view controls; text fields, dialogs, and media previews retain their own keyboard behavior. Mode changes update both the footer and the reference immediately. Closing keyboard-opened help restores the previous focus.

## Arrows, the header, and the sidebar

In Icons and List, plain arrows move interface focus rather than changing directories:

| Key | Icons | List |
| --- | --- | --- |
| Left | Move one tile left; at the left edge, focus the visible sidebar | Focus the visible sidebar |
| Right | Move one tile right | Stay in the file list |
| Up / Down | Move by visual rows | Move through file rows |
| Enter | Open the current item | Open the current item |

Enter on a previewed video, from the listing or inside the preview, pauses the
preview and asks the default player to start where it stopped. mpv, VLC,
Celluloid and MPlayer take the position on their command line; other players
open the file from the beginning, as does a position inside the first or last
second.

Up from the first Icons row or first List item focuses the navigation header, including in empty directories, where the pane itself holds focus. Left/Right traverse its enabled controls without triggering navigation; Enter/Space activates a control. Down returns to the item you left without changing selection. Left from the header's first control can reach the visible sidebar.

From the sidebar, Right returns to the item you left (or the current file view if navigation replaced it, or if you entered the sidebar from the header rather than from a file). Up/Down move between places. Up from Home, the first sidebar row, continues into the **top navigation bar** instead of stopping. Left/Right traverse its enabled controls without activating them; Down returns to the sidebar row you left. If the sidebar is hidden from the top bar, Down returns to the files instead. Empty file views also support these round trips. If the sidebar is hidden, Left in the file view does not change directories.

In the default map each file listing is one **Tab** stop; arrows move inside it. **Tab** from the listing goes to the next interface control (the open preview's controls, then **F1 Shortcuts**), and **Shift+Tab** goes back through the List sort headings, an open filter and the pane header actions, then the sidebar. **Tab** or **Shift+Tab** into a listing lands on the keyboard cursor, so **Enter**, **Space** and **Ctrl+C** act on the item that shows focus. In Columns the whole strip is one stop: **Tab** into it lands on the active column's cursor without changing the active column, **Tab** from any column leaves the strip, and **Shift+Tab** reaches the active column's open filter, then its header actions, then the control before the strip.

An empty, unreadable or still-loading directory has no rows to focus, so the pane itself takes focus. It draws the accent focus ring, is named after the directory, and is described by what it shows ("This directory is empty", the error, or "Loading"). **Tab** and **Shift+Tab** leave it as they leave a listing. In Columns, an unreadable directory's **Retry** button is the next **Tab** stop inside it. When the entries appear, focus moves back to the keyboard cursor.

**Settings → General → Browsing → Keep arrows in file list** (off by default) stops arrow keys from leaving the file list. Use **Ctrl+Shift+B** to focus the sidebar, or use the mouse. **Ctrl+\\** toggles it live. The file chooser respects the same preference.

**Alt+Left / Alt+Right / Alt+Up** remain Back / Forward / Parent in every mode. In the default map, List/Columns retain Miller-column navigation: **Right enters folders or moves into an existing pane to the right**. On a focused file with no pane to the right, Right does nothing; it never opens or previews the file. **Enter** opens files. With **Type to search** off, `h` / `j` / `k` / `l` act as the arrow keys in every view, and Backspace still goes up a level. In [10xer mode](10xer-mode.md), arrows stay in the Columns, List, and Icons panes. **Tab** moves from the file list to the window header, where **Enter** / **Space** activate the focused control and **h** / **j** return to the files. **Ctrl+Shift+B** focuses a visible sidebar; a hidden sidebar stays hidden until the header toggle shows it. In the sidebar, **j** / **k** and **Up** / **Down** move between places and device controls, **l** / **Enter** / **Space** activate the focused one, and **h** / **Left** / **Backspace** return to the files without changing the selection. A **Tab** or arrow key from the footer still returns to the file list. List and Columns **l** / **→** open a directory or enter a file's preview when possible. Icons **h** / **j** / **k** / **l** and arrows always move to the next icon in that direction, including across search-result icons; they never preview or change location. On a file, **i** toggles the preview without moving focus. On a directory it opens the next Miller column without focusing it, or toggles the folder-peek popover in List and Icons.

Columns reserves preview space from startup, even before a file is previewed. **Space**, **i**, and the preview's close button dismiss the content without reclaiming that space, so opening or closing a preview does not move the columns under the pointer. Switch **Appearance → Preview panel** off to explicitly reclaim it. This reservation is window-local and does not enable automatic previews on its own. In narrow windows the preview content hides, but the reserved slot keeps the remaining width beside the focused column, down to zero. The complete sizing, reservation, and dismissal rules are in [Preview panel and column layout](preview-panel-layout.md).

In Columns, the pane to the right mirrors keyboard selection like Finder: moving with **Up/Down**, **Page Up/Page Down**, **Home/End** or **Ctrl+Up/Ctrl+Down** onto a folder shows its contents in a child column that takes the reserved preview space, onto a previewable file closes that child column and opens Quick Preview in the same space, and onto any other file closes the child pane. The focused column stays where it is throughout. Shift-extended ranges do not mirror. A preview closed with **Space**, **i**, **Esc**, the close button, or **Appearance → Preview panel** stays closed while mirroring until it is opened explicitly again. Pointer selection keeps the configured click behavior. Clicking a folder whose column is already open focuses that column rather than closing it, and a double-click ends exactly where a single-click open does; a slow click on a selected folder's name still renames it. In every view, a plain click on an item that is part of a multi-selection collapses the selection to that item and then applies its single-click behavior: a folder set to open on one click opens (in Columns, as its child column), and a previewable file is previewed when **Single-click previews** is on. Pressing a member and dragging still drags the whole selection. **Settings → General → Browsing → Mirror columns selection** (on by default) toggles the mirroring. 10xer mode follows the same preference for its cursor in Columns; see [10xer mode](10xer-mode.md). **l** / **→** enters a directory or a file preview, and **i** toggles a file's preview or opens the next column / toggles folder peek for a directory.

Switching views with **Ctrl+1**, **Ctrl+2**, or **Ctrl+3** keeps keyboard focus in the visible tab. Hidden tabs follow the saved view preference without taking focus. A newer keyboard navigation action supersedes any queued cursor-focus restoration.

During a List heading drag, **Escape** cancels the reorder without clearing the file selection or changing the sort. Releasing the mouse afterward does not save a new column order. The file chooser uses the same behavior: Escape cancels the heading drag, not the chooser.

## Closing dialogs and overlays

Closing an overlay or dialog by any route (Escape, its close or Cancel button, or a
click outside it) returns keyboard focus to the control that opened it. This covers
Ctrl+K search, Ctrl+Shift+K folder jump, Compress, archive conflicts, Customize, and
error dialogs. If that control is gone, for example after the view was rebuilt or an
inline editor closed, focus goes to the file list cursor, or to the pane of an empty or
loading folder. File-operation progress always returns focus to the file list cursor,
because the operation changes the listing. Choosing a search result hands focus to the
browser instead, and a result the browser selects while a dialog closes, such as an
extracted folder, keeps focus.

Closing Settings always returns focus to the file list cursor, even when Settings was
opened with the gear button. The one exception is a filter field or filter result that
had focus when Settings opened, for example with **Ctrl+,**: it gets focus back.
**Ctrl+,** does not open Settings while an action dialog such as Properties is open.
A docked operation may finish and update its file selection behind a dialog, but it
must not take keyboard focus from that dialog.

Customize opens with focus on **Done**, so one **Escape** closes it. Closing its
custom color dialog returns focus to the custom color button.

While a dialog or Settings is open, **Ctrl+K**, **Ctrl+Shift+K**, **F5**,
**Ctrl+Alt+T** and **Ctrl+\\** do nothing and the dialog keeps focus. **Ctrl+K** and
**Ctrl+Shift+K** still close the search palette they opened.

## Opening and navigating the context menu

**Menu** (the hardware context-menu key) and **Shift+F10** open the selection-aware
context menu without the pointer. With an item keyboard-focused, the menu opens for
that item — or the full multi-selection, if the focused item is part of one. With no
selection, it opens the active pane's background menu. The menu is anchored to the
focused item or pane, never to the pointer.

Right-clicking an item also makes it the keyboard cursor, without opening it.
An already-selected item keeps the existing multi-selection; an unselected item
becomes the only selected item. Escape returns keyboard focus to that clicked item.

Once open: **Up/Down** move between enabled actions, wrapping past the first/last;
**Home/End** jump to the first/last enabled action; separators and disabled actions
are skipped. **Enter/Space** activates the focused action immediately on key press.
**Escape** closes the menu without changing the selection and returns keyboard
focus to the item or pane that opened it. This applies in Columns, Icons, List,
Trash, and the file chooser. Closing Properties follows the
[overlay rule](#closing-dialogs-and-overlays); choosing Rename hands focus to the
editor instead.

## Review fixture

Create `Fonts/` (empty), `Scripts/example.txt`, and `LICENSE` under a temporary directory.

- Select LICENSE with the pointer, copy, leave the pointer there, then navigate to Fonts with the keyboard and paste. LICENSE should appear only in Fonts.
- Select a file in Scripts, copy, and move the pointer onto blank space in the parent column. The parent header must gain the destination accent before Ctrl+V.
- Focus the parent, select several items, then click blank child and parent content. The open child and parent selection must remain intact. Ctrl+A must affect the parent only.
- Enter an empty directory and try Delete/Shift+Delete. No confirmation targeting its parent should appear.
- Repeat with a light theme, with filters, and with enough files to scroll. The cursor must remain distinguishable from selection and path markers.

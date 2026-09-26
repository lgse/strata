# Column focus and command destinations

Columns have three independent signals:

- **Selection:** filled rows are the items selected in that directory. Other columns retain a quieter selection when you leave them.
- **Keyboard cursor:** a text-contrast outline identifies the current item in the keyboard-focused list. Only that list shows a cursor; range selections can contain several filled rows.
- **Open path:** the chevron identifies the folder whose child column is open, without an extra border. This is navigation context, not another keyboard cursor.

The destination column has an accent rule across its header, including when the directory is empty. Columns no longer reserve a separate bottom margin for the horizontal scrollbar.

## Input precedence

The last navigation input determines the destination of Ctrl+V:

1. Moving, clicking, or scrolling the pointer restores pointer control. Paste targets the directory column under it, not an individual hovered file or folder.
2. Keyboard navigation (arrows, h/j/k/l, Tab, page movement, entering/leaving folders) or Select All restores keyboard control. Paste targets the focused column, falling back to the active column when browser widgets do not hold focus.
3. Ctrl+V itself does **not** change ownership. A parked pointer cannot override subsequent keyboard navigation. Layout/scroll changes underneath an unmoving pointer do not count as pointer motion.
4. Outside the columns, pointer mode falls back to the focused/active directory. Stale depths are discarded. Icons and List continue to use their single active directory.

The terminal shortcut uses the same directory fallback, but still prefers an explicitly selected directory. New Folder remains keyboard-focus scoped. Context-menu paste and drag-and-drop retain their explicit destinations.

Keyboard navigation suppresses stale row-hover effects and pending folder peeks until deliberate pointer input resumes. It does not erase selection or the open folder path.

## Focus without selection changes

Pressing blank column content focuses that directory, including empty directories, without closing descendants. Releasing a plain click on empty space clears file selections across columns. When returning to an inactive column, it also selects that column's first visible entry as the range anchor. Holding or dragging does not clear selections before marquee intent is resolved. Row clicks, controls, scrollbars, context menus, and marquee selection keep their own interactions. Returning to a column by keyboard preserves a multi-selection; Ctrl+A selects the focused column, not the deepest open column.

Copy/cut use the selection in the focused column, never a hovered row. In Columns, Delete/Shift+Delete with no selected items does nothing: an open parent-path marker is not an implicit deletion target. The separate List/Icons parent-deletion fallback is tracked in #300.

Background selection updates from directory loading must not move keyboard focus to an inactive column.

## Returning to an Icons or List directory

Icons and List remember the selection, keyboard cursor, and scroll position of the
last 128 directories left in that browser. Back, Forward, and Up restore each
visited directory after its entries load, including nested parents. Arrow-key
navigation continues from the restored row. Entries are matched by location,
not their previous row numbers; deleted entries are not selected accidentally.
This is temporary browsing state, not a saved preference. New input in the file view
cancels an in-progress restoration.

## Creating files and folders

In Columns, List, and Icons, **Ctrl+Shift+N** or background menu → **New Folder**
immediately creates `new folder`. Background menu → **New File** immediately
creates an empty `new file`. If the default name is occupied by any item, creation
tries `new folder (1)` / `new file (1)`, then `(2)`, and so on without overwriting
anything. The pane filter is cleared and the entire allocated default name is
selected: one Backspace clears it, and typing replaces it.

For **any file or folder rename**, Enter, clicking outside the field (even empty
pane space), or moving keyboard focus away commits a valid name. Escape keeps
the original name. Finishing with an empty or invalid name also keeps the
original. Cancelling the initial rename does **not** delete the new item: it
remains under its allocated default name. File contents are preserved.

Clicking inside the field continues editing. Existing files retain extension-aware
selection (the stem is selected); folder names containing dots are selected in full.

Names containing `/` or NUL, `.`/`..`, and whitespace-only names (including
Unicode whitespace) are invalid. Valid names are used exactly as typed,
including spaces around a nonblank name, hidden-file prefixes, and Unicode.
Name conflicts, filesystem-specific limits, and permission errors retain the
original item and report an error.

## Filename patterns while filtering

Use **Ctrl+F** to filter a pane. Plain text matches a substring of the filename,
ignoring case. Queries of at least four letters or digits also allow one inserted,
missing, substituted, or adjacent swapped character within a whole filename word.
Words are separated by punctuation or spaces. For example, `trahs` finds
`strata-trash.svg`, but `trash` does not find `strata-search.svg`. Shorter queries
and queries containing punctuation remain literal. Indexed results rank literal
matches ahead of typo matches; parent paths do not qualify a result.

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
Clear the input or press Escape to restore the directory listing. **Ctrl+K**
global fuzzy search is unchanged.

## Preview while filtering

In the browser and file chooser, **Down** from the Ctrl+F input focuses the selected result, or the first result if none is selected. **Up/Down** then navigate the results; **Up** from the first result returns to the input without clearing the query. **Ctrl+F** also returns to the input. With no matches, Down leaves focus in the input.

**Menu/Shift+F10** on a focused result opens its file menu. While the input itself is focused, its text-editing menu remains available. **Space** toggles quick preview for a selected file result in Columns, Icons, and List, including after returning to the query. Previewing a file keeps the query, selection, and current directory intact; on a selected folder result, Space navigates into the folder instead.

While the input is focused, Space types into the query if no result is selected. **Shift+Space** inserts a space there even with a result selected. Space opens a selected folder in every view without opening or loading the preview pane; unsupported files do not open a preview.

## Navigating an archive preview

Quick Look on a local ZIP, 7z, TAR, or TAR.GZ opens the archive's member tree
instead of extracting it. The preview starts at the archive root with its first
member highlighted. The listing keeps its selection, but drops the
keyboard-cursor outline so only one cursor is visible.

Inside the preview, **Up/Down** (or **k/j**) move the highlight, **Right/l/Enter**
opens the highlighted folder, and **Left/h** returns to the parent. Left at the
archive root and Right/Enter on a member file do nothing. Navigating never
extracts anything or touches the filesystem; **Space** and **Escape** still
close the preview.

## Shortcut footer

Every mode has a compact footer with **F1 · Shortcuts** on the left and clipboard status and the item count on the right. **Settings → Keybindings → Show F1 Shortcuts button** controls the button's visibility (on by default). The preference is saved and updates all open windows immediately. Item counts and clipboard status remain visible when the button is hidden. F1 always opens the complete, mode-specific reference; closing it restores the button's configured visibility. F1 or Escape closes the reference, which blocks file-operation shortcuts while open. **Settings → Keybindings** lists only the currently active map and live-updates when 10xer mode changes.

## 10xer mode

**Settings → General → Browsing → 10xer mode** (off by default,
toggle with **Ctrl+Shift+M**, leave with **q**) hides window Search and
pane Close/filter/refresh/sort chrome and installs Yazi-style keys. **q** leaves
the mode and does not close the window. While the mode is on, the footer shows
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

The F1 / `~` popover and **Settings → Keybindings** share the active map's
presentation data and live-update when the mode changes. Default F1 navigation
is specific to the current view; Settings includes the all-view overview.
Context-menu hints use `x`, `y`, `p`, `d` / `D`, `r`, and `i` only when those
commands perform the action. `i` is the next column or folder peek, not preview.
Until those verbs run, the menu keeps the shortcuts that still work and hides
the unbound defaults (`Y` for copy path, `Space` for preview, and `Ctrl+R` for
rename). Planned commands are not shown as working.

While a pane shows its search-results page (including filtered results), the footer
shows the displayed result total in both default and 10xer mode, including
**0 items** on a miss.
Selecting results does not replace that total with the hidden directory's selection;
its tooltip gives the result file/folder breakdown. Dismissing results restores the
ordinary directory count or selection summary.

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

Up from the first Icons row or first List item focuses the navigation header, including in empty directories. Left/Right traverse its enabled controls without triggering navigation; Enter/Space activates a control. Down returns to the item you left without changing selection. Left from the header's first control can reach the visible sidebar.

From the sidebar, Right returns to the item you left (or the current file view if navigation replaced it, or if you entered the sidebar from the header rather than from a file). Up/Down move between places. Up from Home, the first sidebar row, continues into the **top navigation bar** instead of stopping. Left/Right traverse its enabled controls without activating them; Down returns to the sidebar row you left. If the sidebar is hidden from the top bar, Down returns to the files instead. Empty file views also support these round trips. If the sidebar is hidden, Left in the file view does not change directories.

**Settings → General → Browsing → Keep arrows in file list** (off by default) stops arrow keys from leaving the file list. Use **Ctrl+Shift+B** to focus the sidebar, or use the mouse. **Ctrl+\\** toggles it live. The file chooser respects the same preference.

**Alt+Left / Alt+Right / Alt+Up** remain Back / Forward / Parent in every mode. In the default map, List/Columns retain Miller-column navigation: **Right enters folders or moves into an existing pane to the right**. On a focused file with no pane to the right, Right does nothing; it never opens or previews the file. **Enter** opens files; with **Type to search** off, `l` still activates. Backspace and the existing `h` / `l` directory shortcuts remain available. In [10xer mode](10xer-mode.md), arrows and Tab stay in the Columns, List, and Icons panes. The sidebar, window header, footer, and other controls outside those panes stay pointer-operated, and a Tab or arrow key while one of them has focus returns to the file list. **Ctrl+Shift+B** stays with the file list while the mode is on. List and Columns **l** / **→** open a directory or enter a file's preview when possible. Icons **h** / **j** / **k** / **l** and arrows always move to the next icon in that direction, including across search-result icons; they never preview or change location. **i** opens the next Miller column without focusing it, or toggles the folder-peek popover in List and Icons. It does not preview a file.

In Columns, the pane to the right mirrors keyboard selection like Finder: **Up/Down** onto a folder shows its contents without moving focus, onto a previewable file opens Quick Preview, and onto any other file closes the child pane. Pointer selection keeps the configured click behavior. **Settings → General → Browsing → Mirror columns selection** (on by default) toggles the mirroring. 10xer mode leaves that preference saved and does not mirror: cursor movement does not open a child column or a preview. **l** / **→** enters a directory or a file preview, and **i** opens the next column or toggles folder peek. The saved value applies again after leaving the mode.

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
Trash, and the file chooser. Closing Properties returns focus to its originating
control; choosing Rename hands focus to the editor instead.

## Review fixture

Create `Fonts/` (empty), `Scripts/example.txt`, and `LICENSE` under a temporary directory.

- Select LICENSE with the pointer, copy, leave the pointer there, then navigate to Fonts with the keyboard and paste. LICENSE should appear only in Fonts.
- Select a file in Scripts, copy, and move the pointer onto blank space in the parent column. The parent header must gain the destination accent before Ctrl+V.
- Focus the parent, select several items, then click blank child and parent content. The open child and parent selection must remain intact. Ctrl+A must affect the parent only.
- Enter an empty directory and try Delete/Shift+Delete. No confirmation targeting its parent should appear.
- Repeat with a light theme, with filters, and with enough files to scroll. The cursor must remain distinguishable from selection and path markers.

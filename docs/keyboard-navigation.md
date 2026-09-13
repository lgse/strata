# Column focus and command destinations

Columns have three independent signals:

- **Selection:** filled rows are the items selected in that directory. Other columns retain a quieter selection when you leave them.
- **Keyboard cursor:** a text-contrast outline identifies the current item in the keyboard-focused list. Only that list shows a cursor; range selections can contain several filled rows.
- **Open path:** the chevron identifies the folder whose child column is open, without an extra border. This is navigation context, not another keyboard cursor.

The destination column has an accent rule across its header and a **Keyboard · Paste here** or **Pointer · Paste here** footer. The indication remains useful in an empty directory, where there is no row to highlight. When panes overflow, the horizontal scrollbar gets its own track below these labels rather than covering them. No track is reserved when the panes fit.

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

## Returning to a List directory

List mode remembers the selection, keyboard cursor, and scroll position of the
last 128 directories left in that browser. Back, Forward, and Up restore each
visited directory after its entries load, including nested parents. Arrow-key
navigation continues from the restored row. Entries are matched by location,
not their previous row numbers; deleted entries are not selected accidentally.
This is temporary browsing state, not a saved preference. New input in the list
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

## Preview while filtering

In the browser and file chooser, **Down** from the Ctrl+F input focuses the selected result, or the first result if none is selected. **Up/Down** then navigate the results; **Up** from the first result returns to the input without clearing the query. **Ctrl+F** also returns to the input. With no matches, Down leaves focus in the input.

**Menu/Shift+F10** on a focused result opens its file menu. While the input itself is focused, its text-editing menu remains available. **Space** toggles quick preview for the selected result in Columns, Icons, and List, including after returning to the query. The query, selection, and current directory stay intact.

While the input is focused, Space types into the query if no result is selected. **Shift+Space** inserts a space there even with a result selected. Folders and unsupported files do not open a preview.

## Shortcut footer

Every mode has a compact, single-line footer with its navigation hints and common file shortcuts. **Settings → Keybindings → Show keybinding hints** controls its visibility (on by default). The preference is saved and updates all open windows immediately. F1 still opens the reference with hints disabled; closing it hides the footer again. The summary truncates rather than wrapping in narrow windows; **F1 · Shortcuts** always remains available to open the complete, mode-specific reference. F1 or Escape closes it. The reference blocks file-operation shortcuts while it is open.

After copying or cutting files, a highlighted **Ctrl+V · Paste available** hint appears beside the reference button. It reflects the file clipboard, including compatible copies from other applications, rather than assuming every clipboard contains files. It stays available after copying/pasting, and disappears when a completed cut consumes the clipboard or it is cleared/replaced with text. With hints disabled, the paste shortcut still works, but the footer stays hidden.

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

From the sidebar, Right returns to the item you left (or the current file view if navigation replaced it). Up/Down move between places. Up from Home, the first sidebar row, continues into the **top navigation bar** instead of stopping. Left/Right traverse its enabled controls without activating them; Down returns to the sidebar row you left. If the sidebar is hidden from the top bar, Down returns to the files instead. Empty file views also support these round trips. If the sidebar is hidden, Left in the file view does not change directories.

**Settings → Keybindings → Keep arrows in file list** (off by default) stops arrow keys from leaving the file list. Use Tab or the mouse to reach the toolbar and sidebar. **Ctrl+\\** toggles it live. The file chooser respects the same preference.

**Alt+Left / Alt+Right / Alt+Up** remain Back / Forward / Parent in every mode. List/Columns retain Miller-column navigation: **Right enters folders or moves into an existing pane to the right**. On a focused file with no pane to the right, Right does nothing; it never opens or previews the file. **Enter** opens files; the existing `l` activation shortcut is unchanged. Backspace and the existing `h` / `l` directory shortcuts remain available.

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
- Select a file in Scripts, copy, and move the pointer onto blank space in the parent column. The parent must visibly become the paste destination before Ctrl+V.
- Focus the parent, select several items, then click blank child and parent content. The open child and parent selection must remain intact. Ctrl+A must affect the parent only.
- Enter an empty directory and try Delete/Shift+Delete. No confirmation targeting its parent should appear.
- Repeat with a light theme, with filters, and with enough files to scroll. The cursor must remain distinguishable from selection and path markers.

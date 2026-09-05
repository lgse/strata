# Column focus and command destinations

Columns have three independent signals:

- **Selection:** filled rows are the items selected in that directory. Other columns retain a quieter selection when you leave them.
- **Keyboard cursor:** a text-contrast outline identifies the current item in the keyboard-focused list. Only that list shows a cursor; range selections can contain several filled rows.
- **Open path:** the chevron identifies the folder whose child column is open, without an extra border. This is navigation context, not another keyboard cursor.

The destination column has an accent rule across its header and a **Keyboard · Paste here** or **Pointer · Paste here** footer. The indication remains useful in an empty directory, where there is no row to highlight.

## Input precedence

The last navigation input determines the destination of Ctrl+V:

1. Moving, clicking, or scrolling the pointer restores pointer control. Paste targets the directory column under it, not an individual hovered file or folder.
2. Keyboard navigation (arrows, h/j/k/l, Tab, page movement, entering/leaving folders) or Select All restores keyboard control. Paste targets the focused column, falling back to the active column when browser widgets do not hold focus.
3. Ctrl+V itself does **not** change ownership. A parked pointer cannot override subsequent keyboard navigation. Layout/scroll changes underneath an unmoving pointer do not count as pointer motion.
4. Outside the columns, pointer mode falls back to the focused/active directory. Stale depths are discarded. Grid and Explorer continue to use their single active directory.

The terminal shortcut uses the same directory fallback, but still prefers an explicitly selected directory. New Folder remains keyboard-focus scoped. Context-menu paste and drag-and-drop retain their explicit destinations.

Keyboard navigation suppresses stale row-hover effects and pending folder peeks until deliberate pointer input resumes. It does not erase selection or the open folder path.

## Focus without selection changes

Clicking blank column content focuses that directory, including empty directories, without clearing its selection or closing descendants. Row clicks, controls, scrollbars, context menus, and marquee selection keep their own interactions. Returning to a column preserves a multi-selection; Ctrl+A selects the focused column, not the deepest open column.

Copy/cut use the selection in the focused column, never a hovered row. In Columns, Delete/Shift+Delete with no selected items does nothing: an open parent-path marker is not an implicit deletion target. The separate Explorer/Grid parent-deletion fallback is tracked in #300.

Background selection updates from directory loading must not move keyboard focus to an inactive column.

## Shortcut footer

Every mode has a compact, single-line footer with its navigation hints and common file shortcuts. **Settings → Keybindings → Show keybinding hints** controls its visibility (on by default). The preference is saved and updates all open windows immediately. F1 still opens the reference with hints disabled; closing it hides the footer again. The summary truncates rather than wrapping in narrow windows; **F1 · Shortcuts** always remains available to open the complete, mode-specific reference. F1 or Escape closes it. The reference blocks file-operation shortcuts while it is open.

After copying or cutting files, a highlighted **Ctrl+V · Paste available** hint appears beside the reference button. It reflects the file clipboard, including compatible copies from other applications, rather than assuming every clipboard contains files. It stays available after copying/pasting, and disappears when a completed cut consumes the clipboard or it is cleared/replaced with text. With hints disabled, the paste shortcut still works, but the footer stays hidden.

[Keybindings setting](screenshots/291/hints-settings.png) · [Paste available](screenshots/291/paste-available.png) · [Hints disabled](screenshots/291/hints-hidden.png)

The hints describe file-view controls; text fields, dialogs, and media previews retain their own keyboard behavior. Mode changes update both the footer and the reference immediately. Closing keyboard-opened help restores the previous focus.

[Columns footer](screenshots/291/footer-columns.png) · [Grid header focus](screenshots/291/footer-grid-header.png) · [Explorer header, light theme](screenshots/291/footer-explorer-header.png) · [Narrow window](screenshots/291/footer-narrow.png) · [Shortcut reference](screenshots/291/shortcut-reference.png)

## Arrows, the header, and the sidebar

In Grid and Explorer, plain arrows move interface focus rather than changing directories:

| Key | Grid | Explorer |
| --- | --- | --- |
| Left | Move one tile left; at the left edge, focus the visible sidebar | Focus the visible sidebar |
| Right | Move one tile right | Stay in the file list |
| Up / Down | Move by visual rows | Move through file rows |
| Enter | Open the current item | Open the current item |

Up from the first Grid row or first Explorer item focuses the navigation header, including in empty directories. Left/Right traverse its enabled controls without triggering navigation; Enter/Space activates a control. Down returns to the item you left without changing selection. Left from the header's first control can reach the visible sidebar.

From the sidebar, Right returns to the item you left (or the current file view if navigation replaced it). Up/Down move between places. Empty file views also support this round trip. If the sidebar is hidden, Left does not change directories.

**Alt+Left / Alt+Right / Alt+Up** remain Back / Forward / Parent in every mode. Columns retain Miller-column arrow navigation. Backspace and the existing Vim directory shortcuts remain available.

## Review fixture

Create `Fonts/` (empty), `Scripts/example.txt`, and `LICENSE` under a temporary directory.

- Select LICENSE with the pointer, copy, leave the pointer there, then navigate to Fonts with the keyboard and paste. LICENSE should appear only in Fonts.
- Select a file in Scripts, copy, and move the pointer onto blank space in the parent column. The parent must visibly become the paste destination before Ctrl+V.
- Focus the parent, select several items, then click blank child and parent content. The open child and parent selection must remain intact. Ctrl+A must affect the parent only.
- Enter an empty directory and try Delete/Shift+Delete. No confirmation targeting its parent should appear.
- Repeat with a light theme, with filters, and with enough files to scroll. The cursor must remain distinguishable from selection and path markers.

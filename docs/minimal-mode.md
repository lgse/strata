# Minimal mode

Minimal mode is an opt-in Yazi-style browsing map. It hides window Search/Close
and pane filter/refresh/sort chrome in interactive browsers, disables
type-to-search, and uses the footer as the typed-command surface. The portal
file chooser keeps its own dispatcher and full chrome.

Turn it on in **Settings → General → Browsing → Minimal mode**, or with
**Ctrl+Shift+M**. The choice is saved and live-updates every window. **F1** or
**~** opens the in-app table; **Settings → Keybindings** lists the same map.

Paste destinations, cursor versus filled selection, and pointer ownership stay
as in [keyboard navigation](keyboard-navigation.md).

## Enter and leave

| Key | Action |
| --- | --- |
| **Ctrl+Shift+M** | Toggle minimal mode |
| **q** | Leave the mode. Flashes `Left minimal mode — Ctrl+Shift+M returns`. Does not close the window. |
| **Q** | Close the current window |
| **F1** / **~** | Show or hide this reference |
| **Ctrl+,** | Open Settings |

**Type to search** and **Keep arrows in file list** stay saved. While the mode
is on they are unused, and those Settings rows show **Not used in minimal mode.**

## Navigation

Arrows stay in the file list. **h** is parent, never the sidebar.

| Key | Action |
| --- | --- |
| **h** / **←** / **Backspace** / **Alt+↑** | Leave the preview, or parent folder, closing the Miller child pane |
| **l** / **→** | Open a directory, or enter a file's preview when possible |
| **Enter** | Open the focused item, including files |
| **j** / **k** / **↑** / **↓** | Next / previous item in listing order |
| **g g** / **Home** | First item |
| **G** / **End** | Last item |
| **Ctrl+U** / **Ctrl+D** | Half page up / down |
| **Ctrl+B** / **Ctrl+F** / **PgUp** / **PgDn** | Full page up / down |
| **H** / **L** / **Alt+←** / **Alt+→** | Back / forward in history |
| **i** | Toggle the preview drawer |
| **J** / **K** | Scroll the open preview |

**l** / **→** on a previewable file opens the preview if it is closed and moves
keys into that pane. A further **l** / **→** stays there and does not open the
file. Then **j** / **k** / arrows, **g g** / **Home**, **G** / **End**,
**Ctrl+U** / **D** / **B** / **F**, and **PgUp** / **PgDn** scroll the document
instead of the listing. **h** / **←** returns to the miller column or folder
without closing the preview or going to the parent; a following **h** still
goes to the parent. **i** still toggles the drawer. Unpreviewable files and
empty folders flash `Nothing to preview`.

**g g** is first item. **g h** is Home.

## Selection

**Space** toggles the keyboard cursor, not pointer hover, and does not preview
(**i** does).

| Key | Action |
| --- | --- |
| **Space** | Toggle the focused item and move down |
| **v** / **V** | Visual select / visual unset |
| **Ctrl+A** | Select all in the focused pane |
| **Ctrl+R** | Invert the selection |
| **Esc** | Cancel a chord, then dismiss a hidden filter, then dismiss find highlights, then leave visual (keep the fill), then clear the selection |

On a cursor-only row, **Space** adds that item and moves down; it does not
deselect it. After **Space**, **Ctrl+A**, **Ctrl+R**, or leaving visual,
**j** / **k** / **g g** / **G** / paging move the cursor without rewriting the
fill. **v** then motion starts a new range from the cursor. **V** subtracts the
walked span.

## Files

With an empty fill, **y** / **x** / **d** act on the focused item. Typical flow:
cursor or **v** then motion → **y** / **x** → **h** / **l** / **g h** / **g 1**
→ **p**.

| Key | Action |
| --- | --- |
| **y** / **x** | Yank / cut the selection (or the focused item) |
| **p** | Paste. Keep Both is focused on conflicts when that button is offered. |
| **P** | Paste. Replace is focused on conflicts. |
| **Y** / **X** | Clear cut marks and this process's clipboard payload. Does not wipe another application's clipboard. |
| **d** / **Delete** | Move to Trash with confirmation |
| **D** / **Shift+Delete** | Delete permanently with confirmation. Cancel is focused. |
| **r** / **F2** | Rename the focused item in the footer prompt |
| **a** | Create a file. A trailing `/` makes a folder (stripped before validation). Conflicts error instead of uniquifying. |
| **o** / **O** | Open / Open With |
| **c c** / **c n** | Copy path / name |
| **.** / **Ctrl+H** / **Ctrl+.** | Show or hide hidden files |
| **, a** / **, m** / **, s** / **, e** | Sort by name / modified / size / type. Shift reverses. |
| **Ctrl+Z** | Undo the last file operation |

Empty folder: **y** / **x** / **d** / **r** / **i** / **l** / **→** / **Space** flash
`Nothing to yank` / `cut` / `delete` / `rename` / `preview` / `select`. Empty
clipboard **p** flashes `Nothing to paste`.

## Places

Press **g**, then a second key. The chord stays armed while the footer **g-**
mark is showing. Sidebar keycaps appear on Home (**h**), Downloads (**d**),
Trash (**t**), Network (**n**), Recent (**r**), Documents (**k**), Pictures
(**p**), Videos (**v**), and visible PINNED rows (**1**–**9** in display
order), plus a short list of valid second keys. **,** and **c** show the same
kind of list (sort options / copy path or name) while armed. The second key
completes only that chord: **, a** / **, s** sort instead of create / search,
and search-result **j** / **h** cannot steal a pending **g** or **c**.

| Second key | Destination |
| --- | --- |
| **g** | First item |
| **h** | Home |
| **d** | Downloads. Missing: `No Downloads folder`. |
| **c** | Config (`~/.config`) |
| **t** | Trash |
| **n** | Network |
| **r** | Recent |
| **k** | Documents. Missing: `No Documents folder`. |
| **p** | Pictures. Missing: `No Pictures folder`. |
| **v** | Videos. Missing: `No Videos folder`. |
| **1**–**9** | Visible PINNED rows in sidebar order. Missing: `No pin N`. |
| **Space** | Footer `go ›` — type a path or URI. **Tab** / **Shift+Tab** cycle matching folders. |
| **Esc** | Cancel |

An unknown second key cancels with `Unknown chord`. In **go ›**, **Tab** /
**Shift+Tab** cycle matching folders for the typed prefix: the current listing
when the text has no slash, the parent after a slash, and `~` as home. No match
keeps the typed text and stays in the prompt. The entry is cleared on submit
and Escape so credentials do not linger.

## Prompts

Typed commands use the footer, never the pane filter revealer or the global
search dialog. **Enter** submits, **Esc** cancels. **Up** / **Down** move the
listing or pick a history candidate without leaving the prompt. Clicking a
listing row closes the prompt and keeps that selection.

| Key | Action |
| --- | --- |
| **/** / **?** | Find next / previous name in this listing. Does not hide rows. Enter keeps matching substring highlights; **Esc** from the listing dismisses them. |
| **n** / **N** | Repeat the last find. **N** reverses. |
| **f** | Filter this listing (hides non-matches). The footer shows `filter: …` until dismissed. |
| **s** | Recursive name search in the current folder (cap 100). **Esc** keeps hits; **Esc** again dismisses. |
| **z** | Jump to a visited folder (fuzzy + frecency) |
| **Z** | Jump to a recent folder (last visit) |
| **a** | Create |
| **r** | Rename |
| **Tab** / **Shift+Tab** | Cycle matching folders in the go prompt | |

**/** is a cursor jump; **f** hides non-matches. The prompt stays focused while
you type. Matching names stay highlighted after Enter in miller columns and
list. **Esc** from the listing dismisses those highlights without hiding rows.
Empty **/** with Enter does nothing. **Esc** from the prompt cancels without
leaving highlights. Pressing **/** again shows the prompt. **n** repeats the
last query, including multi-character input.

Enter on **f** keeps the filter. Opening **f** again pre-fills the current
query. Empty Enter, **Esc** from the prompt, or **Esc** from the list clears
it. View rebuilds keep the funnel collapsed.

**s** searches the current folder tree only, not every indexed root. **Enter**
focuses the first hit. **Esc** from the prompt keeps the hits so **v** / **V** /
Space / **i** / **→** can use them; a second **Esc** (or **Esc** after Enter)
dismisses back to the directory. **h** leaves search in one step. Empty **Esc**
cancels with no results. The footer count is the hit count, not the hidden
directory's fill.
**z** / **Z** use Strata folder history, not a zoxide database. Empty input
still lists candidates; **Up** / **Down** pick a row, **Enter** goes there.

## Search results

While **s** results are showing:

| Key | Action |
| --- | --- |
| **j** / **k** | Move the result list |
| **l** / **→** | Open a directory hit, or enter a file hit's preview when possible |
| **Enter** | Activate the focused hit |
| **i** | Preview the focused hit |
| **h** | Leave the preview, or dismiss search and restore the directory listing |

**Space** does not preview search rows.

## Sidebar

**Ctrl+Shift+B** focuses the visible sidebar. It does not show a hidden
sidebar; use the header toggle for that.

| Key | Action |
| --- | --- |
| **j** / **k** (or **↓** / **↑**) | Move between places |
| **l** / **Enter** / **Space** | Activate the focused place or device control |
| **h** / **←** / **Backspace** | Return to the file list |

Header buttons reached with Tab use **Enter** / **Space** to activate;
**h** / **j** return to the files.

## Still bound

These GUI conventions stay available alongside the Yazi verbs:

| Key | Action |
| --- | --- |
| **Ctrl+C** / **Ctrl+X** / **Ctrl+V** | Copy / cut / paste |
| **Delete** / **Shift+Delete** | Trash / permanent delete (same as **d** / **D**, with confirmation) |
| **F2** | Footer rename (same as **r**) |
| **F5** | Refresh |
| **Ctrl+L** | Edit the location bar |
| **Ctrl+K** | Global search |
| **Ctrl+1** / **2** / **3** | Columns / Icons / List |
| **Ctrl+Shift+N** | New folder |
| **Alt+Enter** | Properties |
| **Menu** / **Shift+F10** | Context menu |
| **Ctrl++** / **Ctrl+−** / **Ctrl+0** | Text size |

Context-menu shortcut hints follow this map (**x** cut, **y** yank, **p** paste,
**d** / **D** trash / delete, **r** rename, **i** preview). Default-map hints
that are unbound or remapped (**Y** for copy path, **Space** for preview,
**Ctrl+R** for rename) are hidden. Copy path is **c c**; Properties is
**Alt+Enter**.

## Not bound

These default-map shortcuts are unbound or remapped while the mode is on:

| Default-map key | In minimal mode |
| --- | --- |
| **Ctrl+Shift+K** | Unbound. Use **z** / **Z**. |
| **Ctrl+T** | Unbound. Use the context menu. |
| **Ctrl+\\** | Unbound. Arrows never leave the file list. |
| **Ctrl+D** | Half page down. Duplicate is dropped. |
| **Ctrl+F** | Full page down. Filter is **f**. |
| **Ctrl+B** | Full page up. Sidebar toggle is the header button. |
| **Ctrl+R** | Invert selection. Rename is **r** / **F2**. |
| **y** / **p** (default map) | Yank / paste. Path copy is **c c**; pin via **g** then a digit. |
| **Space** | Toggle selection. Preview is **i**. |

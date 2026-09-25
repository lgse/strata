# Omastrata mode

> This document is the target product specification, not a completion report.

Omastrata mode is an opt-in Yazi-style browsing map. In **Settings → General → Browsing** the row is titled **Omastrata mode** and its subtitle is **Opinionated keyboard-centric mode with Yazi-style navigation. Disables some features. Toggle with Ctrl-Shift-M.** The footer shows **OMA** at the right, immediately before the item count, while the mode is on. It hides window Search and pane
Close/filter/refresh/sort chrome in interactive browsers, disables
type-to-search, and uses the footer as the typed-command surface. List column
headings stay clickable. The portal file chooser follows the same preference:
it hides pane Close/filter/refresh/sort chrome, uses this keymap and the footer
prompt, and keeps Accept, Cancel, and the header close control. **Enter** / **o**
confirm a file. **Esc** still cancels the dialog after dismissing a prompt,
filter, or preview. **q** leaves the mode without cancelling. The chooser
continues to disallow folder peeking and column mirroring, so **i** does not
open a peek or an extra Miller column there.

Turn it on in **Settings → General → Browsing → Omastrata mode**, or with
**Ctrl+Shift+M**. The choice is saved and live-updates every window. **F1** or
**~** opens the in-app table; **Settings → Keybindings** lists the same map.

Paste destinations, cursor versus filled selection, and pointer ownership stay
as in [keyboard navigation](keyboard-navigation.md).

## Enter and leave

| Key | Action |
| --- | --- |
| **Ctrl+Shift+M** | Toggle Omastrata mode |
| **q** | Leave the mode. Does not close the window. |
| **Q** | Close the current window |
| **F1** / **~** | Show or hide this reference |
| **Ctrl+,** | Open Settings, canceling any armed chord |

**Ctrl+Shift+M** also works while a browser text field has focus; modal dialogs
keep their own input handling.

**Type to search**, **Keep arrows in file list**, and **Mirror columns selection**
stay saved. While the mode is on they are unused, and those Settings rows show
**Not used in Omastrata mode.**

Leaving the mode clears footer prompts (including typed credentials), chords,
find highlights, retained filters/search results, a keyboard folder peek, and
preview keyboard ownership in every open browser window. It leaves the ordinary
listing's filled selection, any Miller column already opened, and any open preview
intact. Saved column mirroring applies again. Default **Ctrl+F** again follows
the saved **Include subfolders** preference; a previous **s** search does not
force it to recurse.

## Navigation

Arrows stay in the file list. In List and Columns, **h** is parent, never the
sidebar. In Icons, **h** / **j** / **k** / **l** and arrows move among tiles.

| Key | Action |
| --- | --- |
| **h** / **←** | List and Columns: leave the preview, or parent folder, closing the Miller child pane |
| **l** / **→** | List and Columns: open a directory, or enter a file's preview when possible |
| **h** / **j** / **k** / **l** / arrows | Icons: move spatially among tiles. Do not enter a folder, leave the folder, or open preview. |
| **Enter** | Open the focused item, including files |
| **j** / **k** / **↑** / **↓** | List and Columns: next / previous item in listing order |
| **g g** / **Home** | First item |
| **G** / **End** | Last item |
| **Ctrl+U** / **Ctrl+D** | Half page up / down |
| **Ctrl+B** / **Ctrl+F** / **PgUp** / **PgDn** | Full page up / down |
| **H** / **L** / **Alt+←** / **Alt+→** | Back / forward in history |
| **Backspace** / **Alt+↑** | Parent folder |
| **i** | Columns: open the next Miller column for the focused directory without moving focus into it. List and Icons: toggle the folder-peek popover for the focused directory. A file is not previewed. |
| **J** / **K** | Scroll the open preview without taking focus |

Column selection mirroring stays off while the mode is on. **j** / **k** and
**↑** / **↓** only move the cursor. They do not open a child column or a preview.
**l** / **→** enters a directory or a file preview. In Columns, **i** on a directory opens
the next column and leaves focus where it is; moving the cursor does not refresh
that column. A second press does not move focus. In List and Icons, **i** toggles
the existing folder-peek popover for the focused directory. That popover is the
directory peek, not the saved Folder peeking switch and not a file preview.
Pressing **i** again, or **Esc**, closes the popover. **Esc** does not close a
Miller column. **i** on a file or in an empty folder opens neither a column nor
a peek.

In **List** and **Columns**, **l** / **→** on a previewable file opens the
preview if it is closed and moves keys into that pane. A further **l** / **→**
stays there and does not open the file. Then **j** / **k** / arrows,
**g g** / **Home**, **G** / **End**, **Ctrl+U** / **D** / **B** / **F**, and
**PgUp** / **PgDn** scroll the document instead of the listing. **h** / **←**
returns to the miller column or folder without closing the preview or going to
the parent; a following **h** still goes to the parent. Unpreviewable files and
empty folders flash `Nothing to preview`. Icons have no preview-entry key.

In **Icons**, **h** / **j** / **k** / **l** and arrows (including keypad) always
move to the next icon in that direction, including while the icons on screen are
search results. They stay in the current folder. They never open the preview
drawer, dismiss search, or transfer keyboard ownership into a preview. **Enter** /
**o** still open the focused item. **Backspace** / **Alt+↑** still go to the
parent.

**g g** is first item. **g h** is Home.

## Selection

**Space** toggles the keyboard cursor, not pointer hover, and does not preview.
In List and Columns, preview is **l** / **→**. **i** opens a column or toggles
folder peek.

| Key | Action |
| --- | --- |
| **Space** | Toggle the focused item and move down |
| **v** / **V** | Visual select / visual unset |
| **Ctrl+A** | Select all in the focused pane |
| **Ctrl+R** | Invert the selection |
| **Esc** | Dismiss the current interaction, one step per press; see the precedence below |

On a cursor-only row, **Space** adds that item and moves down; it does not
deselect it. After **Space**, **Ctrl+A**, **Ctrl+R**, or leaving visual,
**j** / **k** / **g g** / **G** / paging move the cursor without rewriting the
fill. **v** then motion starts a new range from the cursor. **V** subtracts the
walked span. In visual mode, **Space** toggles the cursor item without moving it.

### Escape precedence

Dialogs, the shortcut reference, text editors, and an open folder-peek popover
handle **Esc** before browsing; active autoscroll also stops before listing
dismissal. Closing the peek does not close a Miller column. In a footer prompt it
cancels that prompt: **f** clears its filter, **/** / **?** clear find highlights,
and a nonempty **s** keeps its results. An armed chord is canceled before any
listing action.

With no prompt or chord, each press takes the first applicable step:

- **Recursive `s` results:** leave visual mode (keep the fill), close an open
  preview, then dismiss the results. Dismissal restores an earlier committed
  **f** filter if one existed; otherwise it restores the directory listing.
- **Ordinary listing or `f` results:** dismiss the hidden filter, dismiss find
  highlights, leave visual mode (keep the fill), close an open preview, then
  clear the selection.

Unlike **h** while the preview owns keys, the preview-close step actually closes
the drawer. Leaving visual mode or closing a preview can therefore require an
extra **Esc** before retained search results disappear.

## Files

With an empty fill, **y** / **x** / **d** act on the focused item. Typical flow:
cursor or **v** then motion → **y** / **x** → **h** / **l** / **g h** / **g 1**
→ **p**.

| Key | Action |
| --- | --- |
| **y** / **x** | Yank / cut the selection (or the focused item) |
| **p** | Paste. Keep Both is focused on conflicts when that button is offered. |
| **P** | Paste. Replace is focused on conflicts. **Ctrl+V** does the same. |
| **Y** / **X** | Clear copy/cut marks and this process's clipboard payload. Does not wipe another application's clipboard. |
| **d** / **Delete** | Move to Trash with confirmation |
| **D** / **Shift+Delete** | Delete permanently with confirmation. Cancel is focused. |
| **r** / **F2** | Rename the focused item in the footer prompt |
| **a** | Create a file. A trailing `/` makes a folder (stripped before validation). Conflicts error instead of uniquifying. |
| **o** / **O** | Open / Open With |
| **c c** / **c n** | Copy path / name |
| **; 1**–**; 9** / **; 0** | Run the first 10 matching custom actions |
| **.** / **Ctrl+H** / **Ctrl+.** | Show or hide hidden files |
| **, a** / **, m** / **, s** / **, e** | Sort by name / modified / size / type. Shift reverses. |
| **Ctrl+Z** | Undo the last file operation |

Press **;**, then **1**–**9** or **0** (tenth slot) to run one of the first ten
custom actions that match the focused item, or the filled selection when one
exists. Matching, order, and confirmation follow the context-menu catalog:
disabled actions, filter misses, non-native locations, and an 11th match are
omitted. A vacant slot flashes `No action N` and does not run a different
action. See [custom actions](custom-actions.md).

Empty folder: **y** / **x** / **d** / **r** / **Space** flash
`Nothing to yank` / `cut` / `delete` / `rename` / `select`. **i** does not
preview; on an empty folder or a file it opens neither a column nor a peek. In
List and Columns, empty-folder and unpreviewable-file **l** / **→** flash
`Nothing to preview`. Empty clipboard **p** flashes `Nothing to paste`.

**O** looks up file types and application choices asynchronously. For a mixed
selection, Recommended Applications contains handlers shared by every selected
type; Other Applications remains available for an explicit choice. A new key,
selection/focus change, navigation, mode exit, or closed window prevents an older
lookup from opening a chooser over the new interaction. Unreadable files and
broken links report an error instead of guessing a type from the first item.

## Places

Press **g**, then a second key. The chord stays armed while the footer **g-**
mark is showing. Sidebar keycaps appear on Home (**h**), Downloads (**d**),
Trash (**t**), Network (**n**), Recent (**r**), Documents (**k**), Pictures
(**p**), Videos (**v**), and visible PINNED rows (**1**–**9** in display
order), plus a short list of valid second keys. **,**, **c**, and **;** show
the same kind of list (sort options / copy path or name / matching custom
actions) while armed. The second key completes only that chord: **, a** /
**, m** / **, s** / **, e** sort instead of create / search, and search-result
**j** / **h** cannot steal a pending **g**, **c**, or **;**. Sort-chord **, n** /
**, t** cancel with `Unknown chord`.

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
keeps the typed text and stays in the prompt. The entry is cleared on submit,
Escape, focus loss, and mode exit so credentials do not linger.

Slash-containing and home-folder completion uses cancellable background GIO work.
Editing or replacing the prompt discards pending results. Enumeration is bounded
(16,384 entries and 1,024 matching folders); an error or limit leaves the text
unchanged with a hint to check or refine the path. URI input is submitted unchanged
and is not folder-completed. Completion never mounts or probes a typed URI.

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
| **s** | Recursive name search in the current folder (cap 100). The footer shows `search: …` until dismissed. Prompt **Esc** keeps hits; listing **Esc** follows the precedence above. |
| **z** | Jump to a visited folder (fuzzy + frecency) |
| **Z** | Jump to a recent folder (last visit) |
| **a** | Create |
| **r** | Rename |
| **Tab** / **Shift+Tab** | Cycle matching folders in the go prompt |

**/** is a cursor jump; **f** hides non-matches. The prompt stays focused while
you type. Matching substrings stay highlighted after Enter in Columns, Icons,
and List. **Esc** from the listing dismisses those highlights without hiding rows.
Empty **/** with Enter does nothing. **Esc** from the prompt cancels without
leaving highlights. Pressing **/** again shows the prompt. **n** repeats the
last query, including multi-character input.

Enter on **f** commits the filter, closes the prompt, and returns keyboard focus
to the filtered listing without opening an item. A following **Enter** opens the
focused item. Opening **f** again pre-fills the current query. Empty Enter,
**Esc** from the prompt, or **Esc** from the list clears it. View rebuilds keep
the funnel collapsed.

**f** follows the saved **Include subfolders** preference. **s** always searches
the current folder tree, not every indexed root, and adds no full-name find
highlight. **S** is unbound; there is no content search. **Enter** on **s**
applies the query, closes the prompt, and returns keyboard focus to the results,
on the first hit when there is one. It does not open that hit. A following
**Enter** opens it through ordinary item activation, not **Open search results
directly**. **Ctrl+K** global search is unchanged. **Esc** from the prompt keeps
the hits so **v** / **V** / Space / **i** can use them (and **→** in List and
Columns). From those results, **Esc** dismisses visual mode and an open preview
before dismissing search, as described above. In List and Columns, **h** dismisses
search directly unless the preview owns keys, in which case it first returns to
the hits without closing the preview. In Icons, **h** / **j** / **k** / **l** and
arrows move among the result icons and do not dismiss search or open an item.
Search dismissal restores a previously committed **f** filter, or the unfiltered
directory otherwise. Empty **Esc** cancels without retaining search results.
The footer count is the displayed hit count, not the hidden directory's fill.

**z** / **Z** use Strata folder history, not a zoxide database. Empty input
still lists candidates; **Up** / **Down** pick a row, **Enter** goes there.
A miss shows `No matching folders` and does not navigate.

## Search results

While **s** results are showing:

| Key | Action |
| --- | --- |
| **j** / **k** | List and Columns: move the result list. Icons: move to the next result icon in that direction. |
| **l** / **→** | List and Columns: open a directory hit, or enter a file hit's preview when possible. Icons: move to the next result icon. |
| **Enter** | With focus already on the results, activate the focused hit once through ordinary open. **Enter** in the **f** or **s** prompt only applies that prompt. |
| **i** | Directory hit: open an unfocused Miller column, or toggle folder peek, without taking preview ownership. A file hit is not previewed. |
| **h** | List and Columns: leave preview keyboard ownership, or dismiss search (restoring an earlier **f** filter if present). Icons: move to the next result icon. |

**Space** does not preview search rows. Use **g g** / **G** to reach the first /
last result; **Home** / **End** and paging keys are swallowed on result lists
rather than moving the hidden directory cursor. **Ctrl+R** inverts **f** matches,
but is inactive for recursive **s** hits. Once the preview owns keys, its scrolling
map takes precedence.

## Sidebar and surrounding controls

Keyboard navigation stays in the Columns, List, and Icons panes. **Tab**,
**Shift+Tab**, and the arrow keys move among the files there. The sidebar,
window header, footer, preview chrome, and other controls outside those panes
stay pointer-operated. When one of those controls already has focus, the next
**Tab** or arrow key returns to the file list.

**Ctrl+Shift+B** stays with the file list while the mode is on. Show or hide
the sidebar with the header toggle. **Ctrl+L** still edits the location bar.
Menus, dialogs, text fields, and the shortcut reference keep their own keys.

## Still bound

These GUI conventions stay available alongside the Yazi verbs:

| Key | Action |
| --- | --- |
| **Ctrl+C** / **Ctrl+X** / **Ctrl+V** | Copy / cut / paste. **Ctrl+V** focuses Replace on conflicts, like **P**. |
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
**d** / **D** trash / delete, **r** rename, **i** next column or folder peek).
Default-map hints that are unbound or remapped (**Y** for copy path, **Space** for preview,
**Ctrl+R** for rename) are hidden. Copy path is **c c**; Properties is
**Alt+Enter**.

## Not bound

These default-map shortcuts are unbound or remapped while the mode is on:

| Default-map key | In Omastrata mode |
| --- | --- |
| **Ctrl+Shift+K** | Unbound. Use **z** / **Z**. |
| **Ctrl+T** | Unbound. Use the context menu. |
| **Ctrl+\\** | Unbound. Arrows never leave the file list. |
| **Ctrl+D** | Half page down. Duplicate is dropped. |
| **Ctrl+F** | Full page down. Filter is **f**. |
| **Ctrl+B** | Full page up. Sidebar toggle is the header button. |
| **Ctrl+R** | Invert selection. Rename is **r** / **F2**. |
| **y** / **p** (default map) | Yank / paste. Path copy is **c c**; jump to an existing pin with **g** then a digit. |
| **Space** | Toggle selection. In List and Columns, preview is **l** / **→**. |

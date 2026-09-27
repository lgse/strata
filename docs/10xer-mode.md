# 10xer mode

> This document is the target product specification, not a completion report.

10xer mode is an opt-in Yazi-style browsing map. In **Settings → General → Browsing** the row is titled **10xer mode** and its subtitle is **Opinionated keyboard-centric mode with Yazi-style navigation. Disables some features. Toggle with Ctrl-Shift-M.** The footer shows a compact **10X** pill at the right, immediately before the item count, while the mode is on. It hides window Search and pane
Close/filter/refresh/sort chrome in interactive browsers, disables
type-to-search, and uses the footer as the typed-command surface. List column
headings stay clickable. The portal file chooser follows the same preference:
it hides pane Close/filter/refresh/sort chrome, uses this keymap and the footer
prompt, and keeps Accept, Cancel, and the header close control. **Enter** / **o**
confirm a file. **Esc** still cancels the dialog after dismissing a prompt,
filter, or preview. **q** leaves the mode without cancelling. The chooser
continues to disallow folder peeking and column mirroring, so **i** does not
open a peek or an extra Miller column there.

Turn it on in **Settings → General → Browsing → 10xer mode**, or with
**Ctrl+Shift+M**. The choice is saved and live-updates every window. **F1** or
**~** opens the in-app table of commands that currently run; **Settings →
Keybindings** lists that same active map as an all-view overview. While the mode
is on, Settings and the reference show **(experimental feature, under active
development)**. The footer shows only the **10X** pill; the experimental note is
in its tooltip and accessible description.

Paste destinations, cursor versus filled selection, and pointer ownership stay
as in [keyboard navigation](keyboard-navigation.md).

## Enter and leave

| Key | Action |
| --- | --- |
| **Ctrl+Shift+M** | Toggle 10xer mode |
| **q** | Leave the mode. Does not close the window. |
| **Q** | Close the current window |
| **F1** / **~** | Show or hide this reference |
| **Ctrl+,** | Open Settings, canceling any armed chord |

**Ctrl+Shift+M** also works while a browser text field has focus; modal dialogs
keep their own input handling.

**Type to search**, **Keep arrows in file list**, and **Mirror columns selection**
stay saved. While the mode is on they are unused, and those Settings rows show
**Not used in 10xer mode.**

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
| **i** | On a file: toggle the preview drawer without moving focus into it. On a directory: in Columns, open the next Miller column without moving focus into it; in List and Icons, toggle the folder-peek popover. |
| **J** / **K** | Scroll the open preview without taking focus |

Column selection mirroring stays off while the mode is on. **j** / **k** and
**↑** / **↓** only move the cursor. They do not open a child column or a preview.
**l** / **→** enters a directory or a file preview. In Columns, **i** on a directory opens
the next column and leaves focus where it is; moving the cursor does not refresh
that column. A second press does not move focus. In List and Icons, **i** toggles
the existing folder-peek popover for the focused directory. That popover is the
directory peek, not the saved Folder peeking switch and not a file preview.
Pressing **i** again, or **Esc**, closes the popover. **Esc** does not close a
Miller column. **i** on a file opens the preview drawer if it is closed and
closes it if it is open; keys stay in the listing, so **j** / **k** keep moving
the cursor and the preview follows it. Use **l** / **→** (List and Columns) or
**J** / **K** to read it. An unpreviewable file flashes `Nothing to preview`. In
an empty folder **i** does nothing.

In **List** and **Columns**, **l** / **→** on a previewable file opens the
preview if it is closed and moves keys into that pane. A further **l** / **→**
stays there and does not open the file. Then **j** / **k** / arrows,
**g g** / **Home**, **G** / **End**, **Ctrl+U** / **D** / **B** / **F**, and
**PgUp** / **PgDn** scroll the document instead of the listing. **h** / **←**
returns to the miller column or folder without closing the preview or going to
the parent; a following **h** still goes to the parent. Unpreviewable files and
empty folders flash `Nothing to preview`. Icons have no preview-entry key.
While the preview owns keys, an accent bar runs across the top of its header,
like the Miller column destination bar, and no column shows that bar. The listing
keeps its fill and location, and its cursor returns with the keys. Leaving the mode, clicking the listing, or
closing the drawer releases ownership; ownership never outlives the drawer.

### Preview keyboard ownership

Each preview surface owns a fixed set of keys. A key a surface does not use is
swallowed rather than passed to the listing behind it, so no key held by a
preview can launch, move, rename, delete, paste into, or select listing items.
Window commands that do not touch the listing (**q**, **Q**, **F1**, **F5**,
**Ctrl+K**, **Ctrl+L**, **Ctrl+,**, **Ctrl+1**–**3**, **Ctrl+H**,
**Ctrl+Shift+B**, **Ctrl+Shift+M**, and text size) still work. Inside a text
field only **F1** and **Ctrl+Shift+M** still work; every other key is typed or
edits the text.

| Key | Document | Archive tree | Password field | Media |
| --- | --- | --- | --- | --- |
| **j** / **k** / **↑** / **↓** | Scroll | Move the member highlight | Typed / text editing | **↑** / **↓** volume; **j** / **k** swallowed |
| **h** / **←** | Return to the listing | Archive parent; at the archive root, return to the listing | Typed / caret | **h** returns to the listing; **←** seeks −5 s |
| **l** / **→** | Swallowed | Open the highlighted folder; a member file does nothing | Typed / caret | **→** seeks +5 s; **l** swallowed |
| **Enter** | Swallowed | Same as **l** | Unlock | Swallowed |
| **Space** | Swallowed | Swallowed | Typed | Play / pause |
| **i** | Close the drawer | Close the drawer | Typed | Close the drawer |
| **Home** / **G** / **End** | Top / bottom | First / last member | Caret (**G** typed) | Swallowed |
| Paging keys | Scroll half / full page | Swallowed | Text editing | Swallowed |
| **m** | Swallowed | Swallowed | Typed | Mute / unmute |
| **J** / **K** | Scroll | Scroll | Typed | Swallowed |
| **Shift+Tab** | Return to the listing | Return to the listing | Return to the listing | Return to the listing |
| **Esc** | Close the drawer | Close the drawer | Close the drawer | Close the drawer |

Returning to the listing keeps the drawer open and lands on the same cursor.
Closing with **Esc** or **i** also returns the keys to the listing, so the
Miller column regains its destination bar. **i** therefore toggles the preview
from either side: open from the listing, closed from the preview. A
focused preview button or slider (reached with **Tab** or the pointer) keeps
GTK's own **Space** / **Enter** / arrow handling; **Shift+Tab** and **Esc** still
return or close. Browsing archive members never extracts them, opens a listing
file, or changes the listing's cursor or fill. Unlocking a password-protected
archive hands the keys to its member tree. **Ctrl+A** / **Ctrl+C** select all
and copy the document's text, never listing items; with **l** the keys go to the
document itself (text, source, or PDF) once it has rendered.

A password prompt takes focus as soon as it appears, including when **i** or
cursor movement shows a locked archive. It then owns the keys like any other
surface: the header shows the owner bar, the Miller column drops its bar, and
**i**, **h**, **j**, **k**, and **l** are typed into the password until **Esc**,
**Shift+Tab**, or unlocking moves focus. If the drawer is hidden for lack of room
when **l** is pressed, it takes the keys when it reappears for the same file;
moving the cursor first cancels that.

With 10xer mode off the default map is unchanged: an archive tree keeps its
arrow, **k** / **j** / **h** / **l**, **Enter**, **Space**, and **Esc** keys
described in [keyboard navigation](keyboard-navigation.md#navigating-an-archive-preview),
and media keys stay on **Ctrl+Alt**.

In **Icons**, **h** / **j** / **k** / **l** and arrows (including keypad) always
move to the next icon in that direction, including while the icons on screen are
search results. They stay in the current folder. They never open the preview
drawer, dismiss search, or transfer keyboard ownership into a preview. **Enter** /
**o** still open the focused item. **Backspace** / **Alt+↑** still go to the
parent.

**g g** is first item. **g h** is Home.

## Selection

**Space** toggles the keyboard cursor, not pointer hover, and does not preview.
**i** toggles a file's preview without moving focus; in List and Columns,
**l** / **→** enter it. On a directory **i** opens a column or toggles folder
peek.

| Key | Action |
| --- | --- |
| **Space** | Toggle the focused item and move down |
| **v** / **V** | Visual select / visual unset |
| **Ctrl+A** | Select all in the focused pane |
| **Ctrl+R** | Invert the selection |
| **Shift+↑** / **Shift+↓** | Extend the selection |
| **Esc** | Dismiss the current interaction, one step per press; see the precedence below |

On a cursor-only row, **Space** adds that item and moves down; it does not
deselect it. After **Space**, **Ctrl+A**, **Ctrl+R**, or leaving visual,
**j** / **k** / **g g** / **G** / paging move the cursor without rewriting the
fill. **v** then motion starts a new range from the cursor. **V** subtracts the
walked span. In visual mode, **Space** toggles the cursor item without moving it.

A range walks the pane in displayed order, including List type groups, and only
rewrites the fill of that pane. Walking back toward the anchor restores the items
the range had covered; items toggled with **Space** stay toggled. The footer shows
**VISUAL** or **UNSET** while a range is active. Pressing the same key again or
**Esc** leaves visual mode and keeps the fill; the other key starts a new range
at the cursor. Opening another folder, moving to another pane, changing the view,
a pointer selection, **Ctrl+A**, **Ctrl+R**, or leaving 10xer mode also end the
range. In an empty folder **v** / **V**, **Space**, **Ctrl+A**, **Ctrl+R**, and
**Shift+↑** / **Shift+↓** flash `Nothing to select`.

**Shift+↑** / **Shift+↓** add the span from the cursor where the run started to
the moved cursor on top of the kept fill, in displayed order; reversing shrinks
the span back. In Icons they move up or down the grid. The run shows no footer
tag and ends at the next key that is not **Shift+↑** / **Shift+↓**, keeping the
fill; a later run starts at the new cursor. During a visual range they extend
that range like **j** / **k**.

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

While the preview owns keys, **Esc** closes the drawer before any other step
and returns keys to the listing. Unlike **h** while the preview owns keys, the
preview-close step actually closes the drawer. Leaving visual mode or closing a preview can therefore require an
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
`Nothing to yank` / `cut` / `delete` / `rename` / `select`. **i** in an empty
folder does nothing; on an unpreviewable file it flashes `Nothing to preview`. In
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
| **f** | Search results: the folder holding the hit under the cursor, with that item selected. Ends the search. Without hits: `Nothing to reveal`. |
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

An unknown second key cancels with `Unknown chord`.

**g Space** opens **go ›** in the footer instead of toggling the selection.
Typing never navigates; **Enter** submits through the same navigation as the
location bar (**Ctrl+L**). A path may be absolute, start with `~`, or be relative
to the open local folder (`..` and `.` resolve like a shell's `cd`). A path
naming a file opens its folder with the file selected. URI input is submitted
unchanged, except that a password typed in the URI moves into the mount
operation instead of the location. A missing or unreachable destination shows
the location bar's error and leaves the current folder open. Empty **Enter**
just closes the prompt. **Esc** cancels without navigating; clicking a listing
row ends the prompt and keeps the clicked selection. The entry is cleared on
submit, **Esc**, focus loss, a replacing prompt, and mode exit, and it keeps no
undo history, so reopening the prompt or another window cannot recover typed
credentials.

In **go ›**, **Tab** / **Shift+Tab** cycle forward / backward through matching
folders (never files) for the typed prefix: the current listing when the text
has no slash, the parent after a slash, and `~` as home. Matching ignores case,
and hidden folders appear when the listing shows them or the prefix starts with
`.`. A completion keeps the typed form (relative, `~/`, or absolute) and ends in
`/`, so the next **Tab** after typing more descends. Beside the entry the footer
shows the position in the cycle (`2 of 5`) or why nothing changed. No match
keeps the typed text and focus with `No matching folders`; another user's
home (`~name`) shows `Only ~ and ~/ are supported`.

Slash-containing and home-folder completion uses cancellable background GIO
work and shows `Listing folders…` while it runs; the prompt keeps accepting
edits and **Esc**. Editing, replacing, cancelling, or submitting the prompt,
leaving the mode, or closing the window cancels pending work, and a late answer
never changes the text. Enumeration is bounded (16,384 entries and 1,024
matching folders); an error or exceeded limit leaves the text unchanged with
`Can’t read that folder — check the path` or `Too many entries — refine the
path` rather than cycling a partial list. Anything that looks like a URI
(a scheme, `//host`, `\\host`, or `user@host:`) is never completed, mounted, or
probed; **Tab** leaves it unchanged with `URIs are not completed`.

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
| **z** | Jump to a visited folder (name match, then frecency) |
| **Z** | Jump to a recently visited folder (last visit first) |
| **a** | Create |
| **r** | Rename |
| **Tab** / **Shift+Tab** | Cycle matching folders in the go prompt |

**/** is a cursor jump; **f** hides non-matches. The prompt covers the footer and
stays focused while you type. Matching is a case-insensitive substring of the
name, searched in display order from the cursor and wrapping at either end.
Matching substrings stay highlighted in the theme's accent color after Enter in
Columns, Icons, and List. **Esc** from the listing dismisses those highlights without hiding rows.
Empty **/** with Enter closes the prompt and changes nothing. **Esc** from the
prompt cancels without leaving highlights. Pressing **/** again shows the
prompt. **n** repeats the last query, including multi-character input, in its
original direction; **N** reverses it, and either shows the highlights again.
A miss reports `No matches for “…”` and leaves the cursor where it was. Moving
focus out of the prompt (clicking a row, another pane) discards it; the
committed query is per window and forgotten when the mode is left.

**f** filters as you type. Enter on **f** commits the filter, closes the prompt,
and returns keyboard focus to the filtered listing (its first result once the
results arrive) without opening an item. A following **Enter** opens the
focused item. Opening **f** again pre-fills the current query. Empty Enter,
**Esc** from the prompt, or **Esc** from the list clears it. Moving focus out of
the prompt keeps what was typed. View rebuilds keep the filter and the funnel
collapsed. While a filter is active, the footer count is the displayed result
count, including zero, with a file/folder breakdown in its tooltip; motion,
**Enter**, and **Ctrl+R** act on the results, never on the hidden directory's
cursor or fill.

**f** follows the saved **Include subfolders** preference. **s** always searches
the current folder tree, not every indexed root, and adds no full-name find
highlight. **S** is unbound; there is no content search. **Enter** on **s**
applies the query, closes the prompt, and returns keyboard focus to the results,
on the first hit when there is one. It does not open that hit. A following
**Enter** opens it through ordinary item activation, not **Open search results
directly**. **Ctrl+K** global search is unchanged. **Esc** from the prompt keeps
the hits so **v** / **V** / Space / **i** can use them (and **→** in List and
Columns). From those results, **Esc** dismisses visual mode, find highlights, and an open
preview before dismissing search, as described above. In List and Columns, **h** dismisses
search directly unless the preview owns keys, in which case it first returns to
the hits without closing the preview. In Icons, **h** / **j** / **k** / **l** and
arrows move among the result icons and do not dismiss search or open an item.
Search dismissal restores a previously committed **f** filter, or the unfiltered
directory otherwise. Empty **Esc** cancels without retaining search results.
The footer count is the displayed hit count, not the hidden directory's fill.
At its left end, the footer shows the search hit under the cursor as a path
relative to the searched folder. When space is short, an ellipsis replaces the
end of its folder part so the file name stays readable; the tooltip has the full
path. **f** filters show no path. Pressing **s** again while hits are showing
pre-fills their query. A new query,
navigation (including opening a directory hit), leaving the mode, or closing the
window discards the previous query's pending hits; a view change keeps the
search. A folder without a local path, such as Network, flashes
`Nothing to search`.

**z** (`jump ›`) and **Z** (`recent ›`) pick from the folders Strata has
opened, the same saved history as the default map's **Ctrl+Shift+K**, not a
zoxide database. **z** ranks by name match first, then by how often and how
recently each folder was opened; **Z** keeps matching folders in last-visit
order. The folder already open is left out. Candidates list above the footer
as you type, including for empty input, with the first one chosen. **Up** /
**Down** choose another row (wrapping) while the entry keeps focus, and
**Enter** or a click opens the chosen folder once. Editing the text lists
fresh candidates and chooses the first again. A miss shows
`No matching folders`, and **Enter** then leaves the prompt open without
navigating. **Esc**, focus loss, a replacing prompt, and leaving the mode close
the prompt without opening a candidate.

## Search results

While **s** results are showing:

| Key | Action |
| --- | --- |
| **j** / **k** | List and Columns: move the result list. Icons: move to the next result icon in that direction. |
| **l** / **→** | List and Columns: open a directory hit, or enter a file hit's preview when possible. Icons: move to the next result icon. |
| **Enter** | With focus already on the results, activate the focused hit once through ordinary open. **Enter** in the **f** or **s** prompt only applies that prompt. |
| **i** | Directory hit: open an unfocused Miller column, or toggle folder peek. File hit: toggle the preview. Neither takes preview ownership. |
| **h** | List and Columns: leave preview keyboard ownership, or dismiss search (restoring an earlier **f** filter if present). Icons: move to the next result icon. |
| **g f** | Open the folder holding the focused hit and select it there. Ends the search without restoring an **f** filter, so the item is visible; **H** returns. |

**Space** does not preview search rows. Use **g g** / **G** to reach the first /
last result; **Home** / **End** and paging keys are swallowed on result lists
rather than moving the hidden directory cursor. **Ctrl+R** inverts **f** matches,
but is inactive for recursive **s** hits. Once the preview owns keys, its scrolling
map takes precedence.

## Sidebar and surrounding controls

Arrows and **h** / **j** / **k** / **l** stay in the Columns, List, and Icons
panes. **Tab** moves from the file list to the window header. **Shift+Tab**
stays with the files. From the footer or other chrome outside the header and
sidebar, the next **Tab** or arrow key returns to the file list.

**Ctrl+Shift+B** focuses the sidebar when it is visible. A hidden sidebar stays
hidden; the header toggle is what shows or hides it. **Ctrl+B** does not toggle
it. Pressing **Ctrl+Shift+B** again returns to the files.

In the sidebar, **j** / **k** and **Up** / **Down** move between places and
device controls. **l**, **Enter**, and **Space** activate the focused place or
device control. **h**, **Left**, and **Backspace** return to the files. Returning
without activating keeps the file selection. After a place opens another folder,
focus is on that listing.

On a header control, **Enter** and **Space** activate it. **h** and **j** return
to the files and do not change directory or run a file operation.

**Ctrl+L** still edits the location bar. Menus, dialogs, text fields, and the
shortcut reference keep their own keys. The file chooser uses these same
round trips.

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
**d** / **D** trash / delete, **r** rename, **i** quick preview, next column, or
folder peek). The file chooser has no **i** preview, so its Quick preview item
shows no hint. Default-map hints that are unbound or remapped (**Y** for copy path, **Space** for preview,
**Ctrl+R** for rename) are hidden. Copy path is **c c**; Properties is
**Alt+Enter**.

## Not bound

These default-map shortcuts are unbound or remapped while the mode is on:

| Default-map key | In 10xer mode |
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

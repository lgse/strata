# Keymap

Strata's shortcuts can be changed, added or removed, separately for the default
map and for [10xer mode](10xer-mode.md). Use **Settings → Keyboard**, or edit
`settings.toml` while Strata is closed. This guide covers
what can be rebound and how the two ways work. How keys are matched on each
layout is in [Keyboard shortcuts](keyboard-shortcuts.md).

## What can be rebound

Every action that **Settings → Keyboard** lists: window and file commands,
navigation, previews, dialogs, the F1 reference, the file chooser, 10xer verbs
and chords, custom action slots, and Escape, Enter, Tab and the arrows. An
action can have several keys. The first is its primary key, which menus and
hints show; every key on the list works.

These keys stay fixed:

- digit ranges: tab numbers (**Ctrl+Shift+1–9 / 0**) and the digits after the
  Go, Action and Tabs chords, which follow their chord's trigger;
- **Backspace**, which goes to the parent folder and deletes text;
- the keys GTK handles in file lists: **Ctrl+Space** toggles the selection, and
  Shift with an arrow, Home, End, Page Up or Page Down extends it;
- **Space** where it selects in the 10xer file list or plays media, and Home,
  End and the page keys where they move a text caret, seek media or scroll
  something other than the file list.

## Two tables

The default map and 10xer mode each have a table of their own. An action both
use has an entry in each, with its own shipped keys and its own changes:
Rename is **F2 / Ctrl+R** in the default table and **r / F2** in the 10xer
table, and removing **F2** from one leaves the other alone. A key in the 10xer
table works only in 10xer mode.

In 10xer mode, a key the 10xer table does not claim goes on to the default
map's handling, which reads the default table, changes included. A key added
there for hidden files, paging, the first or last item, preview or Back
therefore also works in 10xer mode; one added for Rename, Duplicate, Filter,
the sidebar or global search does not. The file chooser's own keys and
thumbnail resizing use the default table in both modes.

An action only one mode ships a key for usually has no handler in the other. The
default table can still run these 10xer actions: Open with, Move to, Copy to,
Restore, invert selection, search subfolders, the visited and recent folder
jumps, half-page paging, and the Sort, Copy and Action chord steps. 10xer mode
can run duplicate, pin, jump to folder and arrow scope, and copy path on a key
that types nothing, such as one with Ctrl or an F key. A key on any other such action, such as yank or a Go
step in the default table, does nothing, and Settings says so on its row.

## Custom action slots

The default table has ten slots, **Custom action 1** to **Custom action 10**,
under **Custom actions**, with no keys shipped. Slot N runs the Nth action in
the order **Settings → Actions** lists them, disabled ones included, and its
row names that action, or says **No action**. Adding or removing an action can
move the ones listed after it to other slots; renaming one or turning it off
does not. The key runs the action on the selection in the file list, as its
context menu entry does, confirmation included. An empty slot, or an action
that is turned off, cannot run or does not match the selection, runs nothing
else in its place: the footer says **No action N** or **No matching actions**.
The F1 reference lists a slot under **Custom actions** once it has a key.
10xer mode has no slots; its Action chord, **;** then a digit, numbers the
actions that match the selection instead.

## Chord groups

10xer mode has five chords: Go (**g**), Copy (**c**), Sort (**,**), Action
(**;**) and Tabs (**t**). Each has a trigger and steps, rebound separately:
the steps, and the digits after Go, Action and Tabs, follow whatever key the
trigger has. A step works only while its trigger waits for the next key, so a
step key never collides with a top-level one; a top-level key equal to a
trigger is an error. Settings shows a step with its trigger's key in front. No
other chord groups can be made.

Shift with a Sort step reverses that sort, on whatever key the step has. A Sort
step therefore cannot hold Shift, and a digit or a symbol typed with Shift, such
as **<**, sorts but cannot be reversed.

The default table has no chords. Its Sort, Copy and Action steps are plain keys
there, which Shift does not reverse, and its Go and Tabs steps do nothing.

## Escape, Enter, Tab and the arrows

`escape`, `activate` (Enter), `focus_next` (Tab), `focus_previous` (Shift+Tab)
and `up`, `down`, `left` and `right` are structural. Another key on one of them
acts as the key itself wherever Strata or GTK handles that key: in the file
list, the sidebar, dialogs, menus, the file chooser and Settings. **h j k l**
are the shipped arrows of this kind. **Escape** always cancels, even when the
`escape` entry leaves it out.

The text-field limit applies to a plain printable key, one without Ctrl, Alt or
Super:

- it types in a focused text field or popover, and in the default map while
  type-to-search is on, instead of acting as the key;
- one for an arrow moves through the file list, sidebar, header, preview,
  footer panels, file chooser and delete confirmations, but not other dialogs,
  Settings, document tables or the media timeline.

A key with Ctrl, Alt or Super acts in text fields too, but where the field
edits text with it, such as **Ctrl+A**, the field usually wins. In text boxes
with several lines, a key for Enter or Tab does not start a new line or insert
a tab.

## Settings → Keyboard

The **Search keybinds** field at the top narrows the shown table to actions
whose name contains the text, or whose slot runs a custom action that does.
The switch below it shows the **Default** or **10xer mode** table, starting with the active mode. Rows follow the F1
reference's sections, and Settings search finds a row by its action, a slot's
custom action, or a key either table gives it. Each key is
a chip with a remove button; an action without keys shows **Not bound**.

**Add** waits for a key: press it to append it to the action, **Escape** to
cancel, or **Backspace** to remove every key the action has. A key is recorded
the way labels show it, so with a Russian layout the key that types **у** with
Ctrl is **Ctrl+E**. A refused key keeps the capture waiting and says why under
the row. **Reset** on a row restores its shipped keys, and **Reset all** those of
the shown table after asking. Changes save at once and apply in every open
window.

Notes under a row explain keys that do not work everywhere. A key that never
fires is struck through. Where type-to-search takes a plain key, the note has a
**Turn off type-to-search** button.

## Validation

Settings checks each key against the rest of its table when you press it:

- **Refused**: the key never fires where the action runs, or it would stop
  another key from working there. Examples are a key another action on the same
  surface holds, such as **Ctrl+C** on Rename, a 10xer chord trigger, a key GTK
  uses to select, a tab digit, Shift on a Sort step, a key an input method needs
  while you type. An action the mode cannot run has no row in its table.
- **Saved with a note**: the key works, but not everywhere. Examples are a plain
  key while type-to-search is on, a plain key for an action that also runs in
  text fields, a key another action takes first in some states, and
  **Alt+letter**, which the file chooser's accept button may take as its
  mnemonic.

The full list of findings is in
[Checking keymap entries](keyboard-shortcuts.md#checking-keymap-entries).

## settings.toml

Only changed entries are saved, under `[keymap.default]` and `[keymap.tenxer]`
in `$XDG_CONFIG_HOME/strata/settings.toml` (usually
`~/.config/strata/settings.toml`):

```toml
[keymap.default]
rename = ["<Control>r", "<Control>e"]
duplicate = []

[keymap.tenxer]
"go.home" = ["<Shift>h"]
toggle_sidebar = ["<Control>n", "F9"]
```

- An entry replaces the action's whole entry in that table, primary key first.
  An empty list removes every key. Settings removes an entry that returns to the
  shipped keys.
- The ids are the stable names in `src/ui/keys/bindings/`. Rebinding an action
  in Settings writes its id to the file. Quote ids that contain a dot:
  unquoted, TOML reads `go.home = [..]` as a table.
- Keys use GTK's accelerator syntax: `<Control>`, `<Alt>`, `<Shift>` and
  `<Super>`, then a key name such as `e`, `F2`, `Return`, `Escape`, `space` or
  `bracketleft`. Write Shift out (`<Shift>g`); `G` alone means plain **g**,
  which is logged as a warning. Digits are digit-row positions, and a plain
  digit also accepts the Shift AZERTY needs.
- An id Strata does not know, an entry that is not a list, or a key Strata
  cannot read is logged once with the file's path and kept in the file, and that
  action keeps its shipped keys. A newer version's entries therefore survive an
  older version's save. **Reset all** keeps the ids it does not know. Tables
  other than `default` and `tenxer` under `[keymap]` are dropped when Strata
  saves.
- Keys a file adds beyond the shipped ones are checked like keys captured in
  Settings when the tables are first built. Each problem is logged once with
  the file, table, id, key and finding, and the entry still applies.

Strata reads the file at launch, so a hand edit made while it runs takes effect
on the next launch, and a change saved from Settings meanwhile overwrites it. See
[the keymap preference](preferences.md#keymap) for how it is stored and applied.

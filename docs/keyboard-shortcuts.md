# Keyboard shortcuts

This document defines how Strata turns a key press into a shortcut on every
keyboard layout, and how code must read key input. It covers every surface that
handles keys: browser windows, the file chooser, previews, Settings, and dialogs.
What individual keys do is documented with each feature, for example in
[keyboard navigation](keyboard-navigation.md) and [10xer mode](10xer-mode.md).
How users rebind keys, in **Settings → Keyboard** or `settings.toml`, is in
[Keymap](keymap.md).

## How a key press becomes a shortcut

Shortcuts follow the key's label first, then its physical position:

- **Letters** use the letter the active layout prints on the key. On French
  AZERTY, German QWERTZ, or Dvorak, **Ctrl+Z** is the key labelled Z.
- **Non-Latin layouts.** When the key types a character outside printable ASCII
  (Cyrillic, Greek, Hebrew, Arabic, accented letters, and so on), the shortcut
  uses the Latin character that the same physical key types in another
  configured layout, at the same Shift level. If no configured layout has one,
  it uses the key's US-QWERTY character. With a Russian layout active, the key
  that types **с** is **Ctrl+C**, and in 10xer mode the key that types **о**
  is **j**.
- **The active layout's own ASCII character wins** over the fallback. On a
  Russian layout the key that types `.` is **Ctrl+.**; the key that types `ю`
  sits on the US `.` position and also resolves to **Ctrl+.**.
- **AltGr** (third- and fourth-level) characters are text input and never fall
  back.
- **The top digit row is 1–0 by position** on every layout, with or without
  Shift. **Ctrl+1**, **Ctrl+Shift+1** and 10xer digits work on AZERTY, where the
  row types `&é"'(` without Shift, and on layouts whose Shift+digit symbols
  differ from US. A chord's digit step (**t 1**, **g 1**, **; 1**) also accepts
  the Shift AZERTY types digits with. A digit typed elsewhere counts too, such
  as the Hungarian 0 left of 1, and keypad digits count when Num Lock is on.
  With AltGr, a top-row key counts only when it types a digit, as on
  Lithuanian; AltGr symbols there, such as the AZERTY `~` and `@` or the `\` of
  German Mac layouts, stay symbols.
- **`+` and `−` win over a digit position where they have a meaning.** Where
  a top-row key types `+` or `−`, Ctrl shortcuts and **g** read it as that
  symbol: on Swiss German, Ctrl and the `+` key (Shift+1) is **Ctrl++**, not
  tab 1, and on AZERTY **g** then the `−` key unpins. **t** and **;** have no
  `+` or `−` step, so they count the key's position: on AZERTY **t** then the
  `−` key selects tab 6, and on Czech **t** then the `+` key selects tab 1.
- **Keypad keys act as the main keys** they share a label with: keypad Enter,
  arrows, Home, End, Page Up, Page Down, Insert, Delete, Tab, Space, `+`, `−`,
  `*`, `/` and `=`.
- **Caps Lock and Num Lock never change a shortcut.**
- **Shift** is part of a letter shortcut: **Ctrl+Shift+K** is not **Ctrl+K**.
  For symbols, the Shift needed to type the symbol is ignored, so **Ctrl++**
  matches whether or not the layout needs Shift for `+`, and **Ctrl+.** works on
  AZERTY, where `.` needs Shift.
- **Modifiers match exactly** over Ctrl, Alt, Shift and Super. **Ctrl+C** does
  not fire for **Ctrl+Alt+C**. Navigation and dismiss keys (Escape, Enter,
  arrows, Tab, Home, End, Page Up, Page Down, Delete, Backspace, function keys)
  may also accept other held modifiers where a surface allows them; **Escape**
  closes a dialog whatever is held. Letter and symbol shortcuts never do. The one
  digit that does is **0** in media, which accepts the Shift AZERTY needs to
  type it.
- **Other keys for Escape, Enter, Tab and the arrows** act as those keys. A
  press that matches one exactly is read as the key it stands for, so every
  surface that handles that key follows it: **h j k l** are the arrows. A key
  the surface tolerates modifiers on keeps that tolerance for modifiers held
  beyond the other key's own, so **Ctrl+[** for Escape still needs Ctrl, except
  for a plain printable one, such as **j**, which matches only without
  modifiers. A plain printable one is typing in a focused text field or
  popover and while type-to-search is on, and acts only where Strata reads it.
  One for Escape or Enter works wherever Strata handles that key, dialogs and
  Settings included; one for an arrow moves through the file list, sidebar,
  header, preview, footer panels, file chooser and delete confirmations, but not other
  dialogs, Settings, document tables or the media timeline. One with Ctrl, Alt or Super acts
  everywhere, text fields included. Escape itself always works, even where
  another key stands in for it.
- **Typing is unaffected.** Type-to-search, filter fields, rename fields, and
  footer prompts receive the character the layout produced.
- **Text fields** follow the same rules for copy, cut, paste, select all, undo,
  and redo.
- **Input methods** for Korean, Japanese, and Chinese run on a Latin layout and
  pass Ctrl shortcuts through, so the same rules apply without special cases.
  While one is composing, it sees every key first; see
  [Input methods and Chinese layouts](#input-methods-and-chinese-layouts).

## Input methods and Chinese layouts

- **Chinese keyboards use the US layout.** The XKB `cn` layout is US, so every
  shortcut is the US one. Chinese is typed with an input method, such as fcitx5
  or IBus, running on that layout: pinyin for Simplified Chinese, and Zhuyin
  (Bopomofo), Cangjie or Quick for Traditional Chinese.
- **An unfinished composition sees each key first.** While the focused text
  field shows an input method's unfinished text (the preedit, such as pinyin
  waiting for a candidate), each key goes to the input method before any Strata
  command. A key it uses never reaches Strata: Space and digits pick a
  candidate, Enter commits the typed letters, Escape cancels, and arrows, Tab
  and Backspace work in the composition. So Escape does not cancel a rename or
  close the filter, the location bar, a footer prompt, global search, or the
  F1 reference, and Space does not open quick preview from the filter. A key
  the input method passes back works as usual after it: Korean input methods
  such as fcitx5-hangul commit the current syllable and pass back Escape,
  arrows and Ctrl shortcuts, so one press both commits and runs the command.
  Keys the text field handles itself go to the field: Enter submits it, and
  cursor keys move in it. On Wayland the input method runs in the compositor
  and Strata never receives the keys it uses. Input methods that GTK loads into
  Strata (`GTK_IM_MODULE=fcitx` or `ibus`, and every input method on X11),
  GTK's own dead keys, and the Compose key would otherwise get each key after
  Strata's handlers.
- **Input method switch keys reach the focused text field.** **Ctrl+Space**
  (fcitx5), **Super+Space** (IBus), **Shift** and **Shift+Space** go to the input
  method in every text field, the F1 search included. Strata's own
  **Ctrl+Space** toggles selection only in a file list.
- **Zhuyin starts syllables with digit and punctuation keys.** Zhuyin input
  methods, such as chewing, type initials and finals on the digit row and on
  `-` `,` `.` `/` `;`, and tones with 3, 4, 6, 7 and Space. Array30 and
  Boshiamy codes use punctuation keys too. While a text field has focus, no
  Strata command takes a plain digit or punctuation key, so the first key of a
  syllable reaches the input method and the composition then sees the rest. New
  bindings on these keys must keep it that way.
- **Hanyu Pinyin with AltGr dead keys** (`cn(altgr-pinyin)`): AltGr+1 to
  AltGr+4 are the tone dead keys that type ā á ǎ à, AltGr+6 and Shift+AltGr+6
  are the ^ and ¨ dead keys, and AltGr+v types ü. These are text input, never
  digits: **Ctrl+1** and **t 1** use the digit keys without AltGr, as on US.
- **Taiwanese** (`tw`) is US without AltGr. AltGr types characters for
  Taiwanese Hokkien and Formosan languages, including combining marks and dead
  keys on the digit row, which are never digits. The `indigenous` and
  `saisiyat` variants type ʼ on the apostrophe key, and `saisiyat` types ː on the
  semicolon key, which the layout fallback still reads as `;` for the 10xer
  **;** chord.
- **Type-to-search starts without the input method.** An input method composes
  only in a focused text field. A key typed in the file list starts
  type-to-search with the character the layout produced, so a pinyin letter
  arrives as that Latin letter, a Zhuyin key as its digit or punctuation, and a
  dead key starts nothing. Typing in the Open With list starts its search the
  same way. To search with an input method, open the filter with **Ctrl+F**
  (**f** in 10xer mode), or move into the Open With search field, and type
  there.

## Shortcut labels

Every place that shows a shortcut renders it from the binding the app matches:
the F1 reference, Settings, context menus, and footer hints. A displayed
shortcut is therefore always one that works: labels leave out a binding that
never fires, such as `<Shift>exclam` or a plain printable key a user added to
the default map while type-to-search types it, and a message that names keys
that are all unbound shows its keyless form (**Close tab**, not
**Close tab ()**). The Settings keyboard page is the one place that still shows
such bindings, struck through, with the reason beside them.

- **Labels show the Latin key**: **Ctrl+C** with a Russian layout active,
  **Ctrl+1** on AZERTY. Keyboards for non-Latin layouts print the Latin letters
  too, and AZERTY keyboards print the digits, so the label is on the key to
  press. Labels do not change when the active layout changes.
- **Key names follow the interface language**, the way keyboards sold for it are
  printed: German shows **Strg+C** and **Entf**, French **Suppr**, Spanish
  **Supr**. Languages whose keyboards print English names keep them.
- **One notation everywhere.** Modifiers appear in the order Ctrl, Alt, Shift,
  Super. Arrows are ↑ ↓ ← →, and other keys use their full names (Enter,
  Escape, Delete, Page Up). After a modifier, letters are uppercase and Shift is
  spelled out (**Ctrl+Shift+N**). Without a modifier, a letter appears as typed
  (**j**) and Shift+letter as the uppercase letter (**J**), the 10xer
  convention. Chords are separated by a space (**g h**).

## Reading key input in code

### One entry point

`ui::keys` (`src/ui/keys.rs`) is the only module that reads GDK key events.
Install handlers with `keys::on_press(&controller, |controller, press| …)`,
which returns a `Propagation`, and `keys::on_release(&controller,
|controller, press| …)`. Each event is normalized once into a `KeyPress` and
passed to the handler.

While the focused text field is composing with an input method, `on_press`
first forwards the key to that field, as GtkSearchEntry's key capture does,
and calls the handler only if the field and its input method leave the key
unhandled; the release of a key they took is forwarded too (see
[Input methods and Chinese layouts](#input-methods-and-chinese-layouts)). A
field that submits on Enter must also do so from its `activate` signal, which
is how Enter arrives after an input method passes it back.
`ui::keys` follows the preedit of each window's focused `gtk::Text` or
`gtk::TextView` through `preedit-changed`, so handlers need no check of their
own. Release handlers still run.

`keys::capture(anchor, handler)` records a new shortcut for Settings: while the
returned guard lives and the anchor is mapped, every press in the anchor's
window except a lone modifier reaches `handler`, as pressed and before any alias
is applied, and no other handler sees it. Each window holds at most one capture.
`KeyPress::binding()` is the press in the form a keymap saves.

`clippy.toml` lists `EventControllerKey::connect_key_pressed`,
`EventControllerKey::connect_key_released`, and
`GtkApplicationExt::set_accels_for_action` under `disallowed-methods`.
`ui::keys` calls the two key signals under a single
`#[expect(clippy::disallowed_methods, reason = "…")]`, and no other module may.
Nothing sets application accelerators: the dispatchers match catalog constants,
commands through the keymap tables.

### `KeyPress`

- `is(key)` reports whether the press matches a catalog constant: a `Command`
  through the active map's bindings, or a fixed `Binding`. Use it for every
  command, including navigation keys such as Escape, Return, and arrows.
  `is_any(keys)` tries several of one type.
- `digit_in(digits)` returns the digit-row or keypad digit of a catalog digit
  range, such as tab numbers or a chord's digit step. Single digit bindings
  (**Ctrl+1**) go through `is()`.
- `text()` returns the typed character, for typing consumers only.
- `is_modifier()` reports a bare modifier key press, and `modifier_key()` which
  modifier it is.
- `modifiers()` returns the held Ctrl, Alt, Shift and Super, for policies that
  depend on what is held rather than on one binding.
- `used_fallback()` reports a key recognized only through the layout fallback,
  which GTK's own bindings miss.
- `aliased()` reports a press read as a structural command's canonical key,
  and `untyped()` undoes that for a plain printable alias such as **j**, for
  surfaces that never read **h j k l** as arrows. See
  [Structural keys](#structural-keys).

`keys::read_event(&controller, &event)` reads a key event that reached another
controller, such as `EventControllerLegacy`, aliases included. `on_release`
reports the key and modifiers as released, without aliases.

### Commands, bindings, and the catalog

A `Binding` is a canonical key plus exact modifiers: `Binding::plain(Key::j)`,
`Binding::ctrl(Key::c)`, `Binding::ctrl_shift(Key::k)`, or
`Binding::new(key, modifiers)`. Letters are lowercase with Shift explicit.
Symbol bindings name the symbol (`Key::plus`), never the key that types it on a
US keyboard. `binding.ignoring(modifiers)` lets a navigation or dismiss key also
match while those modifiers are held; its label does not change.

A `Command` is a rebindable action with a stable id and its default bindings in
each map, the default map and the 10xer map. Each map's entry is an ordered list
of alternates, primary first: Rename is **F2 / Ctrl+R** in the default map and
**r / F2** in 10xer. A press matches a command through the table of the map
10xer mode selects at that moment, so an alternate bound in one map never
matches in the other. Each table starts from the catalog's defaults, and
overrides saved in `settings.toml` replace whole entries (see
[Keymap](preferences.md#keymap)). A plain printable key an override adds
yields to a focused text field, a popover and active type-to-search.
`command.ignoring(modifiers)` also matches while those modifiers are held
beside each binding's own, which stay required:
refresh rebound to **Ctrl+R** still needs Ctrl. `command.in_map(map)` matches
another map's bindings for a stage that claims them: 10xer swallowing
default-map keys it leaves unbound, and the default-map stages that 10xer key
presses fall through to: keys the 10xer stages do not claim, text fields,
popovers, the file chooser, thumbnail resizing and the filter button's tooltip. Those stages read the
default table, its overrides included, in both modes; a 10xer override changes
only what the 10xer stages claim. `command.alternate(index)` is one binding of
the shipped entry, for a label that shows that alternate alone; once a user
changes the entry, the first alternate shows all of it and the others nothing,
so a row never hides a key the user added. It does not match presses.
Backspace, GTK-owned keys, and generic keys that many surfaces read for their
own job (Space, and Home, End and the page keys outside the file listing) stay
fixed `Binding` constants.

A command one map ships no key for usually has no stage there either. The
window's direct stage (`window/keyboard/direct.rs`) gives some of them one,
listed in `keys::DIRECT_IN_DEFAULT` and `keys::DIRECT_IN_TENXER`: in the
default map Open with, Move to, Copy to, Restore, invert selection, search
subfolders, the visited and recent folder jumps, half-page paging, and the Sort,
Copy and Action chord steps, which there are plain keys with no trigger; in
10xer duplicate, pin, jump to folder and arrow scope. A key a user binds to
one of them runs the handler the other map's key runs, with the same prompt,
dialog or action, on the active map's targets: 10xer duplicate acts on the
fill or the item under the cursor, as 10xer verbs do, and 10xer pin takes the
search hit under the cursor, while default-map verbs act on the selection only,
as Copy does, and do nothing when the focused row is not selected.
In the default map, half-page keys move the selection as Page Down does, and
Escape from the file list ends a search the direct key started. The stage runs after the window commands and before inline
editing, never while a text field or text view (a text preview included) has
focus and never in a file chooser, and the focused 10xer file list passes
typed keys on to it. Neither table binds these commands by default, and Shift
does not reverse a direct sort key.
`conflicts::check` reports `NoPath` for a key on any other command its map
ships no key for, such as yank or a Go step in the default map. Such a key can
only come from a hand edit: the Settings table of that map leaves the command
out.

The direct stage also runs the default map's ten custom action slots,
`keys::CUSTOM_ACTIONS` (`custom_action.1` to `custom_action.10`), which no
table binds by default. Slot N runs the Nth action of `ActionCatalog::actions`,
the order **Settings → Actions** lists, disabled ones included, so a slot keeps
its action when another is turned off. `ActionCatalog::listed` returns it only
when it is enabled, runnable and matches the selection; the key then runs it
through `run_numbered_action`, as the 10xer Action chord does. Otherwise
nothing runs and the footer says **No action N** for an empty slot or
**No matching actions**. 10xer keeps its chord, so the slots are `NoPath`
there.

Every shortcut is a named constant in the catalog, `ui::keys::bindings`, with
one file per area under `src/ui/keys/bindings/`. The `catalog!` macro declares
each area's constants, such as `COPY: Command "copy" = Binding::ctrl(Key::c);`
or `SPACE: Binding = Binding::plain(Key::space);`, in a block that says which
F1 maps list them: `listed` (both), `default_only`, `tenxer_only`, or
`unlisted`. A command is bound in the maps its block lists it in, to one key
or, written `=> [first, second]`, to alternates; `, live in both` also binds it
in the other map, for a handler that reads it there, and an unlisted command
names its maps with `live in default`, `live in tenxer` or `live in both`. A
command whose maps bind different keys lists each,
`=> default [..], tenxer [..]`; the custom action slots list neither, since
they ship no key. One action is one command: a map's alternates
belong in its entry, not in another constant. A chord key names its group,
`"go" trigger(Go)` or `"go.home" step(Go)`, a structural key is
`"up" structural`, and a command that follows another names it with
`, after MOVE_TO_TRASH`. Handlers match catalog constants, and the F1 reference
(`ui/shortcut_reference.rs`), context-menu hints, and footer hints refer to the
same constants. Dispatch logic stays in the handlers; the catalog only defines
keys.

What a reference row or hint shows is a `Keys` value, never free text. It holds
`Shown` keys, written `COPY.shown()` or `shown![TAB_NEXT, TAB_PREVIOUS]`; a
command shows every binding the active map gives it, and `RENAME.alternate(0)`
only its primary:

- one key: **Ctrl+C**
- alternatives: **F2 / Ctrl+R**
- a chord: **g h**
- a digit range with its modifiers or chord prefix: **Ctrl+Shift+1–9 / 0**,
  **t 1–9 / t 0**
- alternatives of different shapes: **g g / Home**

### Structural keys

`ESCAPE`, `ENTER`, `FOCUS_NEXT`, `FOCUS_PREVIOUS` and the arrows `UP`, `DOWN`,
`LEFT` and `RIGHT` are structural commands. The first default binding of each is
its canonical key, and its other bindings are aliases: **h j k l** for the
arrows in both maps. `on_press` reads a press that matches an alias exactly as
the canonical key before calling the handler, so `press.is(DOWN)` matches **j**
and every surface sees the same press; `modifiers()` reports what the canonical
key holds, so **Ctrl+J** standing in for ↓ reads as plain ↓, while `text()`
keeps what was typed. A canonical key, with any modifiers, is never read as
another, and physical Escape matches `ESCAPE` even when a table leaves it out.
A plain printable alias is not rewritten while a text field or popover has
focus or type-to-search is on. In 10xer a read-only text view, such as a text
preview, is a document rather than a field: plain printable aliases are
rewritten there, and a key a user adds acts wherever the command's shipped keys
do. The default map treats it as a field.

A structural command compares the key the press reads as; fixed bindings and
other commands compare the key as pressed, so the 10xer **g h** step still sees
**h**, and also the canonical key when they bind it, so a key that stands in
for Tab completes paths as Tab does. `LEFT.alternates()` matches an alias as pressed, rewritten or not, where
**h** and **←** differ, and `LEFT.or_alternates()` matches either, where a
plain printable alias left alone in a popover should still navigate. A surface
that never read **h j k l** as arrows, such as a dialog, Settings, a document
table or a media timeline, matches `press.untyped()`.

GTK's own bindings see the key as pressed. Every Strata window installs
`keys::install_native_aliases`, a last-in-line controller that gives an alias no
handler claimed, other than a plain printable one, the canonical key's GTK
effect: Enter activates the focused widget, then the first GTK key binding for
the canonical key from the focused widget outward runs, such as a list's cursor
move, a text field's caret move or the window's `move-focus` for Tab, and
Escape closes the popover that has focus. Effects GTK widgets implement outside
key bindings, such as a text view's new line, are the text-field limit.

### Checking keymap entries

`ui::keys::conflicts` checks a binding against the rest of its map's table.
`check(keymap, map, command, binding, policy)` returns the findings for one
proposed binding, `audit(keymap, map, policy)` those for every binding of a
table, and `audit_added` those for the keys entries hold beyond their shipped
ones. `Policy` carries what the table cannot say: whether type-to-search is on.
Each `Finding` names its `severity`, its `kind` and the `binding` it is about,
and `inactive()` marks a binding that never fires in the state checked. An
error means the key never fires where it is bound, or fires and stops another
key or a fixed role from working there; a warning means it works only in some
places.

| Kind | Severity | Rule |
| --- | --- | --- |
| `Duplicate(other)` | Error | Another command the same surface matches holds the key, a text size command, which runs before every surface, a tab command, which a tabbed window's tab strip takes before any surface but dialogs and the file chooser, a key the file chooser reads before it accepts a folder (Escape, rename, properties, the filter, the location, the sidebar, the context menu, 10xer mode, and in 10xer mode every 10xer command its stage claims, the clipboard commands it refuses there included), the F1 reference's key, which toggles it before a media preview command or the open reference's own keys read theirs, Escape, which closes the reference before its own keys, or a key a 10xer media preview reads before a media command (its exits and scrolling, volume on the up and down keys, seeking on left and right, and **h**). A plain digit also matches with Shift, so it is the same key as Shift and that digit. Chord steps count only against steps of their group and against the keys an armed chord reads first: the tab commands, text size, the F1 reference, 10xer mode, Settings and Escape. |
| `ShadowsChordGroup(group)` | Error | A top-level key equals a 10xer chord trigger. |
| `ShiftedSortStep` | Error | A Sort step holds Shift, which reverses a step. |
| `SortStepCannotReverse` | Warning | A Sort step is a digit, which already accepts Shift, or a symbol that Shift turns into another, so Shift cannot reverse it. |
| `TypeToSearchShadowed` | Warning | A plain printable key in the default map while type-to-search is on: one a user adds, on any surface, or a shipped one in the file list other than Space and the **/** that opens the filter. Inactive. |
| `TextFieldLimit(Typing)` | Warning | A plain printable key that focused text fields keep: an alias of a structural key, or a key a user adds to a command that also acts in text fields. |
| `TextFieldLimit(FieldKey)` | Warning | An alias of a structural key that `gtk::Text` binds itself, such as **Ctrl+A** or **Ctrl+Backspace**. |
| `ContextOverlap(other)` | Warning | Both commands take the key in some state, and the stage that sees it first decides: a pair listed as sharing by design (10xer **Ctrl+Page Up** pages where no tab strip takes it), the 10xer list's **<** and **>**, which change the track only while a media preview is open, a focused document's zoom reset, which runs before text size, a preview command, which a focused preview reads before the window's commands, the file chooser's keys read after its folder accept or only in some states, a 10xer key the 10xer stage leaves to the chooser, a modifier a surface tolerates beside the key, or an alias read on a surface the other command belongs to. |
| `ReservedByGtk` | Error | **Ctrl+Space**, or Shift with an arrow, Home, End, Page Up or Page Down, on a command the file list or file chooser matches. |
| `InputMethodKey` | Error | **Ctrl+Space** or **Super+Space**, which switch input methods, on a command that acts while a text field has focus, or a shipped plain digit or `-` `,` `.` `/` `;` `'` `[` `]` on one. |
| `ChooserMnemonic` | Warning | Alt with a letter, which the file chooser's accept button may take as its mnemonic. |
| `FixedKey(fixed)` | Error | A key a fixed role holds where the command acts: a pin, action or tab digit after its chord (an armed chord reads steps first, so `go.home = ["1"]` takes pin 1), **Ctrl+Shift** with a tab digit, **Backspace** (with or without Shift or Super), **Space** in the 10xer file list, which selects the cursor's item, or **Space** on a media preview command, which plays and pauses first, as the page keys, **Home** and **End** seek before the commands the preview reads after them. Only the key as bound counts: a modifier a surface tolerates beside it loses to the fixed role. |
| `NeverMatches` | Error | No press reaches it where it acts: Shift with a symbol Shift types (`<Shift>exclam`), a chord trigger in the default map, which has no chords, or a typed key for a command the 10xer stage leaves to later stages (copy path included) or for the 10xer folder accept, which the focused 10xer file list keeps. An alias on another structural key's canonical key is a `Duplicate`: modifier-tolerant matchers, such as Escape's, still read it as pressed. Inactive. |
| `NoPath` | Error | The map ships no key for the command and none of its stages runs it: a 10xer verb or a Go or Tabs step in the default map, or the type-to-search filter key and the custom action slots in 10xer. The direct stage's commands, the default map's custom action slots, and copy path, which the 10xer file list reads on a key that types nothing, have a path. It is the binding's only finding. Inactive. |

`never_fires` is the cheap part of `check` that labels use to leave a binding
out, and `duplicate_outcome` says which side of a `Duplicate` keeps the key
where the stage order is modeled, so a message can say which one stops working.

A chord step on its own trigger's key, such as **g g**, is a step like any other:
the armed chord reads the second press as the step. Surfaces are a short table in
`conflicts.rs`: the preview, the F1 reference, global search, the file chooser's
own keys, path completion and the trash confirmation each match their own
commands, and everything else is the window's, the direct stage's commands
included; a file chooser declines those in 10xer. The chooser's handler, which
reads some window commands before and after its folder accept, is listed in
that order. So are the commands that act in
text fields and the modifiers each command's surfaces tolerate. These tables
copy the dispatchers' stage order by hand, so a change to where or in what order
a stage reads its keys must update `conflicts.rs` in the same change;
`window::tests::keyboard_dispatch::keymap_conflicts` and
`chooser::tests::keyboard::folder_accept_order_matches_the_conflict_check`
drive the real dispatchers with a row per table and fail when `check` and the
dispatcher disagree.

### Settings keyboard page

**Settings → Keyboard** (`ui/settings/keyboard.rs`) edits the overrides through
`PreferenceManager::set_bindings`, `reset_binding` and `reset_map`, and turns
type-to-search off from its warning; it initializes nothing. A segmented
control shows the default table or the 10xer table, starting with the active
map. Rows come from the registry under the F1 reference's section titles and
order: a command sits in the first section of its own map's reference that names
it, then of the other map's. A table lists only the commands its map can run
(`conflicts::no_path` is false), unbound ones showing **Not bound**, and the
default table leaves out chord triggers. It has no "10xer mode" section: the
commands of that section it can run directly sit under "Search and tools". A
10xer chord step shows its trigger's key before its own.
A **Search keybinds** field above the tables narrows them to actions whose name
contains the text.

Each binding is a chip with a remove button. **Add** turns the row into a
capture: Escape cancels, Backspace unbinds the command, and any other key is
checked with `conflicts::check`. An error keeps the capture waiting and shows
why under the row; otherwise the key is appended to the entry. Findings from
`conflicts::audit` render as text under each row, never as tooltips; a key with
`NeverMatches` shows only that finding, and an inactive key is struck through.
A row's **Reset** appears once its entry differs from the shipped one, and
**Reset all** asks, then restores the shown table. A custom action slot's row names the
action it runs under its title, or **No action**, and follows the
`ActionRegistry` as actions are added, renamed or removed; the page filter and
Settings search also match that name. Settings search finds a row by its
action name or by any key either table gives it; action names score like a
setting's aliases, so a setting that matches as well keeps the query, and the
table switch stays shown. Each command's name is in the
page's `NAMES` table and every catalog; a command with a one-row F1 entry reuses
that row's text. The ten slots share one name, `Custom action %{number}`, which
the page and the F1 reference fill with the slot's number.

### Labels

`keys::label` is the only code that turns bindings into text; `label_in`
renders a given language, `label_in_map` a given map for a hint built once and
shown in one map only, `modifiers_label` held modifiers alone (**hold
Ctrl+Shift**), and `digit_label` one member of a digit range. Key names come from
one translated table (`keys.name.*` in every catalog). Context templates such as
`%{keys} in header` stay translatable and receive the formatted label;
`shortcut_reference::with_keys` fills one, as tooltips and accessible names do.
Text built once, such as a tooltip or keycap, renders through
`shortcut_reference::follow_keys` (or `follow_tooltip` / `follow_description`),
which re-renders it when 10xer mode or a keymap override changes; the F1
reference rebuilds itself and leaves out rows whose keys are unbound.

Alternatives that share their modifiers write them once (**Ctrl+Alt+← / →**)
unless a key is a letter (**Ctrl+U / Ctrl+D**). Minus is shown as **−**.

F1 searches `keys::search_text`: the label in the interface language and in
English, with each alternative also spelled out in full, so **strg+0** and
**ctrl+0** both find **Strg++ / − / 0**. A typed `-` finds the **−** in labels
and the **–** of digit ranges, so **1-9** finds **Ctrl+Shift+1–9 / 0**.

Menus display the formatter's text too. GTK formats a menu item's `accel`
attribute itself: it puts Shift first, capitalizes letters, and uses its own
translations. So menu rows must not show GTK's rendering of an `accel` string,
and no code may build an accelerator string by parsing a label. A native menu
row carries its primary binding's `Binding::accelerator()`, which keeps GTK's
accessible shortcut, and `present_native_items` replaces GTK's rendering with the
formatter's label.

### Normalization

1. Start from `gdk::KeyEvent::match_()`, GTK's accelerator form: Caps Lock is
   ignored, modifiers consumed to produce the key are removed, and letters are
   lowercased with Shift added back. Keep only Ctrl, Alt, Shift and Super.
2. If that key is a character outside printable ASCII and the event level is 0
   or 1, replace it with the first printable-ASCII key that
   `Display::map_keycode` reports for another layout at the level Shift
   selects, then canonicalize it the same way. Otherwise use the built-in
   US-QWERTY table entry for the keycode. The level comes from Shift, not from
   the event, because Caps Lock raises the event's level for letters. GDK
   keycodes are evdev codes plus 8 on both X11 and Wayland. If neither source
   has a Latin key, keep the original.
3. At level 0 or 1, digit-row keycodes 10–19 map to 1–9 and 0 regardless of
   the key they type. At a higher level (AltGr, or a Caps Lock level some
   layouts add) a digit-row key counts only as the digit it types, because GDK
   reports no AltGr modifier. Another key counts as the digit it types at level
   0 or 1. Digit matching
   compares the event's modifier state before consumed modifiers are removed,
   so **Ctrl+1** and **Ctrl+Shift+1** stay distinct on every layout.
4. Keypad keys become the main keys they share a label with. Keypad digits stay
   keypad keys and count as digits when Num Lock is on.

The common path costs one `match_()` call and a range check. `map_keycode`
runs only for keys outside ASCII, and nothing is cached, so no keymap-change
listeners are needed.

### Built on GTK

The hard parts come from GDK, which Strata already uses. `ui::keys` adds only
what GTK lacks.

- `KeyEvent::match_()` handles Caps Lock, consumed modifiers, and case folding.
  Do not reimplement them.
- `Display::map_keycode()` supplies the other layouts' characters for the
  fallback.
- `gtk::accelerator_name()` builds a menu item's `accel` string from a binding.

GTK lacks three things: a fallback when no configured layout is Latin,
position-based digits (its own matching misses **Ctrl+1** on AZERTY), and a
single answer per event. `KeyEvent::matches()` is not called per binding:
Strata's dispatcher tries bindings in order, so a physical-key match checked
first could beat an exact match checked later, and each non-exact check looks
up the keymap again.

Labels do not use `gtk::accelerator_get_label()` or `GtkShortcutLabel`
(deprecated since GTK 4.18). GTK puts Shift first, capitalizes plain letters so
**j** and **J** look the same, spells out arrows and symbols, leaves some names
untranslated, and translates with the system locale rather than Strata's
language setting. Its translations are still a useful reference when filling in
the key-name table.

No third-party crate is needed. `keycode` or `xkbcommon` could stand in for the
US-QWERTY table, but that table is about 50 static entries. `keyboard-types`
and `xkeysym` duplicate types GDK already provides.

### Window actions

Search (**Ctrl+K**), jump to folder (**Ctrl+Shift+K**), open terminal
(**Ctrl+Alt+T**), refresh (**F5**), and arrow scope (**Ctrl+\\**) are bindings in
the window dispatcher, not GTK application accelerators. The dispatcher checks
them after its own stages yield the key and activates the same guarded `win.*`
actions; the key is consumed whenever the action is enabled.
`add_guarded_window_action` still decides whether they run while a modal or an
inline edit owns input. Each action runs wherever the active map's table binds
it, text fields included, except open terminal in 10xer; the dispatcher checks
when the key arrives. 10xer mode ships no key for jump to folder and arrow
scope, which a key bound in its table runs here and through the direct stage.
Open terminal keeps **Ctrl+Alt+T** in the 10xer map, where the file list and
sidebar run it, but this after-yield path does not, so in a text field it does
nothing.

### Text fields

When a `gtk::Text` or `gtk::TextView` has focus and a press matched an editing
binding only through the layout fallback, the dispatcher activates the widget's
own action (`clipboard.cut`, `clipboard.copy`, `clipboard.paste`,
`selection.select-all`, `text.undo`, `text.redo`) and stops the event. Otherwise
GTK's own text bindings handle it. The file chooser applies the same rule
(`edit_focused_text`). `keys::TEXT_EDITING` holds these bindings and their
actions, and the open F1 reference lets the same bindings reach its search
field.

Plain keys that only the fallback recognizes stay text where typing goes:
type-to-search starts with the **þ** that Icelandic types on the US `/` key
instead of opening an empty filter, and in 10xer mode the Russian **Ё** key
reads as **~** but types **Ё** into the F1 search instead of closing the
reference. In 10xer mode, text fields such as the location bar type **~**
itself rather than opening the reference; only the open reference's search
closes it on **~**.

A focused cell of a document table follows the same rule for copy and select
all. The table's key handler runs the cell's action, because it owns the
selection that spans cells: **Ctrl+C** copies that selection when there is one,
and **Ctrl+A** clears it first.

## Adding a command

1. **Register it in the catalog.** Add the command to `ui::keys::bindings`, in
   the catalog block for the F1 maps that list it, with its id and each map's
   defaults (see [Commands, bindings, and the catalog](#commands-bindings-and-the-catalog)).
   The id is what `settings.toml` saves, so it never changes once shipped. Mark
   it `live in both` only when a handler reads it in the map its block leaves
   out. A new key for an existing action is another alternate in that command's
   entry, not a new constant.
2. **Name it.** Add it to the Settings keyboard page's `NAMES` table
   (`ui/settings/keyboard.rs`), reusing its F1 row's text when that row names
   it alone, and translate the name in every catalog (see
   [internationalization](internationalization.md));
   `settings::keyboard::tests::every_command_row_has_a_name_in_every_catalog`
   fails until it has one.
3. **Add its F1 row.** Reference the same constant from its row in
   `ui/shortcut_reference.rs` and from any context hint. The label renders
   itself; translate only the action text.
4. **Give it a dispatch path.** Match it in the handler of the stage that owns
   it with `press.is(…)` or `press.digit_in(…)`; a default-map stage that 10xer
   presses fall through to matches it with `in_map(Map::Default)`. A map that
   ships no key for it runs it only from a stage that reads it there: list it in
   `keys::DIRECT_IN_DEFAULT` or `keys::DIRECT_IN_TENXER` for the direct stage, or
   it is `NoPath` in that map. Never compare `text()` or a raw keyval to
   recognize a command, never type a key label by hand, and never add a second
   spelling (an uppercase letter or a US shifted symbol) to cover another
   layout.
5. **Update `conflicts.rs`.** A command only one surface matches goes in
   `SCOPES`, one a stage matches while a text field has focus in
   `IN_TEXT_FIELDS` or `IN_DEFAULT_TEXT_FIELDS`, one matched with `ignoring(..)`
   in `TOLERATED`, one the file chooser reads around its folder accept in
   `CHOOSER_FIRST` or `CHOOSER_SHARED`, and one the 10xer stage leaves to later
   stages in `TENXER_PASSES`. `keys::conflicts::tests::audit_of_default_tables_is_clean`
   checks the shipped tables against every rule. Run the guard tests
   `window::tests::keyboard_dispatch::keymap_conflicts` and
   `chooser::tests::keyboard::folder_accept_order_matches_the_conflict_check`,
   and give them a row when the command's surface or stage order is new.
6. **Test it.** Extend the normalizer table tests when a binding uses a symbol
   or digit, and add QA scenario coverage for a new input route.

## Testing

Rust tests emit `key-pressed` without a GDK event. `KeyPress::from_signal`
reads such a press as a US-QWERTY press, with Caps Lock matched the way GDK
matches it, so handler tests stay layout-neutral and the layout cases live in
the normalizer tests and QA scenarios.

`ui::keys::tests` drives the pure normalizer from hand-written keymap rows for
US, Russian, Ukrainian, Greek, Hebrew, French AZERTY, German QWERTZ, Hungarian,
Lithuanian, Dvorak, Caps Lock, and a Russian-only keymap, plus a Hanyu Pinyin
(`cn(altgr-pinyin)`) dead tone key recorded from GDK. Plain `cn` reports exactly
the US keymap, so the US rows cover it. A GTK test delivers real key events to
check that key handlers yield to a composition in the focused field and still get
the keys its input method passes back. The tests also cover label rendering
(notation, translated key names, chords, digit ranges) and catalog completeness:
every listed constant has a row of its own in each F1 map its catalog block
names (`listed` for both, `default_only`, `tenxer_only`), a chord's second key
right after its first, or is explicitly marked as unlisted. `ui::keys::command`
tests check that commands are bound in exactly the maps their block and
`live in both` name, that each merged command keeps the keys each map had, that
no entry lists a binding twice, that ids round-trip, that chord steps follow
their trigger, and that every fixed binding has a reason not to be a command.
`ui::keys::conflicts` tests give each finding kind a row that reports it and
one that does not, and audit both shipped tables: they hold no errors, and only
the expected notes for **h j k l**, **Ctrl+0** (text size and a focused
document's zoom reset), the Ctrl page keys in 10xer, preview keys that a
focused preview reads before the window's (**Ctrl+F**, **Shift+Enter**, and in
10xer **/**, **G**, **+**, **=** and **-**), and the default map's plain keys
while type-to-search is on. Run
them with `./scripts/test-headless.py keys::`, and run
`./scripts/test-headless.py i18n::` after changing key names.
`./scripts/test-headless.py settings::keyboard` covers the Settings page:
capture through the page's key reader, refused and saved keys, resets per row
and per table, live updates from another window, and search;
`window::tests::preferences::capture_commits_and_second_window_follows`
captures in one window's Settings and presses the key in another.
`window::tests::keyboard_dispatch::direct_commands` binds each direct command in
the map it gains and requires the same effect as the other map's key, and
checks that the shipped tables reach none of them.
`window::tests::keyboard_dispatch::file_verbs::default_map_custom_action_slots_run_their_listed_action`
binds custom action slots and requires each to run its listed action or
nothing.
The QA scenarios in `tests/qa/scenarios/keymap/` rebind through the real
Settings page (`settings-capture-rebinds-rename`) and through a saved
`settings.toml` (`saved-keymap-applies-at-launch`), keep the two tables apart
(`keymap-tables-change-independently`), use Enter and Escape aliases
(`structural-aliases-act-as-enter-and-escape`), record a key on a Russian
layout (`capture-records-latin-key-on-russian-layout`), and refuse a taken key
(`capture-refuses-a-taken-key`).

QA scenarios switch layouts on the run's private Xvfb with a `layout` step:
`setxkbmap` loads the layouts and an XKB group lock selects the active one. Never change the
desktop's layout from a test. Scenarios use US, Russian, Russian-only, French
AZERTY, German QWERTZ, Caps Lock, and Hanyu Pinyin. With Hanyu Pinyin, GTK's
own input method composes the AltGr tone keys in text fields, which stands in
for a pinyin input method's composition. See
[testing](testing.md).

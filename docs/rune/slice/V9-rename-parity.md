# V9 Rename parity

| | |
| --- | --- |
| Status | Done. Gate G3 passes: 13 of its 33 runs are `required` on both targets, three times running. The other 20 are pending: 17 `scope`, 2 `platform`, 1 `rune` |
| Depends on | V8 |
| Unblocks | V10 |
| Owns | The rename wiring in `crates/strata-app`, the `field` half of the Rune QA backend, G3's ledger entries, one mutation patch |
| Commits | `3ba9a1f312ae2b9163c9dde892af88b866bf4bbe` (the shared rules, a refactor), `29410171c` and `236f6c9f8` (two component changes), `ef764f73f` (the wiring), `71f3a9c97` (`field`), `e0ea9caaa` (dead keys), `3b473cbfd` (the ledger), `61a7d6224` (the mutation patch), then this record |

## The result

| | Runs | Of 33 |
| --- | --- | --- |
| `required`, passing on GTK and on Rune, three times running | 13 | 39% |
| `scope` pending: undo (7), a new item (8), the filter and then a new item (2) | 17 | 52% |
| `platform` pending: an input method the pinned image lacks | 2 | 6% |
| `rune` pending: the Escape after a dead key | 1 | 3% |
| `component`, `harness`, `toolkit`, `deviation` | 0 | |

The gate has no stop condition of its own. Counted as V8 counted G2's, after the first
sort: one run is `rune` pending and none is `component` pending, 1 of 33. Two component
gaps were found, and both were closed in `strata-components` rather than left pending
([below](#the-two-component-changes)).

## Goal

Gate G3: inline rename in Columns goes through Strata's rename operation and validator,
and the `inline_renaming` scenarios pass on both targets with every filesystem check
agreeing.

This is the slice's one write path. It proves a file operation started in the Rune
interface reaches Strata's services and comes back as events.

## The set

`tests/qa/scenarios/inline_renaming`: 16 files, 15 of them with Columns runs. Their headers
give 33 Columns runs; 31 ran and passed on GTK at the V3 baseline and the runner ignored
two. Take the list from the runner.

The set leans on three things the read path barely used:

- the `field` predicate and target, about 80 uses: the focused text field's name, text,
  caret and selection;
- `fs` steps, which the runner performs and checks on disk for either target;
- `editor_within_viewport` and `editable_fields`.

## What V8 left

- **Where the editor opens.** `Intent::BeginRename` in
  `crates/strata-app/src/columns/intents.rs`, and since V8 the context menu's Rename row
  (`Wired::Rename` in `chosen`). Both log today. This plan left rename from a context
  menu out of scope; the row is there and waits.
- **The field is one more thing the tree may hide.** The backend now presents two things
  from `Runtime::elements()` that Rune's accessibility tree leaves out: a row's name as its
  `label`, and the panes behind an open menu (`crates/strata-app/src/qa/tree.rs`,
  `stage.rs`). If the editor is modal, or a field's caret is not in the tree, the same
  route is there: `ElementInfo` carries `caret` and the selected text.
- **Ids.** A row's key is `row-<EntryId>`, and an entry's id follows its `Location`, so a
  rename gives the row a new key. `remember` keeps a node by its key path and role, so a
  kept row is gone after a rename, as a GTK row object is not. The snapshot's label node
  is `<row path>/@label`.
- **The layout step.** G3 has one non-US step, `cn-altgr-pinyin` in
  `tone-mark-escape-before-rename`. V7's keymap tests cover `ru`, `fr`, `de` and
  `caps-lock`, not that one, and the scenario composes with a dead key: check what Rune's
  `Runtime::key` can carry of a composition before sorting it.
- **Mutations.** `./scripts/qa-parity.sh mutations` exists. Add
  `tests/qa/parity/mutations/<scenario>.patch`; the script refuses a patch whose scenario
  has no `required` run.
- **Reason codes.** There are seven: a run that needs a part the slice marks Out is
  `scope`, with the part's work package and the first step that needs it.
- **Settings are written now.** `strata-app`'s `settings::Saved` saves what the browser
  says changed. A rename saves nothing, but a test that reads `settings.toml` should know.
- **Rune tests share one process's language.** `rust_i18n::set_locale` in a test races
  every other test of the binary; ask a catalog by name with `t!(…, locale = "de")`.

## The three parts

**Strata's operation.** `Browser::rename(entry, new_name)` validates the name with
`validate_basename`, starts the operation through the `OperationProvider`, and returns its
request id. The result arrives as `RenameCompleted`, `RenameFailed` (with a message; no
request id when the name was rejected before an operation started) or `RenameAbandoned`,
and the moved item as `ItemRelocated`. `Browser::rename_item()` says which entry F2 means.

**Strata's rules in the GTK view.** `src/ui/collection_edit.rs` opens the field, selects
from 0 to `rename_stem_end(name)`, the characters before the last dot when the dot is not
first, and checks each edit with `name_field_error`. These are under `src/ui` today, so
the Rune interface cannot call them.

**The component.** strata-components' C5: `RenameState` (`begin`, `commit`, `cancel`,
`tick`, `error`) and `RenameEditor`, which fills the collection's rename slot and takes a
`validate` hook and an `on_intent` hook. It commits on Enter, Shift+Enter, Tab, blur and a
press outside, and cancels on Escape, on scrolling out and when the entry leaves. Its
`initial_selection` already follows Strata's stem rule.

## Scope

1. **Share Strata's rules.** Move `rename_stem_end` and `name_field_error` out of
   `src/ui/collection_edit.rs` into a shared crate, with their tests, and have the GTK
   view call them there. This is a refactor of the GTK app with no behaviour change.
2. **Wire the editor.** `BeginRename` from the view, and F2 through
   `Browser::rename_item()`, call `RenameState::begin`. `RenameEditor::validate` calls the
   shared `name_field_error`. A commit calls `Browser::rename`; a cancel leaves the name.
3. **Compare the selection rules.** The slice plan says the stem-then-whole-name rule
   comes from Strata. Check the component's `initial_selection` against the shared
   `rename_stem_end` on the names the scenarios use: dotfiles, names with several dots,
   folders. A difference is a `component` finding, and Strata's rule wins.
4. **Follow the events.** `RenameCompleted` closes the editor and keeps the cursor on the
   entry under its new name. The entry's `Location` changed, so its `EntryId` may: move
   what is held by the old id with the selection model's `relocate`. `RenameFailed` shows
   its message in the editor's message slot and keeps the field open.
5. **A typed path moves the file.** The component reports one ending rule with three
   outcomes: Rename, Move for a typed path, Keep. `rename-moves-into-path` covers it.
   Route Move to the operation Strata uses for it, or mark the run `pending` with the
   reason if that needs something outside the slice.
6. **Fill the snapshot's `field` key.** In the backend, `Backend::field` returns the
   focused field's name, text, caret and selection. Offsets are character counts, as
   AT-SPI reports them; convert if Rune reports bytes. The field's accessible name must be
   the one the GTK field has, which the scenarios check.
7. **Add a Rune mutation patch** for rename, named for the scenario that catches it. The
   GTK patches in this set are for `rename-commits-on-enter`, `escape-preserves-name`,
   `long-rename-keeps-caret` and `rename-commits-on-click-away`; pick the matching
   behaviour.
8. **Run and sort the set** as V8 did: fix, implement, or mark `pending` with a reason and
   a blocker. Three clean runs.

## Out of scope

- The rename dialog for conflicts beyond the inline error. Rename from a context menu was
  out when this was planned; the owner then widened the slice to the context menus
  (decision 19), so the menu's Rename row opens the editor as F2 does.
- New Folder and New File naming (`Browser::name_new_item`).
- Undo of a rename, unless a scenario in the set needs it. Then it is `pending`.
- Rename in Icons and List.

## Tests

| Check | Command | Expect |
| --- | --- | --- |
| The shared rules | `./scripts/test-headless.py` with the moved tests' filter | Pass, same count as before the move |
| The GTK app | `./scripts/qa.sh` for `inline_renaming`, and `./scripts/qa-mutation.sh` for its four patches | As at the baseline |
| The wiring, in process | `cargo test -p strata-app --features qa` | Begin, type, commit renames the file on disk; a bad name keeps the field open with Strata's message; Escape leaves the name |
| G3 on both targets | `./scripts/qa-parity.sh g3`, three times | Every `required` run passes each time |
| The filesystem | The scenarios' `fs` steps | Every check agrees across targets |
| Mutation | `./scripts/qa-parity.sh mutations` | The rename patch fails its scenario |
| No GTK | `cargo tree -p strata-rune -i gtk4` | No match |

## Done when

- G3's runs are all in the ledger, and the `required` ones passed in three consecutive
  runs on both targets.
- Every `fs` check agrees across targets.
- The rename mutation patch fails its scenario on the Rune target.

## Risks

- **Offsets.** A caret counted in bytes passes every ASCII scenario and fails the first
  one with an accented name. Test a multi-byte name in process.
- **Timing of the commit.** A commit on blur or on a press outside races the event that
  caused it. `rename-commits-on-click-away` is the scenario most likely to be flaky.
- **Two validators.** Until step 1 lands, the component could validate with its own rule.
  Do step 1 first.
- **Real writes.** This package renames real files. In-process tests work only in
  temporary directories, and the QA runs only in the container's fixture.

## The record

### The shared rules

`3ba9a1f312ae2b9163c9dde892af88b866bf4bbe`, a refactor with no behaviour change.
`rename_stem_end`, `name_field_error` and the `Naming` they take moved from
`src/ui/collection_edit.rs` to `strata_fs::services::name_field`, beside `validate_basename`
and `TypedPath`, which they call. The GTK view calls them there: `collection_edit.rs`
imports them, and the two other callers (`src/ui/chooser/state.rs`,
`src/ui/window/keyboard/files.rs`) name `crate::services::rename_stem_end`. Nothing else
under `src/ui` moved. The messages are `validate_basename`'s and `TypedPath::parse`'s,
already in `strata-fs` and in the thirteen catalogs: no key and no owner changed.

**The two functions had no test of their own.** This plan asked for them to move "with
their tests"; there were none in the tree. Three table tests come with them
(`services::name_field::tests`): the stem's end over dotfiles, several dots, a trailing dot
and names of more than one byte a character; an empty field; and each `Naming`'s refusals.

| `./scripts/test-headless.py rename`, on the host | `strata` | `tests/qa.rs` | `strata-fs` |
| --- | --- | --- | --- |
| Before, at `92204a5f3` | 25 pass | 1 passes, 51 ignored | 87 pass |
| After | 25 pass | 1 passes, 51 ignored | 88 pass: the 87, and the new stem test, whose name has "rename" in it |

`./scripts/test-headless.py name_field` runs the three new tests in `strata-fs`.

**The selection rules agree.** strata-components' `initial_selection` and `rename_stem_end`
are one rule: a file selects up to its last dot unless that dot leads the name, and a
folder, a link to one and a name without a dot select everything. The component counts bytes
and the GTK field characters.
`rename::tests::the_editors_first_selection_is_stratas_stem_rule` holds the two together
over the names G3 renames and over a leading dot, several dots, a trailing dot and names
that are not ASCII. No `component` finding came of it. Validation is not the component's at
all: its editor asks a hook, and the hook is `name_field_error`.

### The wiring

`ef764f73f`. `crates/strata-app/src/rename.rs` is the policy, ported from
`src/ui/browser/inline_edit.rs`; `columns/editing.rs` is where the editor sits in the
columns.

| | In `strata-app` |
| --- | --- |
| F2, Ctrl+R | The window's key table: `Browser::rename_item` names the entry, after the view's selection is handed to the browser |
| The view's `Intent::BeginRename` | A slow second click on an entry: that entry. It shares the menu row's path and has no test of its own |
| The context menu's Rename row | The entry is selected alone and then renamed, as `rename_context_entry` does |
| The field's check | `name_field_error` with `Naming::Path` for a local entry and `Naming::Single` elsewhere, translated by the catalogs. The field's accessible name is the catalogs' "Rename", as the GTK field's |
| Enter, Shift+Enter, a click away, Tab | The editor closes itself and says what it closed on. A changed, valid name goes to `Browser::rename` |
| A typed path | `TypedPath::parse`, then `Browser::place_entry`, which moves the item and makes the folders it needs. A folder typed with a trailing slash is a plain rename. When the move lands, what took the item's row is selected with `Browser::reveal_locations`: the first folder of the path, or the item itself in the folder above (`reveal_placed_entry` in the GTK view) |
| Escape, an unchanged, empty or refused name | Nothing is asked of the browser |
| Tab, Shift+Tab | After the commit the view's cursor moves to the neighbour, the child column follows at once (`mirror_rename_target`), and the editor opens there. At the listing's end it only commits |
| `RenameCompleted` | Nothing more for a plain rename: the browser keeps the cursor and the selection on the entry itself ([ids](#ids-the-cursor-and-the-selection-after-a-rename)) |
| `RenameAbandoned` | The rename is forgotten: a later operation took its place |
| `RenameFailed` | The editor opens again on the entry, with the typed name, the browser's message under the field and as its description, and the field marked invalid. Typing asks Strata's rules again. This is what this plan asked for; the GTK view shows an "Unable to rename item" dialog, which the slice leaves out |
| An open editor | Holds the mirror back, as `child_mirror_blocked` does. The window's keys wait while the field has the keyboard, so F5, F2, Ctrl+. and Alt+Up do nothing there. A right click elsewhere commits the edit before its menu opens, since Rune's field keeps the keyboard through that press; no test holds that one |

F5 is new in the window's key table: it reads every column again
(`Browser::refresh_all`, as `refresh_browser` does in Columns). The slice's scope names the
default key map, and `window-accelerators-leave-rename/columns-f5` is only a real check
with the key bound.

Not ported, each named in the module: naming a new folder or file (`Naming::NewItem`),
undo, the label showing the new name before the rename lands (`start_pending_rename`),
and the field's own menu.

**In process** (`mise run test-rune rename::`), each test in a temporary folder and naming
its scenario. After every rename the test compares three things: the names the view lists,
the names in the folder on disk, and what `Browser` holds (the focused entry, the active
column, each column's selection).

| Test | Shows |
| --- | --- |
| `columns::tests::rename::enter_renames_the_file_and_the_cursor_stays_on_it` | `rename-commits-on-enter`: the stem is selected, the file is renamed on disk, the row shows under its new name, selected and focused |
| `…a_refused_name_keeps_the_field_open_with_stratas_message_and_renames_nothing` | Three refusals with their messages, the field open and invalid; Enter then keeps the name; Escape leaves it |
| `…a_typed_path_moves_the_item_and_what_took_its_place_is_selected` | `notes/todo.txt` and `../notes.txt` |
| `…tab_renames_the_folder_and_carries_the_editor_and_the_child_column_on` | `tab-renames-past-open-folder/columns-mirrored`, with Shift+Tab back |
| `…an_open_column_follows_its_folders_rename_when_nothing_mirrors` | `…/columns-unmirrored`: `ColumnsRelocated`, which V6 left untested |
| `…a_rename_the_browser_fails_opens_the_editor_again_with_its_message` | A name that is taken: `RenameFailed`, and a retry that goes through |
| `…a_rename_inside_a_range_keeps_the_range_and_its_anchor` | `ItemRelocated`, below |
| `…the_windows_keys_wait_while_the_field_has_the_keyboard` | `window-accelerators-leave-rename`, with four keys the window does bind |
| `qa::tests::rename::…` (four) | The scenarios' own steps through the engine, [below](#field) |

### Ids, the cursor and the selection after a rename

V6 wrote that nothing needs `Selection::relocate`: `Browser` says what is selected after a
rename and the adapter writes it. That was unproven, and it was wrong for a range.

- **What `Browser` does.** It moves its own cursor and selection to the renamed entry
  (`follow_moved_selection`), so a plain rename needs no reveal from the view.
- **What the view lost.** The renamed entry has a new location and so a new id. With the
  adopt path alone the view dropped the old id and took the browser's selection back with
  the anchor on the cursor. A Shift range that held the renamed entry then grew from the
  wrong end: the range test above fails that way with the relocation switched off.
- **What is built.** `ItemRelocated` reaches the adapter as the two ids. They wait until
  the listing shows the change, and then the column's selection is told
  (`Selection::relocate`) right before it is brought in step with its rows. The component
  asks for exactly that order.
- **Why they wait.** The browser raises `ItemRelocated` in one GLib dispatch and changes
  the listing in a later one. Told at once, the selection held an id its listing did not
  have. The view then handed the browser an empty selection for that column, which is
  where the first in-process test failed; and a live range tripped the component's own
  check that a selection is in step with its source.
- **When the monitor is first.** If the folder's monitor reports the rename before the
  operation does, the listing has changed already and the pair is dropped. The cursor and
  the selection are still right, from the browser; a range's anchor is then the cursor.
  This order was not seen in the tests.

This does not put decision 11 in question. The view still interprets input and `Browser`
still holds the result; what was missing was the one case where the browser changes an
entry's identity under a selection the view holds.

A `remember`ed row is still gone after a rename, since its key is its id. No run of G3
recalls one.

### The two component changes

Both are in `strata-components`, under the rules for components: no text, no golden
changed, a unit or page test each, and a row in [`STATUS.md`](../work/STATUS.md).

| | Was | Now | Found by |
| --- | --- | --- | --- |
| C5a, `29410171c` | The editor returns the keyboard to its row's collection when Enter or Escape removes the field. A Columns column is keyed by its host and takes no keyboard, so the keyboard was left nowhere | `RenameEditor::focus_return` names the element; `strata-app` names the strip. The default is unchanged | The first engine test: `wait focused` after Escape |
| C2e, `236f6c9f8` | Left in a column with no cursor landed on its last entry. A click on a folder leaves its column the keyboard and no cursor | It goes to the parent column, as `Browser::focus_parent` does whatever the cursor (`navigate_left` in `src/ui/browser/view_navigation.rs`). `KeySettings::left_leaves_without_cursor`, which the Columns view turns on | `tab-renames-past-open-folder/columns-unmirrored`, at line 6 |

C2e is a seventh key where the Columns component differed from the GTK view. Decision 17
ruled on six, each to match GTK whether or not a scenario checks it; this one was changed
under the same ruling, read in the GTK code first, and is for the owner to confirm. Read
and not run, so not changed: Shift+Left in a column, which GTK also takes to the parent
and the view extends in place.

### `field`

`71f3a9c97`. `Backend::field` reads a text field from the accessibility tree
(`crates/strata-app/src/qa/field.rs`):

| | From |
| --- | --- |
| `name`, `text` | The node's name and value |
| `caret`, `selection` | The node's `selection`: for each end, a line and a count of the clusters Rune shaped on it. The caret is the end that moves; `selection` is `null` when the ends meet |
| `focused` | The node is the snapshot's focus |
| `showing` | The node has a box |

**The unit.** The protocol counts characters, as AT-SPI does. Rune counts clusters, each a
user-perceived character, and the tree carries no cluster's length. The backend turns the
count into characters over the field's text by Unicode's grapheme clusters. For a name of
precomposed characters the two are the same number; `é` as a letter and a combining accent
is one cluster and two characters. `qa::field::tests` holds the conversion, and
`qa::tests::rename::the_caret_and_the_selection_count_characters_in_a_name_that_is_not_ascii`
reads a live field over `zō日本.txt`.

**Limits.** A field is read while both ends are on its first line, which is all of a
single-line field. On a later line the tree does not say where the line starts in the
text, and `field` answers nothing: the engine then finds no field. The slice has no
multi-line field.

The engine did not change, and neither did `crates/strata-qa` or `tests/qa.rs`. `scroll`,
`scroll_position` and `orientation` are still unanswered. `editor_within_viewport` and
`editable_fields` are the engine's and needed nothing: `long-rename-keeps-caret` passes at
420 points wide. The role `text` for Rune's `text-input` was mapped by name before and has
now been seen live.

### The layout step

`e0ea9caaa`. `tone-mark-escape-before-rename` types under `cn-altgr-pinyin`, where AltGr+1
and AltGr+3 are dead keys. V7's mechanism typed what a key types and had nothing for a key
that types nothing by itself.

A Rune window takes its keys from winit, which runs every press through xkbcommon's compose
state. The backend now does the same (`qa/compose.rs`): the dead key reaches no handler, the
letter that completes the sequence arrives as the composed text, and a named key that breaks
a sequence off is still delivered. The Compose table is the one of the process's locale, as
winit reads it; a process with no locale set, which is how the QA runner starts one, reads
the UTF-8 table every UTF-8 locale shares.
`qa::keyboard::tests::a_dead_key_types_nothing_until_a_letter_completes_it_and_escape_after_one_is_still_escape`
holds it. In the container the step passes: AltGr+1 then `a` types ā.

**The run is `rune` pending all the same.** Its next step presses Escape with a dead key
pending and expects only the tone mark to go. GTK's input method takes that Escape. winit
delivers it as a key press, Rune's window host passes it to the focused field, and the
editor closes: by the source, a Rune window does what the backend does, and the run stops
at line 18. A host cannot tell that Escape from any other.
[The request](../upstream/rune/98-escape-after-a-dead-key.md) is drafted and not filed. It
has no headless repro and cannot have one, so it wants a check in a window first.

### The surveys

`./scripts/qa-parity.sh g3 --pending`, in the pinned image, from the worktree at
`~/dev/strata-next-verify`. Logs and kept artifacts are in its `target/v9-work/`
(`survey-N.log`, `survey-N-rune-artifacts/`). The runner ignores two of the 33 runs on
either target.

| Survey | Pass on Rune | What changed |
| --- | --- | --- |
| 1, the wiring and `field` as first written | 12 of the 31 that ran | Everything: before it no run could get past its first `field` wait |
| 2, the tree of `e0ea9caaa` before the code-health refactors | 13 | C2e (`tab-renames-past-open-folder/columns-unmirrored`); dead keys moved `tone-mark-escape-before-rename` from line 12 to line 18 |

### The sort

**Required (13).**

| Scenario | Runs |
| --- | --- |
| `escape-preserves-name` | `columns` |
| `escape-preserves-name-remaining` | `columns-existing-folder` |
| `invalid-names-retain-original` | `columns-enter`, `columns-click` |
| `long-rename-keeps-caret` | `columns` |
| `rename-commits-on-click-away` | `columns` |
| `rename-commits-on-enter` | `columns` |
| `rename-moves-into-path` | `columns-file-as-folder` |
| `tab-renames-past-open-folder` | `columns-mirrored`, `columns-unmirrored` |
| `window-accelerators-leave-rename` | `columns-f5`, `columns-ctrl-backslash`, `columns-ctrl-k` |

Two of these pass on Rune for a weaker reason than on GTK:
`window-accelerators-leave-rename/columns-ctrl-backslash` and `columns-ctrl-k` check that a
window shortcut leaves an open editor alone, and `strata-app` binds neither Ctrl+\ (the
arrow scope) nor Ctrl+K (search). The guard itself is real, and the in-process test holds
it with four keys the window does bind.

**Pending (20).** Line numbers are the scenario file's, where the run stops on Rune.

| Run | Code | Stops at | Blocker |
| --- | --- | --- | --- |
| `rename-moves-into-path`: `columns-file`, `columns-open-folder`, `columns-existing-folder`, `columns-parent` | `scope` | Line 26 | Undo: `strata-app` wires no Ctrl+Z. Before it the typed path moves the item, the fs checks hold, and what took its place is selected and focused |
| `rename-moves-into-path/columns-back-into-folder` | `scope` | Line 19 | Undo. Before it `../documents/renamed.txt` renames the item in its own folder |
| `rename-and-undo-restores`: `columns-file`, `columns-folder` | `scope` | Lines 28, 29 | Undo. Before it the rename lands and the entry is selected and focused |
| `escape-preserves-name-remaining`: `columns-new-file`, `columns-new-folder` | `scope` | Lines 16, 22 | A new item: the menu's New File row, Ctrl+Shift+N |
| `new-item-first-free-number/columns` | `scope` | Line 15 | A new item |
| `new-item-named-with-path`: five runs | `scope` | Line 10 | A new item; `columns-refused` then needs the "Unable to rename item" dialog (C15) |
| `new-item-clears-hiding-filter`: `columns-file`, `columns-folder` | `scope` | Line 4 | The filter bar (C18), then a new item |
| `input-method-composes-over-selection`: two runs | `platform` | Not started | fcitx5-chewing on a session bus, which the image lacks: the scenario's own `skip` keeps the runner from starting it on either target |
| `tone-mark-escape-before-rename/columns` | `rune` | Line 18 | [The Escape after a dead key](#the-layout-step) |

No run is `toolkit` pending: nothing was reworded and nothing is proposed. No run is
`harness` pending.

Undo is seven runs behind one key: `Browser` has the undo engine, and each of the seven
passes every step before Ctrl+Z. This plan and the slice's scope leave undo out, so it was
not wired. It is the cheapest way to more required runs in this gate, and it is an owner
item in the [README](README.md#owner-decisions).

### The mutation patch

`61a7d6224`, `tests/qa/parity/mutations/rename-commits-on-enter.patch`: in
`rename.rs`, a name the editor closed on is never taken to `Browser::rename`. Enter still
closes the editor. It matches the GTK set's `rename-enter.patch`, under which Enter
discards the name.

## Done when, and the evidence

All on 2026-11-11. The pinned-image runs are from the worktree at
`~/dev/strata-next-verify`, at `61a7d6224`, in the unchanged base, with Docker. Their logs
and kept artifacts are in its `target/v9-61a7d6224/`; `run-gate.sh` there is the sequence
that produced them.

| Done when | Met |
| --- | --- |
| G3's runs are all in the ledger, and the `required` ones passed in three consecutive runs on both targets | Yes. All 33 are listed, each `required` or `pending` with a code and a blocker; the runner's ledger tests pass in every run below. `./scripts/qa-parity.sh g3` three times running: 13 of 13 on GTK and 13 of 13 on Rune each time (`qa-parity-g3-{1,2,3}.log`; bundles in `g3-{1,2,3}-gtk-artifacts/` and `g3-{1,2,3}-rune-artifacts/`, under `<scenario>/<run>/`) |
| Every `fs` check agrees across targets | Yes. The 13 required runs make 47 `fs` checks between them, which the runner performs on the fixture for either target, and a run passes only when each holds: 47 of 47 on GTK and on Rune, in each of the three runs. The two targets' transcripts agree in every verdict of every step (`ok`, `error`, `failures`, `criticals`): 39 of 39 pairs, 648 steps a target (`compare-verdicts.py`, `compare-verdicts.txt`). In the pending runs that get as far, the checks before the stop hold on Rune too: the five moves of `rename-moves-into-path` pass their placement checks, and the two of `rename-and-undo-restores` their rename check |
| The rename mutation patch fails its scenario on the Rune target | Yes. `./scripts/qa-parity.sh mutations`: 3 of 3 caught (`qa-parity-mutations.log`, `qa-parity-mutation/summary.md`). `rename-commits-on-enter` passes unpatched and fails patched at line 13, the `fs` check for the new name on disk |

| Test of the plan | Result |
| --- | --- |
| The shared rules | [Above](#the-shared-rules): the same counts before and after, plus the new tests. Run again on the final code: `./scripts/test-headless.py rename` gives 25, 1 with 51 ignored, and 88; `./scripts/test-headless.py name_field` runs the three new tests in `strata-fs` |
| The GTK app | `./scripts/qa.sh` with the 15 `inline_renaming` scenarios the runner starts: 42 runs pass, in all three view modes (`qa-inline-renaming.log`, `qa-inline-renaming-artifacts/`). `./scripts/qa-mutation.sh rename-commits-on-enter escape-preserves-name long-rename-keeps-caret rename-commits-on-click-away`: 4 of 4 caught (`qa-mutation.log`, `qa-mutation/summary.md`). All 21 patches that applied before the refactor still apply (`git apply --check`); `cut.patch` is stale as before |
| The wiring, in process | `mise run test-rune rename::`: the 13 tests of `strata-app` in the tables above, and the component's own |
| G3 on both targets, three times | Above |
| The filesystem | Above |
| Mutation | Above |
| No GTK | `cargo tree -p strata-rune -i gtk4`, with and without `--features qa`: "package ID specification `gtk4` did not match any packages" |

**The gates before this one,** on the final code, since the adapter and the backend they
rest on changed: `./scripts/qa-parity.sh g1`, 1 of 1 on each target (`qa-parity-g1.log`);
`./scripts/qa-parity.sh g2`, 22 of 22 on each (`qa-parity-g2.log`, `g2-gtk-artifacts/`,
`g2-rune-artifacts/`).

**The Rust suite and lint,** in the pinned image: `./scripts/quality.sh fmt` and `clippy`
are clean (`quality-fmt.log`, `quality-clippy.log`), and `./scripts/quality.sh test`
verifies 3,956 tests passed exactly once with 771 ignored (`quality-test.log`): V7's 3,953
and the three new tests of the shared rules. V8 had skipped that phase.

**The Rune side,** on the host, at `61a7d6224`:

- `mise run test-rune`: 73 test binaries, 2,385 tests pass, none fail, 253 ignored,
  counted as distinct test names. Nineteen are new: 16 in `strata-app` (eight of the
  adapter's, four of the engine's, the selection rules, two of `field`, one of dead keys),
  two in `strata-components` and one page test. V8 recorded 2,365, so its count and this
  one differ by one in method. The run includes every golden.
- `mise run test-platform window::`: three pass. The windowed host runs the same `poll`
  and `on_key`, which changed.
- Clippy on the Rune lane with warnings denied, with every feature and with none, and
  `cargo fmt --all --check`: clean.
- Code health, through CodeScene's local tool since `CS_ACCESS_TOKEN` is not set: every new
  file scores 10 (`rename.rs`, `columns/editing.rs`, `qa/field.rs`, `qa/compose.rs`,
  `services/name_field.rs` and the new test files). No touched file scores lower than at
  `92204a5f3`: `columns/mirror.rs` 9.09 and now 9.68, `columns/intents.rs` 10 and 10,
  `columns/mod.rs` and `columns/selection.rs` 9.68 and 9.68,
  `collection/keyboard/machine.rs` 8.69 and 8.69, `collection/keyboard/tests.rs` 7.55 and
  7.55, `rename/editor.rs` 9.59 and 9.59, `rename/tests.rs` 8.81 and 8.81,
  `columns/view.rs` 7.99 and 7.99, `src/ui/collection_edit.rs` 9.19 and 9.19.
- `python3 -m unittest discover -s scripts -p 'test_*.py'`: 243 tests pass. `typos` and
  `git diff --check` are clean. `Cargo.lock` changed by one line: `strata-app` now names
  `unicode-segmentation`, which the lock already held.

Not run, and why:

- The full `./scripts/qa.sh` and the other 17 GTK mutation patches: `crates/strata-qa`,
  `tests/qa.rs` and the engine did not change. The GTK lane changed by the two moved
  functions, which the scenarios above and `quality.sh test` cover.
- `./scripts/test-headless.py i18n::`: no string, key or catalog changed.
- `./scripts/qa-parity.sh slice`: it runs what ran here one by one, and its whole-suite
  part inherits the flaky `new-folder-with-selection-groups/columns`, which was not run
  here.
- `input-method-composes-over-selection`: named to `qa.sh` once by mistake, where it could
  not run for want of fcitx5-chewing, as its `skip` says
  (`qa-inline-renaming-with-the-skipped-scenario.log`). The run above leaves it to its
  `skip`.
- A host QA run: no consent, and none needed.
- `mise run test-platform` beyond `window::`: `strata-platform` did not change.
- `mise run bless`: no golden changed. The full Rune-side run above includes every golden.
- CI: nothing was pushed by this package.

## Open

- **Owner items 23 to 26** in the [README](README.md#owner-decisions): undo, new items,
  the failed rename's editor against GTK's dialog, and request 98's check in a window.
  And C2e, the seventh key, to confirm under decision 17.
- **20 pending runs:** 17 wait on undo, new items and the filter; two on an input method
  the image lacks; one on Rune.
- **Not built:** `scroll`, `scroll_position`, `orientation` and reveal-before-aim in the
  backend; a field's later lines; the label showing a new name before the rename lands;
  the field's own menu; a gallery scene of the editor in Columns.
- **Not seen:** the monitor reporting a rename before the operation does. The code keeps
  the cursor and the selection then, and loses a range's anchor.
- **Shift+Left in a column** is read as differing from GTK and was not run or changed.

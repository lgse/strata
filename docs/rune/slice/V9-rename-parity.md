# V9 Rename parity

| | |
| --- | --- |
| Status | Not started |
| Depends on | V8 |
| Unblocks | V10 |
| Owns | The rename wiring in `crates/strata-app`, the `field` half of the Rune QA backend, G3's ledger entries, one mutation patch |
| Lands as | Feature commits on `strata-next` |

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

- Rename from a context menu, and the rename dialog for conflicts beyond the inline
  error.
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
